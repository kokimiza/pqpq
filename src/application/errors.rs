use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum ApplicationError {
    #[error("Domain error: {0}")]
    Domain(#[from] crate::domain::errors::DomainError),

    #[error("Repository error: {0}")]
    Repository(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Ring creation failed: {0}")]
    RingCreationFailed(String),

    #[error("Ring not found: {0}")]
    RingNotFound(String),

    #[error("Ring join failed: {0}")]
    RingJoinFailed(String),

    #[error("Failed to list rings: {0}")]
    ListRingsFailed(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),
}
