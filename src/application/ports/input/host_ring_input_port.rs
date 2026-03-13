use crate::application::dto::request::{HostRingRequest, WaitForMatchRequest};
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait HostRingInputPort: Send + Sync {
    /// リングを作成してホストとして待機（結果はOutput Portへ通知）
    async fn create_ring(&self, request: HostRingRequest) -> Result<()>;

    /// マッチング成立を待機（結果はOutput Portへ通知）
    async fn wait_for_match(&self, request: WaitForMatchRequest) -> Result<()>;
}
