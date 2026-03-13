use crate::application::dto::request::{HostRingRequest, WaitForMatchRequest};
use crate::application::ports::input::HostRingInputPort;
use anyhow::Result;
use std::sync::Arc;

pub struct HostRingController<I: HostRingInputPort> {
    input_port: Arc<I>,
}

impl<I: HostRingInputPort> HostRingController<I> {
    pub fn new(input_port: Arc<I>) -> Self {
        Self { input_port }
    }

    /// リングを作成（結果はOutput Portへ通知）
    pub async fn create_ring(&self) -> Result<()> {
        let request = HostRingRequest;
        self.input_port.create_ring(request).await
    }

    /// マッチングを待機（結果はOutput Portへ通知）
    pub async fn wait_for_match(&self, ring_id: uuid::Uuid) -> Result<()> {
        let request = WaitForMatchRequest { ring_id };
        self.input_port.wait_for_match(request).await
    }
}
