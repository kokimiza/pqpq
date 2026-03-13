use uuid::Uuid;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct JoinRingResponse {
    pub ring_id: Uuid,
    pub host_sdp: String,
}
