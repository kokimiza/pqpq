use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct WaitForMatchRequest {
    pub ring_id: Uuid,
}
