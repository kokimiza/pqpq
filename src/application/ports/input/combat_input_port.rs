use crate::application::dto::request::{CombatFrameRequest, StartCombatRequest};
use crate::application::dto::response::{CombatFrameResponse, StartCombatResponse};
use anyhow::Result;
use async_trait::async_trait;

/// 戦闘入力ポート
#[async_trait]
pub trait CombatInputPort: Send + Sync {
    /// 戦闘を開始
    async fn start_combat(&self, request: StartCombatRequest) -> Result<StartCombatResponse>;

    /// フレームを更新
    async fn update_frame(&self, request: CombatFrameRequest) -> Result<CombatFrameResponse>;
}
