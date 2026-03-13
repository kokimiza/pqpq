use crate::domain::entities::{Fighter, FighterState};

/// 物理演算サービス
///
/// ファイターの移動、重力、地面判定などの物理演算を行う。
/// 複数のファイターに対して同じロジックを適用するため、
/// ドメインサービスとして定義。
///
/// # 運動方程式
///
/// Semi-implicit Euler法（速度ベースの積分法）を採用:
///
/// ```text
/// v(t+1) = v(t) + a * Δt
/// x(t+1) = x(t) + v(t+1) * Δt
/// ```
///
/// この手法は以下の理由で格闘ゲームに最適:
///
/// 1. **数値安定性**: 速度を先に更新するため、エネルギーが増加しにくい
/// 2. **シンプル**: 実装が簡単で、デバッグしやすい
/// 3. **予測可能**: プレイヤーの入力に対する応答が一貫している
/// 4. **実績**: 多くの商用格闘ゲーム・アクションゲームで採用されている
///
/// # フレームレート
///
/// 格闘ゲームは通常60FPSで動作するため、Δt = 1/60 ≈ 0.0167秒。
/// 内部的にはΔt = 1.0として扱い、物理定数を調整する。
pub struct PhysicsService;

/// ゲーム世界の物理定数
///
/// 格闘ゲームの「手触り」を決定する重要なパラメータ。
/// これらの値は60FPSを前提に調整されている。
#[derive(Debug, Clone, Copy)]
pub struct PhysicsConstants {
    /// 重力加速度（1フレームあたりの速度変化）
    ///
    /// 値が大きいほど、キャラクターは速く落下する。
    /// 格闘ゲームでは2-3が一般的。
    pub gravity: i8,

    /// 地面のY座標
    ///
    /// ターミナルの高さ24を想定し、下部に地面を配置。
    pub ground_y: u8,

    /// 最大落下速度
    ///
    /// 無限に加速しないよう、落下速度に上限を設ける。
    pub max_fall_speed: i8,

    /// ジャンプ初速度（負の値 = 上方向）
    ///
    /// 値が大きいほど、高くジャンプする。
    /// -8 〜 -12が一般的。
    pub jump_velocity: i8,

    /// 移動速度
    ///
    /// 1フレームあたりの水平移動量。
    /// 2-3が一般的（速すぎると操作が難しくなる）。
    pub walk_speed: i8,
}

impl Default for PhysicsConstants {
    fn default() -> Self {
        Self {
            gravity: 2,
            ground_y: 20,
            max_fall_speed: 10,
            jump_velocity: -8,
            walk_speed: 2,
        }
    }
}

impl PhysicsService {
    /// ファイターの物理演算を1フレーム進める
    ///
    /// Semi-implicit Euler法を使用:
    /// 1. v(t+1) = v(t) + a * Δt  (速度を先に更新)
    /// 2. x(t+1) = x(t) + v(t+1) * Δt  (更新後の速度で位置を更新)
    ///
    /// この方法は数値的に安定で、多くの商用格闘ゲームで採用されている。
    ///
    /// # 引数
    /// * `fighter` - 更新対象のファイター
    /// * `constants` - 物理定数
    /// * `delta_time` - フレーム時間（通常は1.0、スローモーション時は0.5など）
    pub fn update_physics(fighter: &mut Fighter, constants: &PhysicsConstants, delta_time: f32) {
        let mut vx = fighter.velocity_x();
        let mut vy = fighter.velocity_y();

        // ステップ1: 速度を更新 v(t+1) = v(t) + a * Δt

        // 重力加速度を適用（空中にいる場合）
        if fighter.position_y() < constants.ground_y {
            vy = (vy + (constants.gravity as f32 * delta_time) as i8).min(constants.max_fall_speed);
        }

        // 空気抵抗（X方向の減衰）
        if vx != 0 {
            let friction = 0.9; // 摩擦係数
            vx = (vx as f32 * friction) as i8;
            if vx.abs() < 1 {
                vx = 0;
            }
        }

        // 速度を更新
        fighter.set_velocity(vx, vy);

        // ステップ2: 位置を更新 x(t+1) = x(t) + v(t+1) * Δt
        let new_x = Self::apply_velocity_x(fighter.position_x(), (vx as f32 * delta_time) as i8);
        let new_y = Self::apply_velocity_y(
            fighter.position_y(),
            (vy as f32 * delta_time) as i8,
            constants,
        );

        fighter.update_position(new_x, new_y);

        // 地面に着地した場合、Y速度をリセット
        if fighter.position_y() >= constants.ground_y && vy > 0 {
            fighter.set_velocity(vx, 0);
        }
    }

