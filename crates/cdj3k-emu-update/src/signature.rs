//! The index's Ed25519 signature, checked against the update key's public
//! half. The secret half signs in `cd.yml`'s release job; the same key signs
//! the macOS `.dmg` for Sparkle.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_compact::{PublicKey, Signature};

use crate::Error;

const UPDATE_KEY: &str = include_str!("../../../packaging/update-ed25519.pub");

/// Whether `signature` (a `.sig` file's text: the base64 signature) signs
/// `data` with the update key.
pub fn verify(data: &[u8], signature: &str) -> Result<(), Error> {
    verify_with(UPDATE_KEY, data, signature)
}

fn verify_with(public_key: &str, data: &[u8], signature: &str) -> Result<(), Error> {
    let key =
        public_key_from(public_key).ok_or_else(|| Error::new("the update key is unreadable"))?;
    let sig = decode(signature)
        .and_then(|bytes| Signature::from_slice(&bytes).ok())
        .ok_or_else(|| Error::new("the update index signature is malformed"))?;
    key.verify(data, &sig)
        .map_err(|_| Error::new("the update index is not signed by the update key"))
}

fn public_key_from(text: &str) -> Option<PublicKey> {
    decode(text).and_then(|bytes| PublicKey::from_slice(&bytes).ok())
}

fn decode(text: &str) -> Option<Vec<u8>> {
    STANDARD.decode(text.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = include_str!("../tests/fixtures/test.pub");
    const INDEX: &[u8] = include_bytes!("../tests/fixtures/index.json");
    const SIG: &str = include_str!("../tests/fixtures/index.json.sig");

    #[test]
    fn the_signed_index_verifies() {
        verify_with(KEY, INDEX, SIG).unwrap();
    }

    #[test]
    fn a_changed_byte_does_not() {
        let mut tampered = INDEX.to_vec();
        let at = tampered.iter().position(|&b| b == b'4').unwrap();
        tampered[at] = b'5';
        assert!(verify_with(KEY, &tampered, SIG).is_err());
    }

    #[test]
    fn nor_does_another_key() {
        assert!(
            verify(INDEX, SIG).is_err(),
            "the fixture is not signed by the update key"
        );
    }

    #[test]
    fn the_update_key_decodes() {
        assert!(public_key_from(UPDATE_KEY).is_some());
    }
}
