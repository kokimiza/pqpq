use crate::domain::errors::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sdp(String);

impl Sdp {
    pub fn new(value: String) -> Result<Self, DomainError> {
        if value.is_empty() {
            return Err(DomainError::InvalidSdp("SDP cannot be empty".to_string()));
        }
        // 基本的なSDP形式チェック
        if !value.contains("v=0") {
            return Err(DomainError::InvalidSdp(
                "SDP must contain version line".to_string(),
            ));
        }
        Ok(Self(value))
    }

    pub fn value(&self) -> &str {
        &self.0
    }
}
