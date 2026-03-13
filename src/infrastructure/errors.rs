use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum InfrastructureError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Database connection failed: {0}")]
    DatabaseConnection(String),

    #[error("Query execution failed: {0}")]
    QueryExecution(String),

    #[error("Cryptography error: {0}")]
    Crypto(String),

    #[error("WebRTC error: {0}")]
    WebRtc(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Environment variable not found: {0}")]
    EnvVar(#[from] std::env::VarError),
}
