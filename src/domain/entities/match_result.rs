use crate::domain::value_objects::PublicKey;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// 対戦終了の種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FinishType {
    /// KO（HP 0）
    Ko,
    /// タイムアウト（HP判定）
    Timeout,
    /// 切断（相手が切断）
    Disconnect,
}

impl FinishType {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            FinishType::Ko => "ko",
            FinishType::Timeout => "timeout",
            FinishType::Disconnect => "disconnect",
        }
    }

    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "ko" => Some(FinishType::Ko),
            "timeout" => Some(FinishType::Timeout),
            "disconnect" => Some(FinishType::Disconnect),
            _ => None,
        }
    }
}

/// 署名付き対戦結果エンティティ
///
/// 敗者がEd25519署名を付与することで戦績の偽装を防止する。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MatchResult {
    id: Uuid,
    winner_pubkey: PublicKey,
    loser_pubkey: PublicKey,
    winner_hp: u8,
    loser_hp: u8,
    duration_frames: u32,
    finish_type: FinishType,
    raw_payload: String,
    loser_signature: String,
    ring_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

#[allow(dead_code)]
impl MatchResult {
    pub fn new(
        winner_pubkey: PublicKey,
        loser_pubkey: PublicKey,
        winner_hp: u8,
        loser_hp: u8,
        duration_frames: u32,
        finish_type: FinishType,
        raw_payload: String,
        loser_signature: String,
        ring_id: Option<Uuid>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            winner_pubkey,
            loser_pubkey,
            winner_hp,
            loser_hp,
            duration_frames,
            finish_type,
            raw_payload,
            loser_signature,
            ring_id,
            created_at: Utc::now(),
        }
    }

    pub fn id(&self) -> &Uuid {
        &self.id
    }

    pub fn winner_pubkey(&self) -> &PublicKey {
        &self.winner_pubkey
    }

    pub fn loser_pubkey(&self) -> &PublicKey {
        &self.loser_pubkey
    }

    pub fn winner_hp(&self) -> u8 {
        self.winner_hp
    }

    pub fn loser_hp(&self) -> u8 {
        self.loser_hp
    }

    pub fn duration_frames(&self) -> u32 {
        self.duration_frames
    }

    pub fn finish_type(&self) -> FinishType {
        self.finish_type
    }

    pub fn raw_payload(&self) -> &str {
        &self.raw_payload
    }

    pub fn loser_signature(&self) -> &str {
        &self.loser_signature
    }

    pub fn ring_id(&self) -> Option<&Uuid> {
        self.ring_id.as_ref()
    }

    pub fn created_at(&self) -> &DateTime<Utc> {
        &self.created_at
    }
}
