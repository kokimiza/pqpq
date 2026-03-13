use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum DomainError {
    #[error("Invalid token format: {0}")]
    InvalidToken(String),

    #[error("Invalid public key: {0}")]
    InvalidPublicKey(String),

    #[error("Invalid signature: {0}")]
    InvalidSignature(String),

    #[error("Invalid SDP format: {0}")]
    InvalidSdp(String),

    #[error("Ring not found: {0}")]
    RingNotFound(String),

    #[error("Ring already occupied")]
    RingOccupied,

    #[error("Invalid ring state: {0}")]
    InvalidRingState(String),
}
