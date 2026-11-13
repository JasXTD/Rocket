//! JSON Web Key private material: a `kty` member near a long private `d` (RSA,
//! EC, OKP) or secret `k` (oct) value. JWKs appear as JSON, escaped JSON,
//! JavaScript or Python objects and YAML, so this is a bounded lexical check,
//! not a JSON parser. Public JWKs (only n/e or x/y) never match.
use super::ascii_entropy;
use crate::Result;

/// Farthest a private member may sit from its `kty`, in either order. A 4096-bit
/// RSA JWK with CRT values is about 3.5 KiB; an `x5c` chain can add more.
const NEAR: usize = 16 * 1024;
/// A 256-bit EC or OKP `d` has 43 characters; RSA values are far longer.
const PRIVATE_MINIMUM: usize = 40;
/// A 128-bit symmetric `k` has 22 characters.
const SECRET_MINIMUM: usize = 22;

fn value_byte(byte: &u8) -> bool {
    // base64url, plus the standard alphabet some tools emit.
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'/')
}

fn whitespace(bytes: &[u8], mut at: usize, check: &mut dyn FnMut() -> Result<()>) -> Result<usize> {
    // JSON permits exactly these four whitespace bytes. Inspect without copying
    // or imposing a silent truncation that would let padded keys pass. The input
    // size/deadline are bounded by the caller, and long runs remain cancellable.
    while matches!(bytes.get(at), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        if at & (64 * 1024 - 1) == 0 {
            check()?;
        }
        at += 1;
    }
    Ok(at)
}

/// The member `name` at `at`, written as "name", 'name', \"name\" (escaped JSON)
/// or as a bare object/YAML key, then `:` or `=`. Returns where its value
/// starts, after any opening quote.
fn member(
    bytes: &[u8],
    at: usize,
    name: &[u8],
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Option<usize>> {
    let mut i = at + name.len();
    if bytes.get(at..i) != Some(name) {
        return Ok(None);
    }
    match at.checked_sub(1).map(|before| bytes[before]) {
        Some(quote @ (b'"' | b'\'')) => {
            if bytes.get(i) == Some(&b'\\') {
                i += 1;
            }
            if bytes.get(i) != Some(&quote) {
                return Ok(None);
            }
            i += 1;
        }
        None | Some(b'{' | b'(' | b'[' | b',' | b' ' | b'\t' | b'\n' | b'\r') => {}
        Some(_) => return Ok(None),
    }
    i = whitespace(bytes, i, check)?;
    if !matches!(bytes.get(i), Some(b':' | b'=')) {
        return Ok(None);
    }
    i = whitespace(bytes, i + 1, check)?;
    if bytes.get(i) == Some(&b'\\') {
        i += 1;
    }
    if matches!(bytes.get(i), Some(b'"' | b'\'' | b'`')) {
        i += 1;
    }
    Ok(Some(i))
}

/// `Some(true)` for the symmetric `oct` key type, `Some(false)` for RSA, EC or OKP.
fn key_type(
    bytes: &[u8],
    at: usize,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Option<bool>> {
    let Some(start) = member(bytes, at, b"kty", check)? else {
        return Ok(None);
    };
    let value = &bytes[start..];
    let (symmetric, length) = if value.starts_with(b"RSA") || value.starts_with(b"OKP") {
        (false, 3)
    } else if value.starts_with(b"EC") {
        (false, 2)
    } else if value.starts_with(b"oct") {
        (true, 3)
    } else {
        return Ok(None);
    };
    Ok((!value.get(length).is_some_and(u8::is_ascii_alphanumeric)).then_some(symmetric))
}

/// A varied value of at least `minimum` characters for the member `name`.
fn material(
    bytes: &[u8],
    at: usize,
    name: &[u8],
    minimum: usize,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<bool> {
    let Some(start) = member(bytes, at, name, check)? else {
        return Ok(false);
    };
    // Measure a bounded prefix only; key material needs no more.
    let value = &bytes[start..];
    let length = value
        .iter()
        .take(4096)
        .take_while(|b| value_byte(b))
        .count();
    Ok(length >= minimum && ascii_entropy(value[..length].iter().copied()) >= 3.0)
}

/// The 1-based line of a private JWK member near its `kty`, if any.
pub(super) fn find(bytes: &[u8], check: &mut dyn FnMut() -> Result<()>) -> Result<Option<usize>> {
    // Only the latest position of each member kind is kept, so memory stays
    // constant. A forward scan and bounded material lookahead keep work linear;
    // whitespace runs contain no candidates and remain cancellable during lookahead.
    let (mut asymmetric, mut symmetric) = (None, None);
    let (mut private, mut secret) = (None, None);
    let near = |earlier: Option<usize>, at: usize| earlier.filter(|&e| at - e <= NEAR);
    for (at, &byte) in bytes.iter().enumerate() {
        if at & (64 * 1024 - 1) == 0 {
            check()?;
        }
        let found = match byte {
            b'k' => match key_type(bytes, at, check)? {
                Some(true) => {
                    symmetric = Some(at);
                    near(secret, at)
                }
                Some(false) => {
                    asymmetric = Some(at);
                    near(private, at)
                }
                None if material(bytes, at, b"k", SECRET_MINIMUM, check)? => {
                    secret = Some(at);
                    near(symmetric, at).map(|_| at)
                }
                None => None,
            },
            b'd' if material(bytes, at, b"d", PRIVATE_MINIMUM, check)? => {
                private = Some(at);
                near(asymmetric, at).map(|_| at)
            }
            _ => None,
        };
        if let Some(position) = found {
            let line = bytes[..position].iter().filter(|b| **b == b'\n').count() + 1;
            return Ok(Some(line));
        }
    }
    Ok(None)
}
