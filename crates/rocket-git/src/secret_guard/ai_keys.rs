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
    for first in *b"sgxhA" {
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
                b'A' => prefixed(tail, b"AIza", 35, key_byte),
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
