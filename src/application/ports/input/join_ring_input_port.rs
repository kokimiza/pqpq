use crate::application::dto::request::{JoinRingRequest, WaitForMatchRequest};
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait JoinRingInputPort: Send + Sync {
    /// リングに参加（結果はOutput Portへ通知）
    async fn join_ring(&self, request: JoinRingRequest) -> Result<()>;

    /// マッチング成立を待機（結果はOutput Portへ通知）
    async fn wait_for_match(&self, request: WaitForMatchRequest) -> Result<()>;
}
