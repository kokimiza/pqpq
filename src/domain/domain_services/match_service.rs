use crate::domain::entities::{Fighter, FinishType, MatchResult, User};
use crate::domain::value_objects::PublicKey;

/// 対戦管理サービス
///
/// 対戦の開始、進行、終了を管理する。
/// 複数のエンティティ（Fighter、User、MatchResult）にまたがる処理を扱う。
pub struct MatchService;

/// 対戦の状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchState {
    /// 準備中
    Preparing,

    /// 対戦中
    InProgress,

    /// 終了（勝者が決定）
    Finished,

    /// 引き分け
    Draw,
}

/// 対戦結果
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MatchOutcome {
    /// 勝者の公開鍵
    pub winner_pubkey: Option<PublicKey>,

    /// 敗者の公開鍵
    pub loser_pubkey: Option<PublicKey>,

    /// スコア（勝者のラウンド数, 敗者のラウンド数）
    pub score: (u8, u8),

    /// 対戦状態
    pub state: MatchState,
}

impl MatchService {
    /// 対戦を初期化
    ///
    /// # 引数
    /// * `player1` - プレイヤー1
    /// * `player2` - プレイヤー2
    ///
    /// # 戻り値
    /// * (Fighter1, Fighter2) - 初期配置されたファイター
    pub fn initialize_match(player1: &User, player2: &User) -> (Fighter, Fighter) {
        // プレイヤー1は左側、プレイヤー2は右側に配置
        let fighter1 = Fighter::new(
            PublicKey::new(player1.id().to_string()).unwrap(),
            15,
            20,
            crate::domain::entities::Facing::Right,
        );

        let fighter2 = Fighter::new(
            PublicKey::new(player2.id().to_string()).unwrap(),
            65,
            20,
            crate::domain::entities::Facing::Left,
        );

        (fighter1, fighter2)
    }

    /// 対戦の勝敗を判定
    ///
    /// # 引数
    /// * `fighter1` - ファイター1
    /// * `fighter2` - ファイター2
    ///
    /// # 戻り値
    /// * 対戦結果
    pub fn determine_outcome(fighter1: &Fighter, fighter2: &Fighter) -> MatchOutcome {
        let hp1 = fighter1.hp();
        let hp2 = fighter2.hp();

        if hp1 == 0 && hp2 == 0 {
            // 両者KO（引き分け）
            return MatchOutcome {
                winner_pubkey: None,
                loser_pubkey: None,
                score: (0, 0),
                state: MatchState::Draw,
            };
        }

        if hp1 == 0 {
            // プレイヤー2の勝利
            return MatchOutcome {
                winner_pubkey: Some(fighter2.pubkey().clone()),
                loser_pubkey: Some(fighter1.pubkey().clone()),
                score: (0, 1),
                state: MatchState::Finished,
            };
        }

        if hp2 == 0 {
            // プレイヤー1の勝利
            return MatchOutcome {
                winner_pubkey: Some(fighter1.pubkey().clone()),
                loser_pubkey: Some(fighter2.pubkey().clone()),
                score: (1, 0),
                state: MatchState::Finished,
            };
        }

        // まだ対戦中
        MatchOutcome {
            winner_pubkey: None,
            loser_pubkey: None,
            score: (0, 0),
            state: MatchState::InProgress,
        }
    }

