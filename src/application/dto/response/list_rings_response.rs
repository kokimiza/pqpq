use uuid::Uuid;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RingInfo {
    pub ring_id: Uuid,
    pub token: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct ListRingsResponse {
    pub rings: Vec<RingInfo>,
}