    /// X方向の速度を位置に適用
    fn apply_velocity_x(current_x: u8, velocity_x: i8) -> u8 {
        let new_x = current_x as i16 + velocity_x as i16;
        new_x.clamp(0, 79) as u8
    }

    /// Y方向の速度を位置に適用
    fn apply_velocity_y(current_y: u8, velocity_y: i8, constants: &PhysicsConstants) -> u8 {
        let new_y = current_y as i16 + velocity_y as i16;
        new_y.clamp(0, constants.ground_y as i16) as u8
    }

    /// ファイターをジャンプさせる
    ///
    /// # 引数
    /// * `fighter` - ジャンプするファイター
    /// * `constants` - 物理定数
    ///
    /// # 戻り値
    /// * `true` - ジャンプ成功
    /// * `false` - ジャンプ失敗（空中にいる、硬直中など）
    pub fn try_jump(fighter: &mut Fighter, constants: &PhysicsConstants) -> bool {
        // 地面にいて、かつIdleまたはAttacking状態の場合のみジャンプ可能
        if fighter.position_y() < constants.ground_y {
            return false;
        }

        match fighter.state() {
            FighterState::Idle | FighterState::Attacking { .. } => {
                fighter.set_velocity(fighter.velocity_x(), constants.jump_velocity);
                true
            }
            FighterState::Stunned { .. } => false,
        }
    }

    /// ファイターを移動させる
    ///
    /// # 引数
    /// * `fighter` - 移動するファイター
    /// * `direction` - 移動方向（-1: 左, 0: 停止, 1: 右）
    /// * `constants` - 物理定数
    ///
    /// # 戻り値
    /// * `true` - 移動成功
    /// * `false` - 移動失敗（硬直中など）
    pub fn try_move(fighter: &mut Fighter, direction: i8, constants: &PhysicsConstants) -> bool {
        match fighter.state() {
            FighterState::Idle => {
                let velocity_x = direction * constants.walk_speed;
                fighter.set_velocity(velocity_x, fighter.velocity_y());
                true
            }
            FighterState::Attacking { .. } | FighterState::Stunned { .. } => false,
        }
    }

    /// 2体のファイターが重ならないように押し出す
    ///
    /// # 引数
    /// * `fighter1` - ファイター1
    /// * `fighter2` - ファイター2
    pub fn resolve_overlap(fighter1: &mut Fighter, fighter2: &mut Fighter) {
        let distance = (fighter1.position_x() as i16 - fighter2.position_x() as i16).abs();

        // 重なり判定（2キャラクター分の幅を3と仮定）
        if distance < 3 {
            let push_amount = (3 - distance) / 2;

            if fighter1.position_x() < fighter2.position_x() {
                // fighter1が左、fighter2が右
                let new_x1 = fighter1.position_x().saturating_sub(push_amount as u8);
                let new_x2 = (fighter2.position_x() + push_amount as u8).min(79);
                fighter1.update_position(new_x1, fighter1.position_y());
                fighter2.update_position(new_x2, fighter2.position_y());
            } else {
                // fighter2が左、fighter1が右
                let new_x2 = fighter2.position_x().saturating_sub(push_amount as u8);
                let new_x1 = (fighter1.position_x() + push_amount as u8).min(79);
                fighter1.update_position(new_x1, fighter1.position_y());
                fighter2.update_position(new_x2, fighter2.position_y());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::Facing;
    use crate::domain::value_objects::PublicKey;

    #[test]
    fn test_gravity_application() {
        let mut fighter = Fighter::new(PublicKey::from_bytes(&[1u8; 32]), 40, 10, Facing::Right);

        let constants = PhysicsConstants::default();
        PhysicsService::update_physics(&mut fighter, &constants, 1.0);

        // 重力が適用されて下に移動
        assert!(fighter.position_y() > 10);
    }

    #[test]
    fn test_jump() {
        let mut fighter = Fighter::new(PublicKey::from_bytes(&[1u8; 32]), 40, 20, Facing::Right);

        let constants = PhysicsConstants::default();
        let success = PhysicsService::try_jump(&mut fighter, &constants);

        assert!(success);
        assert_eq!(fighter.velocity_y(), constants.jump_velocity);
    }

    #[test]
    fn test_cannot_jump_in_air() {
        let mut fighter = Fighter::new(PublicKey::from_bytes(&[1u8; 32]), 40, 10, Facing::Right);

        let constants = PhysicsConstants::default();
        let success = PhysicsService::try_jump(&mut fighter, &constants);

        assert!(!success);
    }
}
