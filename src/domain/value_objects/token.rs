use crate::domain::errors::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token(String);

impl Token {
    pub fn new(value: String) -> Result<Self, DomainError> {
        if value.is_empty() {
            return Err(DomainError::InvalidToken(
                "Token cannot be empty".to_string(),
            ));
        }
        Ok(Self(value))
    }

    pub fn generate() -> Self {
        use rand::distr::{Alphanumeric, SampleString};
        let mut rng = rand::rng();

        let token = Alphanumeric.sample_string(&mut rng, 8);
        Self(token.to_uppercase())
    }

    pub fn value(&self) -> &str {
        &self.0
    }
}
