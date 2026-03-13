use anyhow::Result;
use ed25519_dalek::SigningKey;

pub struct Ed25519Signer;

impl Ed25519Signer {
    pub fn generate_keypair() -> Result<(String, String)> {
        let signing_key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
        let verifying_key = signing_key.verifying_key();
        let private_key = hex::encode(signing_key.to_bytes());
        let public_key = hex::encode(verifying_key.to_bytes());

        Ok((private_key, public_key))
    }
}
