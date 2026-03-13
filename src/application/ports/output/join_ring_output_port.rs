use crate::application::dto::response::JoinRingResponse;
use crate::application::ports::output::ErrorLayer;
use anyhow::Result;
use async_trait::async_trait;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait JoinRingOutputPort: Send + Sync {
    async fn notify_progress(&self, message: &str) -> Result<()>;
    async fn notify_error(&self, layer: ErrorLayer, message: &str) -> Result<()>;
    async fn present(&self, response: JoinRingResponse) -> Result<()>;
}
