use crate::application::dto::response::{CombatFrameResponse, StartCombatResponse};

/// 戦闘出力ポート
pub trait CombatOutputPort {
    /// 戦闘開始を表示
    fn present_start(&self, response: StartCombatResponse);

    /// フレーム更新を表示
    fn present_frame(&self, response: CombatFrameResponse);
}
