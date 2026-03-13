use crate::domain::value_objects::{CombatInputKey, InputHistory, PublicKey};

/// ファイターの向き
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    Left,
    Right,
}

/// ファイターのステート（状態）
///
/// 当たり判定とフレームデータの基礎となる状態管理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FighterState {
    /// 待機状態（構え）
    ///
    /// 移動可能、ガード可能、攻撃入力受付可能
    Idle,

    /// 攻撃中
    ///
    /// 攻撃判定発生中、移動不可、キャンセル可能フレームあり
    Attacking {
        /// 攻撃開始からの経過フレーム
        frame: u8,
        /// 攻撃の種類（パンチ/キック/特殊技など）
        attack_type: u8,
    },

    /// 硬直状態
    ///
    /// 攻撃後の隙、被弾時のヒットストップ、ガード硬直など
    Stunned {
        /// 硬直残りフレーム数
        remaining_frames: u8,
    },
}

/// 操作キャラクターエンティティ
///
/// 単なる座標データではなく、「当たり判定」と「ステート（構え・攻撃中・硬直）」を
/// 管理する中心エンティティ。ロールバックネットコード対応のため、
/// 高速なシリアライズ/デシリアライズが可能。
#[derive(Debug, Clone)]
pub struct Fighter {
    /// プレイヤーの公開鍵（識別子）
    pubkey: PublicKey,

    /// X座標（0-79、ターミナル幅に対応）
    position_x: u8,

    /// Y座標（0-23、ターミナル高さに対応）
    position_y: u8,

    /// X方向の速度（-127 〜 127）
    velocity_x: i8,

    /// Y方向の速度（-127 〜 127、ジャンプ/落下）
    velocity_y: i8,

    /// 体力（0-255）
    hp: u8,

    /// ファイターの状態
    state: FighterState,

    /// 向き
    facing: Facing,

    /// 入力履歴（隠しコマンド判定用）
    ///
    /// キャラクターごとに異なる隠しコマンドを持つため、
    /// Fighterエンティティが入力履歴を管理する。
    input_history: InputHistory,
}

impl Fighter {
    /// 新しいファイターを作成（初期配置）
    ///
    /// # 引数
    /// * `pubkey` - プレイヤーの公開鍵
    /// * `position_x` - 初期X座標
    /// * `position_y` - 初期Y座標（地面）
    /// * `facing` - 初期の向き
    pub fn new(pubkey: PublicKey, position_x: u8, position_y: u8, facing: Facing) -> Self {
        assert!(position_x < 80, "position_x must be < 80");
        assert!(position_y < 24, "position_y must be < 24");

        Self {
            pubkey,
            position_x,
            position_y,
            velocity_x: 0,
            velocity_y: 0,
            hp: 255,
            state: FighterState::Idle,
            facing,
            input_history: InputHistory::new(30), // 30フレーム分の履歴
        }
    }

    /// 入力を処理
    ///
    /// 入力履歴に追加し、隠しコマンドの判定に使用する。
    pub fn process_input(&mut self, input: CombatInputKey) {
        self.input_history.push(input);
    }

    /// 特定のコマンドパターンが入力されたか確認
    ///
    /// キャラクター固有の隠しコマンド判定に使用。
    ///
    /// # 引数
    /// * `pattern` - コマンドパターン（例: 波動拳コマンド）
    /// * `tolerance_frames` - パターン間の許容フレーム数
    #[allow(dead_code)]
    pub fn check_command(&self, pattern: &[u8], tolerance_frames: usize) -> bool {
        self.input_history
            .matches_pattern(pattern, tolerance_frames)
    }

    /// 入力履歴をクリア（ラウンド開始時など）
    #[allow(dead_code)]
    pub fn clear_input_history(&mut self) {
        self.input_history.clear();
    }

    /// 位置を更新
    pub fn update_position(&mut self, x: u8, y: u8) {
        assert!(x < 80, "position_x must be < 80");
        assert!(y < 24, "position_y must be < 24");
        self.position_x = x;
        self.position_y = y;
    }

    /// 速度を設定
    pub fn set_velocity(&mut self, vx: i8, vy: i8) {
        self.velocity_x = vx;
        self.velocity_y = vy;
    }

    /// ダメージを受ける
    ///
    /// # 戻り値
    /// * `true` - まだ生存している
    /// * `false` - HPが0になった（KO）
    pub fn take_damage(&mut self, damage: u8) -> bool {
        self.hp = self.hp.saturating_sub(damage);
        self.hp > 0
    }

    /// ステートを変更
    pub fn set_state(&mut self, state: FighterState) {
        self.state = state;
    }

    /// 向きを変更
    #[allow(dead_code)]
    pub fn set_facing(&mut self, facing: Facing) {
        self.facing = facing;
    }

    /// 相手の方向を向く
    pub fn face_opponent(&mut self, opponent_x: u8) {
        self.facing = if self.position_x < opponent_x {
            Facing::Right
        } else {
            Facing::Left
        };
    }

    pub fn pubkey(&self) -> &PublicKey {
        &self.pubkey
    }

    pub fn position_x(&self) -> u8 {
        self.position_x
    }

    pub fn position_y(&self) -> u8 {
        self.position_y
    }

    pub fn velocity_x(&self) -> i8 {
        self.velocity_x
    }

    pub fn velocity_y(&self) -> i8 {
        self.velocity_y
    }

