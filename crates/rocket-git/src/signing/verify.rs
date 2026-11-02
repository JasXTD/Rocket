use super::*;
#[derive(Clone, Debug)]
pub enum SignatureTarget {
    Commit(ObjectId),
    Tag(TagEntry),
}
impl SignatureTarget {
    pub fn id(&self) -> &ObjectId {
        match self {
            Self::Commit(id) => id,
            Self::Tag(entry) => &entry.id,
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Commit(_) => "commit",
            Self::Tag(_) => "tag",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureStatus {
    Unsigned,
    Valid,
    Untrusted,
    Invalid,
    Expired,
    Revoked,
    Unavailable,
}
impl SignatureStatus {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unsigned => "unsigned",
            Self::Valid => "valid",
            Self::Untrusted => "untrusted",
            Self::Invalid => "invalid",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::Unavailable => "unavailable",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Unsigned => "Unsigned",
            Self::Valid => "Valid signature · trust policy accepted",
            Self::Untrusted => "Valid signature · trust unknown or rejected",
            Self::Invalid => "Invalid signature",
            Self::Expired => "Expired signature or key",
            Self::Revoked => "Revoked signing key",
            Self::Unavailable => "Unable to verify",
        }
    }
    fn failure(self) -> Option<SigningFailure> {
        match self {
            Self::Unsigned | Self::Valid => None,
            Self::Untrusted => Some(SigningFailure::UntrustedSigner),
            Self::Invalid => Some(SigningFailure::InvalidSignature),
            Self::Expired => Some(SigningFailure::Expired),
            Self::Revoked => Some(SigningFailure::Revoked),
            Self::Unavailable => Some(SigningFailure::CannotVerify),
        }
    }
    pub fn guidance(self) -> &'static str {
        self.failure().map_or(match self {Self::Unsigned=>"This object has no signature. Unsigned does not mean invalid.",_=>"The signature matches this object and Git accepted the configured trust policy. Check the signer and fingerprint before relying on its identity."},SigningFailure::guidance)
    }
}
#[derive(Clone, Debug)]
pub struct SignatureVerification {
    pub id: ObjectId,
    pub kind: &'static str,
    pub status: SignatureStatus,
    pub format: &'static str,
    pub signer: Option<String>,
    pub fingerprint: Option<String>,
    pub trust: String,
}
impl Repository {
    pub fn verify_signature(
        &self,
        target: &SignatureTarget,
        cancel: &CancellationToken,
    ) -> Result<SignatureVerification> {
        self.runner
            .step("signature-verify", "verify_object_signature", || {
                let before = self.signing(cancel)?;
                let id = target.id();
                let object_type = match target {
                    SignatureTarget::Commit(_) => "commit",
                    SignatureTarget::Tag(entry) => {
                        self.inspect_tag(entry, cancel)?;
                        if entry.annotated() {
                            "tag"
                        } else {
                            "lightweight"
                        }
                    }
                };
                let mut verification = SignatureVerification {
                    id: id.clone(),
                    kind: target.kind(),
                    status: SignatureStatus::Unsigned,
                    format: "none",
                    signer: None,
                    fingerprint: None,
                    trust: "Not applicable".into(),
                };
                if object_type != "lightweight" {
                    let raw = self
                        .runner
                        .run(
                            self.cwd(),
                            &args(&["--no-replace-objects", "cat-file", object_type, id.as_str()]),
                            vec![],
                            cancel,
                        )?
                        .success("signature-verify")?;
                    if let Some(format) = signature_format(&raw, object_type) {
                        verification.format = format;
                        let out = self.runner.step(
                            "signature-verify",
                            "run_signature_verifier",
                            || {
                                self.runner.run(
                                    self.cwd(),
                                    &args(&[
                                        "--no-replace-objects",
                                        if object_type == "tag" {
                                            "verify-tag"
                                        } else {
                                            "verify-commit"
                                        },
                                        "--raw",
                                        id.as_str(),
                                    ]),
                                    vec![],
                                    cancel,
                                )
                            },
                        )?;
                        parse_verification(&mut verification, out.status.success(), &out.stderr);
                    }
                }
                if self.signing(cancel)? != before {
                    return Err(Error::Signing(SigningFailure::StaleSettings));
                }
                // Fixed codes only. Signer identities, key paths and helper output never enter logs.
                let _ =
                    self.runner.step(
                        "signature-verify",
                        "signature_result",
                        || match verification.status.failure() {
                            Some(f) => Err(Error::Signing(f)),
                            None => Ok(()),
                        },
                    );
                Ok(verification)
            })
    }
}
fn signature_format(raw: &[u8], kind: &str) -> Option<&'static str> {
    let armor = |line: &[u8]| match line {
        b"-----BEGIN SSH SIGNATURE-----" => Some("ssh"),
        b"-----BEGIN PGP SIGNATURE-----" => Some("openpgp"),
        b"-----BEGIN SIGNED MESSAGE-----" => Some("unknown"),
        _ => None,
    };
    if kind == "commit" {
        // Never infer a commit's signer format from its message text.
        raw.split(|b| *b == b'\n')
            .take_while(|l| !l.is_empty())
            .find_map(|line| {
                line.strip_prefix(b"gpgsig ")
                    .or_else(|| line.strip_prefix(b"gpgsig-sha256 "))
            })
            .map(|line| armor(line).unwrap_or("unknown"))
    } else {
        raw.split(|b| *b == b'\n').find_map(armor)
    }
}
fn visible(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(1024)
        .collect()
}
fn parse_verification(v: &mut SignatureVerification, success: bool, raw: &[u8]) {
    let text = String::from_utf8_lossy(raw);
    let statuses: Vec<_> = text
        .lines()
        .filter_map(|line| line.strip_prefix("[GNUPG:] "))
        .collect();
    let has = |prefix: &str| {
        statuses
            .iter()
            .any(|s| s.split_whitespace().next() == Some(prefix))
    };
    v.status = SignatureStatus::Unavailable;
    v.trust = "Not established".into();
    if v.format == "ssh" {
        let good = text
            .lines()
            .find(|l| l.starts_with("Good \"git\" signature "));
        if let Some(line) = good {
            if let Some((_, key)) = line.rsplit_once(" key ") {
                v.fingerprint = Some(visible(key));
            }
            if let Some(principal) = line
                .strip_prefix("Good \"git\" signature for ")
                .and_then(|s| s.rsplit_once(" with ").map(|(p, _)| p))
            {
                v.signer = Some(visible(principal));
            }
            if success && v.signer.is_some() {
                v.status = SignatureStatus::Valid;
                v.trust = "Allowed signer matched".into();
            } else {
                v.status = SignatureStatus::Untrusted;
                v.trust = "No accepted principal".into();
            }
            if text.contains("Unable to open allowed keys file")
                || text.contains("invalid key")
                || text.contains("invalid options")
            {
                v.status = SignatureStatus::Unavailable;
                v.trust = "SSH trust file unreadable or invalid".into();
            }
        } else if text.contains("incorrect signature")
            || text.contains("Signature verification failed")
        {
            v.status = SignatureStatus::Invalid;
        }
        return;
    }
    if let Some(line) = statuses.iter().find_map(|s| s.strip_prefix("VALIDSIG ")) {
        v.fingerprint = line.split_whitespace().next().map(visible);
    }
    if let Some(line) = statuses.iter().find_map(|s| {
        s.strip_prefix("GOODSIG ")
            .or_else(|| s.strip_prefix("EXPKEYSIG "))
            .or_else(|| s.strip_prefix("REVKEYSIG "))
    }) {
        v.signer = line.split_once(' ').map(|(_, s)| visible(s));
    }
    if let Some(trust) = statuses.iter().find_map(|s| {
        s.strip_prefix("TRUST_")
            .and_then(|s| s.split_whitespace().next())
    }) {
        v.trust = visible(trust);
    }
    v.status = if has("BADSIG") {
        SignatureStatus::Invalid
    } else if has("REVKEYSIG") || has("KEYREVOKED") {
        SignatureStatus::Revoked
    } else if has("EXPSIG") || has("EXPKEYSIG") || has("SIGEXPIRED") || has("KEYEXPIRED") {
        SignatureStatus::Expired
    } else if has("VALIDSIG") && has("GOODSIG") {
        if success && (has("TRUST_FULLY") || has("TRUST_ULTIMATE")) {
            SignatureStatus::Valid
        } else {
            SignatureStatus::Untrusted
        }
    } else {
        SignatureStatus::Unavailable
    };
}
#[cfg(test)]
mod tests {
    use super::*;
    fn result(format: &'static str, ok: bool, text: &str) -> SignatureVerification {
        let mut v = SignatureVerification {
            id: ObjectId::parse("1111111111111111111111111111111111111111").unwrap(),
            kind: "commit",
            status: SignatureStatus::Unsigned,
            format,
            signer: None,
            fingerprint: None,
            trust: String::new(),
        };
        parse_verification(&mut v, ok, text.as_bytes());
        v
    }
    #[test]
    fn trust_is_not_inferred_from_exit_or_signature_alone() {
        assert_eq!(
            result(
                "openpgp",
                true,
                "[GNUPG:] GOODSIG 1 User\n[GNUPG:] VALIDSIG ABC 0\n[GNUPG:] TRUST_UNDEFINED 0"
            )
            .status,
            SignatureStatus::Untrusted
        );
        assert_eq!(
            result("openpgp", true, "arbitrary helper output").status,
            SignatureStatus::Unavailable
        );
        assert_eq!(
            result(
                "ssh",
                false,
                "Good \"git\" signature with ED25519 key SHA256:abc\nNo principal matched."
            )
            .status,
            SignatureStatus::Untrusted
        );
        assert_eq!(
            result(
                "ssh",
                true,
                "Good \"git\" signature for user with ED25519 key SHA256:abc"
            )
            .status,
            SignatureStatus::Valid
        );
    }
    #[test]
    fn rejected_revoked_and_expired_take_precedence() {
        assert_eq!(
            result(
                "openpgp",
                false,
                "[GNUPG:] VALIDSIG ABC 0\n[GNUPG:] REVKEYSIG A User\n[GNUPG:] TRUST_ULTIMATE 0"
            )
            .status,
            SignatureStatus::Revoked
        );
        assert_eq!(
            result("openpgp", false, "[GNUPG:] EXPKEYSIG A User").status,
            SignatureStatus::Expired
        );
        assert_eq!(
            result("openpgp", false, "[GNUPG:] BADSIG A User").status,
            SignatureStatus::Invalid
        );
    }
    #[test]
    fn unreadable_ssh_trust_file_is_not_a_completed_trust_check() {
        let v = result(
            "ssh",
            false,
            "Good \"git\" signature with ED25519 key SHA256:abc\nUnable to open allowed keys file \"PRIVATE_path\": No such file or directory\nNo principal matched.",
        );
        assert_eq!(v.status, SignatureStatus::Unavailable);
        assert_eq!(v.fingerprint.as_deref(), Some("SHA256:abc"));
        assert!(!v.trust.contains("PRIVATE_path"));
    }
    #[test]
    fn commit_message_armor_cannot_select_the_verifier_format() {
        assert_eq!(
            signature_format(b"tree 1\n\n-----BEGIN SSH SIGNATURE-----\n", "commit"),
            None
        );
        assert_eq!(signature_format(b"tree 1\ngpgsig -----BEGIN PGP SIGNATURE-----\n data\n\n-----BEGIN SSH SIGNATURE-----\n", "commit"),Some("openpgp"));
    }
}
