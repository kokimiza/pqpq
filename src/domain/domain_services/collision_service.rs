use crate::domain::entities::{Facing, Fighter, FighterState};

/// 当たり判定サービス
///
/// 2体のFighterの位置・状態から、攻撃ヒット判定を行う。
/// 単独のFighterエンティティで実行すると違和感があるため、
/// ドメインサービスとして定義。
pub struct CollisionService;

/// 当たり判定の結果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CollisionResult {
    /// ヒットなし
    NoHit,

    /// 攻撃がヒット
    Hit {
        /// ダメージ量
        damage: u8,
        /// ヒットストップフレーム数
        hitstop_frames: u8,
    },

    /// ガード成功
    Guarded {
        /// ガード削りダメージ
        chip_damage: u8,
        /// ガード硬直フレーム数
        blockstun_frames: u8,
    },
}

impl CollisionService {
    /// 2体のファイター間の当たり判定を実行
    ///
    /// # 引数
    /// * `attacker` - 攻撃側のファイター
    /// * `defender` - 防御側のファイター
    ///
    /// # 戻り値
    /// * 当たり判定の結果
    pub fn check_collision(attacker: &Fighter, defender: &Fighter) -> CollisionResult {
        // 攻撃中でなければヒット判定なし
        let attack_frame = match attacker.state() {
            FighterState::Attacking { frame, attack_type } => (*frame, *attack_type),
            _ => return CollisionResult::NoHit,
        };

        // 攻撃判定の発生フレーム（3-8フレーム目に判定発生と仮定）
        if attack_frame.0 < 3 || attack_frame.0 > 8 {
            return CollisionResult::NoHit;
        }

        // 距離判定（攻撃リーチ内か）
        let distance = Self::calculate_distance(attacker, defender);
        let attack_reach = Self::get_attack_reach(attack_frame.1);

        if distance > attack_reach {
            return CollisionResult::NoHit;
        }

        // 向きが合っているか（相手の方を向いて攻撃しているか）
        if !Self::is_facing_opponent(attacker, defender) {
            return CollisionResult::NoHit;
        }

        // ダメージ計算
        let base_damage = Self::get_attack_damage(attack_frame.1);

        // TODO: ガード判定（将来実装）
        // 現在は常にヒット扱い
        CollisionResult::Hit {
            damage: base_damage,
            hitstop_frames: 4,
        }
    }

    /// 2体のファイター間の距離を計算
    fn calculate_distance(fighter1: &Fighter, fighter2: &Fighter) -> u8 {
        let dx = (fighter1.position_x() as i16 - fighter2.position_x() as i16).abs();
        let dy = (fighter1.position_y() as i16 - fighter2.position_y() as i16).abs();

        // マンハッタン距離を使用（簡易的な距離計算）
        (dx + dy).min(255) as u8
    }

    /// 攻撃のリーチを取得
    fn get_attack_reach(attack_type: u8) -> u8 {
        match attack_type {
            0 => 3, // 弱パンチ
            1 => 5, // 強パンチ
            2 => 4, // 弱キック
            3 => 6, // 強キック
            _ => 4, // デフォルト
        }
    }

    /// 攻撃のダメージを取得
    fn get_attack_damage(attack_type: u8) -> u8 {
        match attack_type {
            0 => 10, // 弱パンチ
            1 => 25, // 強パンチ
            2 => 15, // 弱キック
            3 => 30, // 強キック
            _ => 10, // デフォルト
        }
    }

    /// 攻撃者が防御者の方を向いているか確認
    fn is_facing_opponent(attacker: &Fighter, defender: &Fighter) -> bool {
        match attacker.facing() {
            Facing::Right => attacker.position_x() < defender.position_x(),
            Facing::Left => attacker.position_x() > defender.position_x(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value_objects::PublicKey;

    #[test]
    fn test_no_collision_when_idle() {
        let attacker = Fighter::new(PublicKey::from_bytes(&[1u8; 32]), 10, 10, Facing::Right);
        let defender = Fighter::new(PublicKey::from_bytes(&[2u8; 32]), 15, 10, Facing::Left);

        let result = CollisionService::check_collision(&attacker, &defender);
        assert_eq!(result, CollisionResult::NoHit);
    }

    #[test]
    fn test_collision_when_attacking() {
        let mut attacker = Fighter::new(PublicKey::from_bytes(&[1u8; 32]), 10, 10, Facing::Right);
        attacker.set_state(FighterState::Attacking {
            frame: 5,
            attack_type: 1,
        });

        let defender = Fighter::new(PublicKey::from_bytes(&[2u8; 32]), 13, 10, Facing::Left);

        let result = CollisionService::check_collision(&attacker, &defender);
        assert!(matches!(result, CollisionResult::Hit { .. }));
    }

    #[test]
    fn test_no_collision_when_too_far() {
        let mut attacker = Fighter::new(PublicKey::from_bytes(&[1u8; 32]), 10, 10, Facing::Right);
        attacker.set_state(FighterState::Attacking {
            frame: 5,
            attack_type: 0,
        });

        let defender = Fighter::new(PublicKey::from_bytes(&[2u8; 32]), 30, 10, Facing::Left);

        let result = CollisionService::check_collision(&attacker, &defender);
        assert_eq!(result, CollisionResult::NoHit);
    }
}