    /// 対戦結果を記録用のエンティティに変換
    ///
    /// # 引数
    /// * `outcome` - 対戦結果
    /// * `fighter1` - ファイター1（HP取得用）
    /// * `fighter2` - ファイター2（HP取得用）
    /// * `duration_frames` - 対戦時間（フレーム数）
    /// * `finish_type` - 終了種別
    /// * `loser_signature` - 敗者による署名（Base64）
    /// * `ring_id` - 対戦が行われたリングのID
    ///
    /// # 戻り値
    /// * MatchResultエンティティ（記録可能な場合）
    #[allow(dead_code)]
    pub fn create_match_result(
        outcome: &MatchOutcome,
        fighter1: &Fighter,
        fighter2: &Fighter,
        duration_frames: u32,
        finish_type: FinishType,
        loser_signature: String,
        ring_id: Option<uuid::Uuid>,
    ) -> Option<MatchResult> {
        if outcome.state != MatchState::Finished {
            return None;
        }

        let winner_pubkey = outcome.winner_pubkey.as_ref()?;
        let loser_pubkey = outcome.loser_pubkey.as_ref()?;

        // 勝者/敗者のHPを特定
        let (winner_hp, loser_hp) = if winner_pubkey == fighter1.pubkey() {
            (fighter1.hp(), fighter2.hp())
        } else {
            (fighter2.hp(), fighter1.hp())
        };

        let raw_payload = format!(
            "winner:{},loser:{},whp:{},lhp:{},frames:{},type:{}",
            winner_pubkey.value(),
            loser_pubkey.value(),
            winner_hp,
            loser_hp,
            duration_frames,
            finish_type.as_str()
        );

        Some(MatchResult::new(
            winner_pubkey.clone(),
            loser_pubkey.clone(),
            winner_hp,
            loser_hp,
            duration_frames,
            finish_type,
            raw_payload,
            loser_signature,
            ring_id,
        ))
    }

    /// タイムアウトによる判定
    ///
    /// 残りHPが多い方を勝者とする。
    ///
    /// # 引数
    /// * `fighter1` - ファイター1
    /// * `fighter2` - ファイター2
    ///
    /// # 戻り値
    /// * 対戦結果
    pub fn timeout_decision(fighter1: &Fighter, fighter2: &Fighter) -> MatchOutcome {
        let hp1 = fighter1.hp();
        let hp2 = fighter2.hp();

        if hp1 > hp2 {
            MatchOutcome {
                winner_pubkey: Some(fighter1.pubkey().clone()),
                loser_pubkey: Some(fighter2.pubkey().clone()),
                score: (1, 0),
                state: MatchState::Finished,
            }
        } else if hp2 > hp1 {
            MatchOutcome {
                winner_pubkey: Some(fighter2.pubkey().clone()),
                loser_pubkey: Some(fighter1.pubkey().clone()),
                score: (0, 1),
                state: MatchState::Finished,
            }
        } else {
            MatchOutcome {
                winner_pubkey: None,
                loser_pubkey: None,
                score: (0, 0),
                state: MatchState::Draw,
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
    fn test_initialize_match() {
        let user1 = User::new("player1-addr".to_string(), "Player1".to_string());
        let user2 = User::new("player2-addr".to_string(), "Player2".to_string());

        let (fighter1, fighter2) = MatchService::initialize_match(&user1, &user2);

        assert_eq!(fighter1.position_x(), 15);
        assert_eq!(fighter2.position_x(), 65);
        assert_eq!(fighter1.facing(), Facing::Right);
        assert_eq!(fighter2.facing(), Facing::Left);
    }

    #[test]
    fn test_determine_outcome_player1_wins() {
        let fighter1 = Fighter::new(PublicKey::from_bytes(&[1u8; 32]), 15, 20, Facing::Right);
        let mut fighter2 = Fighter::new(PublicKey::from_bytes(&[2u8; 32]), 65, 20, Facing::Left);

        fighter2.take_damage(255); // KO

        let outcome = MatchService::determine_outcome(&fighter1, &fighter2);
        assert_eq!(outcome.state, MatchState::Finished);
        assert!(outcome.winner_pubkey.is_some());
        assert_eq!(outcome.score, (1, 0));
    }

    #[test]
    fn test_determine_outcome_draw() {
        let mut fighter1 = Fighter::new(PublicKey::from_bytes(&[1u8; 32]), 15, 20, Facing::Right);
        let mut fighter2 = Fighter::new(PublicKey::from_bytes(&[2u8; 32]), 65, 20, Facing::Left);

        fighter1.take_damage(255);
        fighter2.take_damage(255);

        let outcome = MatchService::determine_outcome(&fighter1, &fighter2);
        assert_eq!(outcome.state, MatchState::Draw);
        assert!(outcome.winner_pubkey.is_none());
    }
}
