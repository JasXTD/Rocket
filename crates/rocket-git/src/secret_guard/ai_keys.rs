//! Offline recognition of complete, known provider-key formats. No credentials
//! are sent anywhere. See SECURITY.md for format references and coverage limits.
use super::{ascii_entropy, find_candidate};
use crate::Result;

fn key_byte(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn payload(bytes: &[u8], length: usize, alphabet: fn(&u8) -> bool) -> bool {
    let Some(value) = bytes.get(..length) else {
        return false;
    };
    value.iter().all(alphabet)
        && !bytes.get(length).is_some_and(key_byte)
        // Repeated filler is common in docs. There is no file/name allowlist;
        // realistic synthetic keys intentionally retain the same protection.
        && ascii_entropy(value.iter().copied()) >= 3.0
}

fn openai(bytes: &[u8]) -> bool {
    if let Some(value) = bytes.strip_prefix(b"sk-")
        && payload(value, 48, u8::is_ascii_alphanumeric)
        && value.get(20..28) == Some(b"T3BlbkFJ")
    {
        return true;
    }
    for prefix in [b"sk-proj-".as_slice(), b"sk-svcacct-", b"sk-admin-"] {
        if let Some(value) = bytes.strip_prefix(prefix) {
            for before in [58, 74] {
                for after in [58, 74] {
                    if payload(value, before + 8 + after, key_byte)
                        && value.get(before..before + 8) == Some(b"T3BlbkFJ")
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn prefixed(bytes: &[u8], prefix: &[u8], length: usize, alphabet: fn(&u8) -> bool) -> bool {
    bytes
        .strip_prefix(prefix)
        .is_some_and(|value| payload(value, length, alphabet))
}

fn base64_byte(byte: &u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/')
}

/// A complete standard-base64 value at the start of `bytes`: at most `limit`
/// characters, up to two `=`, a whole number of four-character groups and no
/// key character after it. A truncated key or a slice of a longer one fails.
fn complete_base64(bytes: &[u8], limit: usize) -> Option<&[u8]> {
    let run = bytes
        .iter()
        .take(limit)
        .take_while(|b| base64_byte(b))
        .count();
    let end = run
        + bytes[run..]
            .iter()
            .take(2)
            .take_while(|b| **b == b'=')
            .count();
    (run > 0 && end % 4 == 0 && !bytes.get(end).is_some_and(key_byte)).then(|| &bytes[..end])
}

/// Decodes standard base64 by complete four-character groups, as AWS writes
/// Bedrock keys. Malformed input only yields bytes that fail the checks below.
fn decode(text: &[u8]) -> Vec<u8> {
    let sextet = |byte: u8| match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => 0, // `=` padding
    };
    let mut decoded = Vec::with_capacity(text.len() / 4 * 3);
    for group in text.as_chunks::<4>().0 {
        let n = group
            .iter()
            .fold(0u32, |n, byte| n << 6 | u32::from(sextet(*byte)));
        let length = 3 - group.iter().filter(|b| **b == b'=').count().min(2);
        decoded.extend_from_slice(&n.to_be_bytes()[1..1 + length]);
    }
    decoded
}

/// Amazon Bedrock long-term keys: ABSK and base64 of
/// `<IAM user name>[+1]-at-<12-digit account ID>:<base64 secret>`. Console keys
/// belong to a BedrockAPIKey-xxxx user, but any IAM user can hold one, so the
/// account and secret structure is checked rather than the user name.
fn bedrock_long_term(bytes: &[u8]) -> bool {
    // User names have at most 64 characters, so real keys stay well below 512.
    let Some(text) = bytes
        .strip_prefix(b"ABSK")
        .and_then(|value| complete_base64(value, 512))
    else {
        return false;
    };
    let decoded = decode(text);
    // The user name may itself contain "-at-", so try each occurrence.
    (1..decoded.len()).any(|at| {
        let (user, rest) = decoded.split_at(at);
        let Some(rest) = rest.strip_prefix(b"-at-") else {
            return false;
        };
        let Some(secret) = rest.get(13..) else {
            return false;
        };
        user.len() <= 72
            && user
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || b"_+=,.@-".contains(b))
            && rest[..12].iter().all(u8::is_ascii_digit)
            && rest[12] == b':'
            // The secret is itself base64 of 44 random bytes; it ends the key.
            && complete_base64(secret, 128).is_some_and(|value| {
                value.len() == secret.len() && (32..=64).contains(&decode(value).len())
            })
    })
}

/// Amazon Bedrock short-term keys (valid for at most 12 hours):
/// bedrock-api-key- and base64 of a presigned bedrock:CallWithBearerToken URL.
fn bedrock_short_term(bytes: &[u8]) -> bool {
    // Real keys exceed 1,000 characters, mostly the session token; 16 KiB
    // leaves room for long tokens yet bounds the decoding work.
    let Some(text) = bytes
        .strip_prefix(b"bedrock-api-key-")
        .and_then(|value| complete_base64(value, 16 * 1024))
    else {
        return false;
    };
    let url = decode(text);
    let contains = |needle: &[u8]| url.windows(needle.len()).position(|w| w == needle);
    contains(b"Action=CallWithBearerToken").is_some()
        && contains(b"X-Amz-Signature=").is_some_and(|at| {
            let signature = &url[at + 16..];
            signature.len() >= 64
                && signature[..64].iter().all(u8::is_ascii_hexdigit)
                && !signature.get(64).is_some_and(u8::is_ascii_hexdigit)
        })
}

/// These formats lack a unique provider fingerprint, so only recognize them in
/// credential assignments. Hashes, dotted identifiers and URLs elsewhere remain
/// ordinary data. Names identify provider settings, never a project or fixture.
pub(super) fn context_name(key: &[u8]) -> bool {
    matches!(
        key,
        b"apikey"
            | b"apisecret"
            | b"token"
            | b"accesstoken"
            | b"authtoken"
            | b"secret"
            | b"password"
            | b"deepseekapikey"
            | b"deepseekkey"
            | b"zaiapikey"
            | b"zhipuapikey"
            | b"zhipuaiapikey"
            | b"glmapikey"
            | b"glmkey"
    )
}

pub(super) fn context_value(value: &[u8]) -> bool {
    if let Some(body) = value.strip_prefix(b"sk-")
        && body.len() == 32
        && payload(body, 32, u8::is_ascii_hexdigit)
    {
        return true;
    }
    let mut parts = value.split(|b| *b == b'.');
    let (Some(id), Some(secret), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    // Z.ai/GLM's legacy API keys are an id.secret pair, not a JWT or a hostname.
    (16..=64).contains(&id.len())
        && (16..=128).contains(&secret.len())
        && payload(id, id.len(), u8::is_ascii_hexdigit)
        && payload(secret, secret.len(), key_byte)
}

pub(super) fn contains_key(bytes: &[u8], check: &mut dyn FnMut() -> Result<()>) -> Result<bool> {
    for first in *b"sgxhAb" {
        if find_candidate(bytes, first, check, |at| {
            if at > 0 && key_byte(&bytes[at - 1]) {
                return None;
            }
            let tail = &bytes[at..];
            let matched = match first {
                b's' => {
                    openai(tail)
                        || [
                            b"sk-ant-api03-".as_slice(),
                            b"sk-ant-api01-",
                            b"sk-ant-admin01-",
                        ]
                        .iter()
                        .any(|prefix| {
                            tail.strip_prefix(*prefix).is_some_and(|value| {
                                payload(value, 95, key_byte) && value.get(93..95) == Some(b"AA")
                            })
                        })
                        || prefixed(tail, b"sk-or-v1-", 64, |b| {
                            b.is_ascii_digit() || (b'a'..=b'f').contains(b)
                        })
                }
                b'g' => prefixed(tail, b"gsk_", 52, u8::is_ascii_alphanumeric),
                b'x' => prefixed(tail, b"xai-", 80, u8::is_ascii_alphanumeric),
                b'h' => prefixed(tail, b"hf_", 34, u8::is_ascii_alphabetic),
                b'A' => prefixed(tail, b"AIza", 35, key_byte) || bedrock_long_term(tail),
                b'b' => bedrock_short_term(tail),
                _ => false,
            };
            matched.then_some(())
        })?
        .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}
