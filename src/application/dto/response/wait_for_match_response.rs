use uuid::Uuid;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WaitForMatchResponse {
    pub ring_id: Uuid,
    pub matched: bool,
    pub opponent_sdp: Option<String>,
}
