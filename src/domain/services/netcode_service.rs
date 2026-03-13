use crate::domain::entities::Fighter;
use crate::domain::value_objects::CombatInputKey;

/// ロールバック統計情報
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct NetcodeStats {
    pub current_frame: u32,
    pub state_history_size: usize,
    pub input_history_size: usize,
    pub confirmed_frames: usize,
    pub pending_frames: usize,
}

/// ロールバック・ネットコード・サービス
///
/// GGPO スタイルのロールバック機能を提供するドメインサービス
#[cfg_attr(test, mockall::automock)]
pub trait NetcodeService: Send + Sync {
    /// 現在のゲーム状態を保存
    fn save_state(&mut self, fighter1: Fighter, fighter2: Fighter);

    /// 入力を追加（ローカル入力は確定、リモート入力は予測）
    fn add_input(&mut self, local_input: CombatInputKey, remote_input: Option<CombatInputKey>);

    /// リモート入力を確定（予測が外れた場合はtrueを返す）
    fn confirm_remote_input(&mut self, frame: u32, remote_input: CombatInputKey) -> bool;

    /// 指定フレームの状態を取得
    fn get_state(&self, frame: u32) -> Option<(Fighter, Fighter)>;

    /// 指定フレームの入力を取得
    fn get_input(&self, frame: u32) -> Option<(CombatInputKey, Option<CombatInputKey>)>;

    /// 現在のフレーム番号を取得
    fn current_frame(&self) -> u32;

    /// フレームを進める
    fn advance_frame(&mut self);

    /// 入力遅延を考慮した実行フレームを取得
    #[allow(dead_code)]
    fn get_execution_frame(&self) -> u32;

    /// ロールバックが必要な最も古いフレームを取得
    fn get_rollback_frame(&self) -> Option<u32>;

    /// 状態をクリア
    fn clear(&mut self);

    /// 統計情報を取得
    #[allow(dead_code)]
    fn get_stats(&self) -> NetcodeStats;
}

/// 入力シリアライザー・サービス
///
/// WebRTC DataChannelで送信するための入力データのシリアライズ/デシリアライズ
#[cfg_attr(test, mockall::automock)]
pub trait InputSerializer: Send + Sync {
    /// 入力をバイナリにシリアライズ
    fn serialize(&self, frame: u32, input: CombatInputKey) -> Vec<u8>;

    /// バイナリから入力をデシリアライズ
    fn deserialize(&self, data: &[u8]) -> anyhow::Result<(u32, CombatInputKey)>;

    /// 複数の入力をバッチでシリアライズ
    #[allow(dead_code)]
    fn serialize_batch(&self, inputs: &[(u32, CombatInputKey)]) -> Vec<u8>;

    /// バッチデータをデシリアライズ
    #[allow(dead_code)]
    fn deserialize_batch(&self, data: &[u8]) -> anyhow::Result<Vec<(u32, CombatInputKey)>>;
}