    pub fn hp(&self) -> u8 {
        self.hp
    }

    pub fn state(&self) -> &FighterState {
        &self.state
    }

    pub fn facing(&self) -> Facing {
        self.facing
    }

    /// ロールバック用の高速スナップショット作成
    ///
    /// 全状態を固定長バイナリにシリアライズ。
    /// PublicKeyは32バイト固定と仮定。
    /// 入力履歴は最新16フレーム分のみ保存（ロールバック時の復元用）。
    #[allow(dead_code)]
    pub fn to_snapshot(&self) -> Vec<u8> {
        let mut snapshot = Vec::with_capacity(128);

        // PublicKey (32 bytes)
        snapshot.extend_from_slice(&self.pubkey.as_bytes());

        // Position (2 bytes)
        snapshot.push(self.position_x);
        snapshot.push(self.position_y);

        // Velocity (2 bytes)
        snapshot.push(self.velocity_x as u8);
        snapshot.push(self.velocity_y as u8);

        // HP (1 byte)
        snapshot.push(self.hp);

        // State (3 bytes: type + data)
        match self.state {
            FighterState::Idle => {
                snapshot.push(0);
                snapshot.push(0);
                snapshot.push(0);
            }
            FighterState::Attacking { frame, attack_type } => {
                snapshot.push(1);
                snapshot.push(frame);
                snapshot.push(attack_type);
            }
            FighterState::Stunned { remaining_frames } => {
                snapshot.push(2);
                snapshot.push(remaining_frames);
                snapshot.push(0);
            }
        }

        // Facing (1 byte)
        snapshot.push(match self.facing {
            Facing::Left => 0,
            Facing::Right => 1,
        });

        // Input history (1 byte for length + up to 16 bytes for inputs)
        let history_len = self.input_history.len().min(16);
        snapshot.push(history_len as u8);

        // 最新16フレーム分の入力履歴を保存
        let start_idx = self.input_history.len().saturating_sub(16);
        for i in start_idx..self.input_history.len() {
            if let Some(input) = self.input_history.history.get(i) {
                snapshot.push(input.as_byte());
            }
        }

        snapshot
    }

    /// スナップショットから復元
    ///
    /// # パニック
    /// * データが不正な場合
    #[allow(dead_code)]
    pub fn from_snapshot(data: &[u8]) -> Self {
        assert!(data.len() >= 42, "Invalid snapshot data");

        let pubkey = PublicKey::from_bytes(&data[0..32]);
        let position_x = data[32];
        let position_y = data[33];
        let velocity_x = data[34] as i8;
        let velocity_y = data[35] as i8;
        let hp = data[36];

        let state = match data[37] {
            0 => FighterState::Idle,
            1 => FighterState::Attacking {
                frame: data[38],
                attack_type: data[39],
            },
            2 => FighterState::Stunned {
                remaining_frames: data[38],
            },
            _ => panic!("Invalid state type"),
        };

        let facing = match data[40] {
            0 => Facing::Left,
            1 => Facing::Right,
            _ => panic!("Invalid facing"),
        };

        // 入力履歴の復元
        let mut input_history = InputHistory::new(30);
        if data.len() > 41 {
            let history_len = data[41] as usize;
            for i in 0..history_len {
                if let Some(&byte) = data.get(42 + i) {
                    input_history.push(CombatInputKey::from_byte(byte));
                }
            }
        }

        Self {
            pubkey,
            position_x,
            position_y,
            velocity_x,
            velocity_y,
            hp,
            state,
            facing,
            input_history,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fighter_creation() {
        let pubkey = PublicKey::from_bytes(&[0u8; 32]);
        let fighter = Fighter::new(pubkey, 10, 20, Facing::Right);

        assert_eq!(fighter.position_x(), 10);
        assert_eq!(fighter.position_y(), 20);
        assert_eq!(fighter.hp(), 255);
        assert_eq!(fighter.facing(), Facing::Right);
    }

    #[test]
    fn test_take_damage() {
        let pubkey = PublicKey::from_bytes(&[0u8; 32]);
        let mut fighter = Fighter::new(pubkey, 10, 20, Facing::Right);

        assert!(fighter.take_damage(50));
        assert_eq!(fighter.hp(), 205);

        assert!(!fighter.take_damage(250));
        assert_eq!(fighter.hp(), 0);
    }

    #[test]
    fn test_snapshot_roundtrip() {
        let pubkey = PublicKey::from_bytes(&[1u8; 32]);
        let mut fighter = Fighter::new(pubkey.clone(), 15, 10, Facing::Left);
        fighter.set_velocity(5, -3);
        fighter.take_damage(50);
        fighter.set_state(FighterState::Attacking {
            frame: 3,
            attack_type: 1,
        });

        let snapshot = fighter.to_snapshot();
        let restored = Fighter::from_snapshot(&snapshot);

        assert_eq!(restored.position_x(), 15);
        assert_eq!(restored.position_y(), 10);
        assert_eq!(restored.velocity_x(), 5);
        assert_eq!(restored.velocity_y(), -3);
        assert_eq!(restored.hp(), 205);
        assert_eq!(restored.facing(), Facing::Left);

        if let FighterState::Attacking { frame, attack_type } = restored.state() {
            assert_eq!(*frame, 3);
            assert_eq!(*attack_type, 1);
        } else {
            panic!("State not restored correctly");
        }
    }
}
