use uuid::Uuid;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HostRingResponse {
    pub ring_id: Uuid,
    pub token: String,
    pub host_pubkey: String,
}
