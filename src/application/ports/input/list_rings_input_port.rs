use crate::application::dto::request::ListRingsRequest;
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait ListRingsInputPort: Send + Sync {
    async fn execute(&self, request: ListRingsRequest) -> Result<()>;
}
