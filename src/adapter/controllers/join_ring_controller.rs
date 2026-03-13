use crate::application::dto::request::{JoinRingRequest, WaitForMatchRequest};
use crate::application::ports::input::JoinRingInputPort;
use anyhow::Result;
use std::sync::Arc;

pub struct JoinRingController<I: JoinRingInputPort> {
    input_port: Arc<I>,
}

impl<I: JoinRingInputPort> JoinRingController<I> {
    pub fn new(input_port: Arc<I>) -> Self {
        Self { input_port }
    }

    /// リングに参加（結果はOutput Portへ通知）
    pub async fn join_ring(&self, token: String) -> Result<()> {
        let request = JoinRingRequest { token };
        self.input_port.join_ring(request).await
    }

    /// マッチングを待機（結果はOutput Portへ通知）
    pub async fn wait_for_match(&self, ring_id: uuid::Uuid) -> Result<()> {
        let request = WaitForMatchRequest { ring_id };
        self.input_port.wait_for_match(request).await
    }
}
