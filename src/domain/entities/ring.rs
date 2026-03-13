use crate::domain::value_objects::{PublicKey, Sdp, Token};
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RingStatus {
    Open,
    Matched,
    Expired,
}

#[derive(Debug, Clone)]
pub struct Ring {
    id: Uuid,
    token: Token,
    host_sdp: Sdp,
    guest_sdp: Option<Sdp>,
    host_pubkey: PublicKey,
    guest_pubkey: Option<PublicKey>,
    status: RingStatus,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

impl Ring {
    pub fn new(token: Token, host_sdp: Sdp, host_pubkey: PublicKey) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            token,
            host_sdp,
            guest_sdp: None,
            host_pubkey,
            guest_pubkey: None,
            status: RingStatus::Open,
            created_at: now,
            expires_at: now + Duration::minutes(15),
        }
    }

    /// リポジトリからの復元用
    pub fn reconstruct(
        id: Uuid,
        token: Token,
        host_sdp: Sdp,
        guest_sdp: Option<Sdp>,
        host_pubkey: PublicKey,
        guest_pubkey: Option<PublicKey>,
        status: RingStatus,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            token,
            host_sdp,
            guest_sdp,
            host_pubkey,
            guest_pubkey,
            status,
            created_at,
            expires_at,
        }
    }

    pub fn with_guest_answer(&mut self, guest_sdp: Sdp) {
        self.guest_sdp = Some(guest_sdp);
        self.status = RingStatus::Matched;
    }

    #[allow(dead_code)]
    pub fn with_guest_pubkey(&mut self, guest_pubkey: PublicKey) {
        self.guest_pubkey = Some(guest_pubkey);
    }

    /// 期限切れかどうか
    #[allow(dead_code)]
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    pub fn id(&self) -> &Uuid {
        &self.id
    }

    pub fn token(&self) -> &Token {
        &self.token
    }

    pub fn host_sdp(&self) -> &Sdp {
        &self.host_sdp
    }

    pub fn guest_sdp(&self) -> Option<&Sdp> {
        self.guest_sdp.as_ref()
    }

    pub fn host_pubkey(&self) -> &PublicKey {
        &self.host_pubkey
    }

    #[allow(dead_code)]
    pub fn guest_pubkey(&self) -> Option<&PublicKey> {
        self.guest_pubkey.as_ref()
    }

    pub fn status(&self) -> &RingStatus {
        &self.status
    }

    pub fn created_at(&self) -> &DateTime<Utc> {
        &self.created_at
    }

    #[allow(dead_code)]
    pub fn expires_at(&self) -> &DateTime<Utc> {
        &self.expires_at
    }
}
