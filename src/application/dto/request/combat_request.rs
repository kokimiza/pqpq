use crate::domain::value_objects::CombatInputKey;

/// 戦闘フレーム更新リクエスト
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct CombatFrameRequest {
    /// プレイヤー1の入力
    pub player1_input: CombatInputKey,

    /// プレイヤー2の入力
    pub player2_input: CombatInputKey,

    /// 現在のフレーム番号
    pub frame_number: u32,
}

/// 戦闘開始リクエスト
#[derive(Debug, Clone)]
pub struct StartCombatRequest {
    /// プレイヤー1のユーザーID
    pub player1_id: String,

    /// プレイヤー2のユーザーID
    pub player2_id: String,

    /// ラウンド時間（秒）
    pub round_time_seconds: u32,
}
