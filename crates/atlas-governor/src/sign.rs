//! Ed25519 signing utilities for commits and policy objects (T4.9).

use crate::{GovernorError, Result};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Sign `data` with `key`; returns a lowercase hex-encoded signature.
pub fn sign_bytes(key: &SigningKey, data: &[u8]) -> String {
    hex::encode(key.sign(data).to_bytes())
}

/// Verify a hex-encoded Ed25519 signature against `data`.
pub fn verify_bytes(pubkey: &VerifyingKey, data: &[u8], sig_hex: &str) -> Result<()> {
    let sig_bytes = hex::decode(sig_hex).map_err(|e| GovernorError::Sign(e.to_string()))?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| GovernorError::Sign("invalid signature length".into()))?;
    let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
    pubkey
        .verify(data, &sig)
        .map_err(|e| GovernorError::Sign(e.to_string()))
}

/// A self-contained signed payload: payload + signature + public key.
/// Useful for signing commit hashes, policy documents, or audit roots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedEnvelope {
    /// Hex-encoded payload bytes.
    pub payload: String,
    /// Hex-encoded Ed25519 signature over the payload.
    pub signature: String,
    /// Hex-encoded verifying (public) key.
    pub pubkey: String,
}

impl SignedEnvelope {
    /// Create and sign a new envelope.
    pub fn sign(key: &SigningKey, payload: &[u8]) -> Self {
        Self {
            payload: hex::encode(payload),
            signature: sign_bytes(key, payload),
            pubkey: hex::encode(key.verifying_key().to_bytes()),
        }
    }

    /// Verify the envelope's signature and return the decoded payload.
    pub fn verify(&self) -> Result<Vec<u8>> {
        let payload = hex::decode(&self.payload).map_err(|e| GovernorError::Sign(e.to_string()))?;
        let pub_bytes =
            hex::decode(&self.pubkey).map_err(|e| GovernorError::Sign(e.to_string()))?;
        let pub_arr: [u8; 32] = pub_bytes
            .try_into()
            .map_err(|_| GovernorError::Sign("invalid pubkey length".into()))?;
        let pubkey =
            VerifyingKey::from_bytes(&pub_arr).map_err(|e| GovernorError::Sign(e.to_string()))?;
        verify_bytes(&pubkey, &payload, &self.signature)?;
        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    fn fresh_key() -> SigningKey {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    #[test]
    fn sign_verify_roundtrip() {
        let key = fresh_key();
        let data = b"atlas commit abc123";
        let sig = sign_bytes(&key, data);
        assert!(verify_bytes(&key.verifying_key(), data, &sig).is_ok());
    }

    #[test]
    fn wrong_data_rejected() {
        let key = fresh_key();
        let sig = sign_bytes(&key, b"original");
        assert!(verify_bytes(&key.verifying_key(), b"tampered", &sig).is_err());
    }

    #[test]
    fn envelope_roundtrip() {
        let key = fresh_key();
        let env = SignedEnvelope::sign(&key, b"policy v1");
        let payload = env.verify().unwrap();
        assert_eq!(payload, b"policy v1");
    }

    #[test]
    fn tampered_envelope_rejected() {
        let key = fresh_key();
        let mut env = SignedEnvelope::sign(&key, b"data");
        env.payload = hex::encode(b"evil");
        assert!(env.verify().is_err());
    }
}
