use crate::domain::entities::Fighter;
use crate::domain::services::{NetcodeService, NetcodeStats};
use crate::domain::value_objects::CombatInputKey;
use std::collections::VecDeque;

/// ロールバック用のゲーム状態スナップショット
#[derive(Clone, Debug)]
pub struct GameStateSnapshot {
    pub frame: u32,
    pub fighter1: Fighter,
    pub fighter2: Fighter,
}

/// 入力履歴エントリ
#[derive(Clone, Copy, Debug)]
pub struct InputHistoryEntry {
    pub frame: u32,
    pub local_input: CombatInputKey,
    pub remote_input: Option<CombatInputKey>,
    pub confirmed: bool,
}

/// GGPO スタイルのロールバック・ネットコード・マネージャー
///
/// 機能:
/// - 過去のゲーム状態を保存（最大8フレーム）
/// - 入力履歴を管理
/// - 予測が外れた場合、過去の状態に巻き戻して再シミュレート
/// - 入力のみを送信（状態は送信しない）
pub struct RollbackManager {
    /// 保存する最大フレーム数
    max_history: usize,
    /// ゲーム状態のスナップショット履歴
    state_history: VecDeque<GameStateSnapshot>,
    /// 入力履歴
    input_history: VecDeque<InputHistoryEntry>,
    /// 現在のフレーム番号
    current_frame: u32,
    /// 入力遅延フレーム数（デフォルト: 2）
    #[allow(dead_code)]
    input_delay: u32,
}

impl RollbackManager {
    /// 新しいロールバックマネージャーを作成
    pub fn new(max_history: usize, input_delay: u32) -> Self {
        Self {
            max_history,
            state_history: VecDeque::with_capacity(max_history),
            input_history: VecDeque::with_capacity(max_history * 2),
            current_frame: 0,
            input_delay,
        }
    }

    /// デフォルト設定で作成（8フレーム履歴、2フレーム遅延）
    pub fn default_config() -> Self {
        Self::new(8, 2)
    }
}

impl NetcodeService for RollbackManager {
    fn save_state(&mut self, fighter1: Fighter, fighter2: Fighter) {
        let snapshot = GameStateSnapshot {
            frame: self.current_frame,
            fighter1,
            fighter2,
        };

        self.state_history.push_back(snapshot);

        // 最大履歴数を超えたら古いものを削除
        if self.state_history.len() > self.max_history {
            self.state_history.pop_front();
        }
    }

    fn add_input(&mut self, local_input: CombatInputKey, remote_input: Option<CombatInputKey>) {
        let entry = InputHistoryEntry {
            frame: self.current_frame,
            local_input,
            remote_input,
            confirmed: remote_input.is_some(),
        };

        self.input_history.push_back(entry);

        // 古い入力履歴を削除
        while self.input_history.len() > self.max_history * 2 {
            self.input_history.pop_front();
        }
    }

    fn confirm_remote_input(&mut self, frame: u32, remote_input: CombatInputKey) -> bool {
        // 該当フレームの入力を探す
        for entry in self.input_history.iter_mut() {
            if entry.frame == frame {
                let needs_rollback = if let Some(predicted) = entry.remote_input {
                    // 予測と実際の入力が異なる場合はロールバックが必要
                    predicted.as_byte() != remote_input.as_byte()
                } else {
                    // 予測がなかった場合もロールバックが必要
                    true
                };

                entry.remote_input = Some(remote_input);
                entry.confirmed = true;

                return needs_rollback;
            }
        }

        false
    }

    fn get_state(&self, frame: u32) -> Option<(Fighter, Fighter)> {
        self.state_history
            .iter()
            .find(|s| s.frame == frame)
            .map(|s| (s.fighter1.clone(), s.fighter2.clone()))
    }

    fn get_input(&self, frame: u32) -> Option<(CombatInputKey, Option<CombatInputKey>)> {
        self.input_history
            .iter()
            .find(|e| e.frame == frame)
            .map(|e| (e.local_input, e.remote_input))
    }

    fn current_frame(&self) -> u32 {
        self.current_frame
    }

    fn advance_frame(&mut self) {
        self.current_frame += 1;
    }

    fn get_execution_frame(&self) -> u32 {
        self.current_frame.saturating_sub(self.input_delay)
    }

    fn get_rollback_frame(&self) -> Option<u32> {
        // 未確定の入力がある最も古いフレームを探す
        self.input_history
            .iter()
            .filter(|e| !e.confirmed)
            .map(|e| e.frame)
            .min()
    }

    fn clear(&mut self) {
        self.state_history.clear();
        self.input_history.clear();
        self.current_frame = 0;
    }

    fn get_stats(&self) -> NetcodeStats {
        let total_frames = self.input_history.len();
        let confirmed_frames = self.input_history.iter().filter(|e| e.confirmed).count();
        let pending_frames = total_frames - confirmed_frames;

        NetcodeStats {
            current_frame: self.current_frame,
            state_history_size: self.state_history.len(),
            input_history_size: self.input_history.len(),
            confirmed_frames,
            pending_frames,
        }
    }
}
