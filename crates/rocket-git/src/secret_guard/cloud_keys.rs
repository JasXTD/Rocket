//! Offline recognition of AWS access-key IDs and Google OAuth client secrets.
//! No candidate is sent to a provider. See SECURITY.md for formats and limits.
use super::{ascii_entropy, find_candidate};
use crate::Result;

fn key_byte(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

/// AKIA (long-term), ASIA (temporary STS), ABIA (STS service bearer token) and
/// ACCA (context-specific credential) IDs: the prefix and 16 uppercase letters
/// or digits. An ID cannot sign requests alone, but it is how a pasted key pair,
/// credentials file or STS response shows up. AWS documentation examples end in
/// EXAMPLE (AKIAIOSFODNN7EXAMPLE); a generated ID realistically never does.
fn aws_key_id(bytes: &[u8]) -> bool {
    let Some((prefix, rest)) = bytes.split_at_checked(4) else {
        return false;
    };
    let Some(payload) = rest.get(..16) else {
        return false;
    };
    matches!(prefix, b"AKIA" | b"ASIA" | b"ABIA" | b"ACCA")
        && payload
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && !rest.get(16).is_some_and(u8::is_ascii_alphanumeric)
        && !payload.ends_with(b"EXAMPLE")
        // Filler such as AKIAXXXXXXXXXXXXXXXX is a placeholder. Sixteen random
        // characters score about 3.6 bits (4 at most): 2.5 rejects XXXX or ABAB
        // repeats yet misses about 3 in a million generated IDs (3.0: 1 in 275).
        && ascii_entropy(payload.iter().copied()) >= 2.5
}

pub(super) fn contains_aws_key_id(
    bytes: &[u8],
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<bool> {
    Ok(find_candidate(bytes, b'A', check, |at| {
        // A slice of a longer identifier is not an ID; `_` and `-` may precede one.
        let inside = at > 0 && bytes[at - 1].is_ascii_alphanumeric();
        (!inside && aws_key_id(&bytes[at..])).then_some(())
    })?
    .is_some())
}

/// Google OAuth client secrets (since 2021): GOCSPX- and 28 URL-safe characters.
pub(super) fn contains_google_client_secret(
    bytes: &[u8],
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<bool> {
    Ok(find_candidate(bytes, b'G', check, |at| {
        if at > 0 && key_byte(&bytes[at - 1]) {
            return None;
        }
        let value = bytes[at..].strip_prefix(b"GOCSPX-")?;
        let secret = value.get(..28)?;
        (secret.iter().all(key_byte)
            && !value.get(28).is_some_and(key_byte)
            && ascii_entropy(secret.iter().copied()) >= 3.0)
            .then_some(())
    })?
    .is_some())
}
