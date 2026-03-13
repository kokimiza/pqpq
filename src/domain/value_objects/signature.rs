use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct Signature(String);

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum SignatureError {
    #[error("Invalid signature format")]
    InvalidFormat,
}

#[allow(dead_code)]
impl Signature {
    pub fn new(value: String) -> Result<Self, SignatureError> {
        if value.is_empty() {
            return Err(SignatureError::InvalidFormat);
        }
        Ok(Self(value))
    }

    pub fn value(&self) -> &str {
        &self.0
    }
}
