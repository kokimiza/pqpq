use crate::domain::domain_services::MatchState;

/// 戦闘フレーム更新レスポンス
#[derive(Debug, Clone)]
pub struct CombatFrameResponse {
    /// プレイヤー1の位置X
    pub player1_x: u8,

    /// プレイヤー1の位置Y
    pub player1_y: u8,

    /// プレイヤー1のHP
    pub player1_hp: u8,

    /// プレイヤー1の状態（文字列表現）
    pub player1_state: String,

    /// プレイヤー2の位置X
    pub player2_x: u8,

    /// プレイヤー2の位置Y
    pub player2_y: u8,

    /// プレイヤー2のHP
    pub player2_hp: u8,

    /// プレイヤー2の状態（文字列表現）
    pub player2_state: String,

    /// 対戦状態
    pub match_state: MatchState,

    /// 残り時間（フレーム数）
    pub remaining_frames: u32,

    /// 勝者のプレイヤー番号（1 or 2、引き分けの場合は0）
    pub winner: u8,
}

/// 戦闘開始レスポンス
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StartCombatResponse {
    /// カウントダウン開始フレーム
    pub countdown_frames: u32,

    /// 総ラウンドフレーム数
    pub total_frames: u32,

    /// 成功メッセージ
    pub message: String,
}
