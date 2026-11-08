//! Existing Linux envelope primitive, also used with a distinct macOS format.
use chacha20poly1305::{
    Key, KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
pub(super) const NONCE: usize = 24;
pub(super) const TAG: usize = 16;

pub(super) fn seal(
    magic: &[u8; 4],
    aad: &[u8],
    key: &[u8; 32],
    nonce: [u8; NONCE],
    bytes: &[u8],
) -> Option<Vec<u8>> {
    let sealed = XChaCha20Poly1305::new(&Key::from(*key))
        .encrypt(&XNonce::from(nonce), Payload { msg: bytes, aad })
        .ok()?;
    let mut blob = Vec::with_capacity(4 + NONCE + sealed.len());
    blob.extend_from_slice(magic);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&sealed);
    Some(blob)
}
pub(super) fn open(magic: &[u8; 4], aad: &[u8], key: &[u8; 32], blob: &[u8]) -> Option<Vec<u8>> {
    let rest = blob.strip_prefix(magic.as_slice())?;
    if rest.len() < NONCE + TAG {
        return None;
    }
    let (nonce, ciphertext) = rest.split_at(NONCE);
    let nonce: [u8; NONCE] = nonce.try_into().ok()?;
    XChaCha20Poly1305::new(&Key::from(*key))
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .ok()
}
