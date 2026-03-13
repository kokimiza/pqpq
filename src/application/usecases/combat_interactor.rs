use crate::application::dto::request::{CombatFrameRequest, StartCombatRequest};
use crate::application::dto::response::{CombatFrameResponse, StartCombatResponse};
use crate::application::errors::ApplicationError;
use crate::application::ports::input::CombatInputPort;
use crate::domain::domain_services::{
    CollisionResult, CollisionService, MatchService, MatchState, PhysicsConstants, PhysicsService,
};
use crate::domain::entities::{Fighter, FighterState, User};
use crate::domain::services::{InputSerializer, NetcodeService, P2PService};
use crate::domain::value_objects::CombatInputKey;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 戦闘状態
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CombatPhase {
    /// カウントダウン中（3, 2, 1）
    Countdown,
    /// 戦闘中
    Fighting,
    /// 終了
    Finished,
}

#[derive(Clone, Copy)]
struct CombatState {
    physics_constants: PhysicsConstants,
    phase: CombatPhase,
    countdown_frames: u32,
    total_frames: u32,
    elapsed_frames: u32,
}

/// 戦闘インタラクター
///
/// 戦闘の全体フローを管理する:
/// 1. カウントダウン（3, 2, 1）
/// 2. 戦闘開始（両者操作可能）
/// 3. 毎フレーム更新（入力処理、物理演算、当たり判定）
/// 4. ロールバック・ネットコード（予測が外れた場合は巻き戻し）
/// 5. 勝敗判定（HP 0 またはタイムアップ）
pub struct CombatInteractor<N: NetcodeService, S: InputSerializer> {
    fighter1: Mutex<Option<Fighter>>,
    fighter2: Mutex<Option<Fighter>>,
    state: Mutex<CombatState>,
    netcode: Mutex<N>,
    p2p: Arc<dyn P2PService>,
    serializer: Arc<S>,
    is_host: bool,
}

impl<N: NetcodeService, S: InputSerializer> CombatInteractor<N, S> {
    pub fn new(netcode: N, p2p: Arc<dyn P2PService>, serializer: Arc<S>, is_host: bool) -> Self {
        Self {
            fighter1: Mutex::new(None),
            fighter2: Mutex::new(None),
            state: Mutex::new(CombatState {
                physics_constants: PhysicsConstants::default(),
                phase: CombatPhase::Countdown,
                countdown_frames: 180,
                total_frames: 0,
                elapsed_frames: 0,
            }),
            netcode: Mutex::new(netcode),
            p2p,
            serializer,
            is_host,
        }
    }

    fn process_input(
        fighter: &mut Fighter,
        input: &CombatInputKey,
        physics_constants: &PhysicsConstants,
    ) {
        fighter.process_input(*input);

        if matches!(fighter.state(), FighterState::Stunned { .. }) {
            return;
        }

        if input.is_punch() {
            fighter.set_state(FighterState::Attacking {
                frame: 0,
                attack_type: 0,
            });
            return;
        }

        if input.is_kick() {
            fighter.set_state(FighterState::Attacking {
                frame: 0,
                attack_type: 2,
            });
            return;
        }

        if matches!(fighter.state(), FighterState::Idle) {
            let horizontal = input.horizontal();
            if horizontal != 0 {
                PhysicsService::try_move(fighter, horizontal, physics_constants);
            }

            if input.is_up() {
                PhysicsService::try_jump(fighter, physics_constants);
            }
        }
    }

    fn check_collisions(fighter1: &mut Fighter, fighter2: &mut Fighter) {
        let result1 = CollisionService::check_collision(fighter1, fighter2);
        match result1 {
            CollisionResult::Hit {
                damage,
                hitstop_frames,
            } => {
                fighter2.take_damage(damage);
                fighter2.set_state(FighterState::Stunned {
                    remaining_frames: hitstop_frames,
                });
            }
            CollisionResult::Guarded {
                chip_damage,
                blockstun_frames,
            } => {
                fighter2.take_damage(chip_damage);
                fighter2.set_state(FighterState::Stunned {
                    remaining_frames: blockstun_frames,
                });
            }
            CollisionResult::NoHit => {}
        }

        let result2 = CollisionService::check_collision(fighter2, fighter1);
        match result2 {
            CollisionResult::Hit {
                damage,
                hitstop_frames,
            } => {
                fighter1.take_damage(damage);
                fighter1.set_state(FighterState::Stunned {
                    remaining_frames: hitstop_frames,
                });
            }
            CollisionResult::Guarded {
                chip_damage,
                blockstun_frames,
            } => {
                fighter1.take_damage(chip_damage);
                fighter1.set_state(FighterState::Stunned {
                    remaining_frames: blockstun_frames,
                });
            }
            CollisionResult::NoHit => {}
        }
    }

    fn update_fighter_states(fighter: &mut Fighter) {
        match fighter.state() {
            FighterState::Attacking { frame, attack_type } => {
                let new_frame = frame + 1;
                if new_frame >= 15 {
                    fighter.set_state(FighterState::Idle);
                } else {
                    fighter.set_state(FighterState::Attacking {
                        frame: new_frame,
                        attack_type: *attack_type,
                    });
                }
            }
            FighterState::Stunned { remaining_frames } => {
                if *remaining_frames > 0 {
                    fighter.set_state(FighterState::Stunned {
                        remaining_frames: remaining_frames - 1,
                    });
                } else {
                    fighter.set_state(FighterState::Idle);
                }
            }
            FighterState::Idle => {}
        }
    }

    /// ゲームロジックを1フレーム実行（ロールバック用）
    fn simulate_frame(
        fighter1: &mut Fighter,
        fighter2: &mut Fighter,
        input1: CombatInputKey,
        input2: CombatInputKey,
        physics_constants: &PhysicsConstants,
    ) {
        Self::process_input(fighter1, &input1, physics_constants);
        Self::process_input(fighter2, &input2, physics_constants);

        PhysicsService::update_physics(fighter1, physics_constants, 1.0);
        PhysicsService::update_physics(fighter2, physics_constants, 1.0);
        PhysicsService::resolve_overlap(fighter1, fighter2);

        fighter1.face_opponent(fighter2.position_x());
        fighter2.face_opponent(fighter1.position_x());

        Self::check_collisions(fighter1, fighter2);
        Self::update_fighter_states(fighter1);
        Self::update_fighter_states(fighter2);
    }

    fn create_response(
        fighter1: &Fighter,
        fighter2: &Fighter,
        phase: CombatPhase,
        elapsed_frames: u32,
        total_frames: u32,
    ) -> CombatFrameResponse {
        let outcome = MatchService::determine_outcome(fighter1, fighter2);

        let match_state = if phase == CombatPhase::Finished {
            if elapsed_frames >= total_frames {
                let timeout_outcome = MatchService::timeout_decision(fighter1, fighter2);
                timeout_outcome.state
            } else {
                outcome.state
            }
        } else if phase == CombatPhase::Countdown {
            MatchState::Preparing
        } else {
            outcome.state
        };

        let winner = if match_state == MatchState::Finished {
            if outcome.winner_pubkey.as_ref() == Some(fighter1.pubkey()) {
                1
            } else if outcome.winner_pubkey.as_ref() == Some(fighter2.pubkey()) {
                2
            } else {
                0
            }
        } else {
            0
        };

        CombatFrameResponse {
            player1_x: fighter1.position_x(),
            player1_y: fighter1.position_y(),
            player1_hp: fighter1.hp(),
            player1_state: format!("{:?}", fighter1.state()),
            player2_x: fighter2.position_x(),
            player2_y: fighter2.position_y(),
            player2_hp: fighter2.hp(),
            player2_state: format!("{:?}", fighter2.state()),
            match_state,
            remaining_frames: total_frames.saturating_sub(elapsed_frames),
            winner,
        }
    }

    /// ローカル入力をP2P経由で送信
    async fn send_local_input(&self, input: CombatInputKey, frame: u32) -> Result<()> {
        if !self.p2p.is_connected().await {
            // 接続が確立されていない場合はスキップ
            return Ok(());
        }

        let data = self.serializer.serialize(frame, input);
        self.p2p.send(&data).await?;
        Ok(())
    }

    /// リモート入力をP2P経由で受信し、Netcodeに確定入力として反映
    async fn receive_remote_input(
        &self,
        netcode: &mut N,
        current_frame: u32,
    ) -> Option<CombatInputKey> {
        if !self.p2p.is_connected().await {
            return None;
        }

        let received = self.p2p.receive_all().await;
        if received.is_empty() {
            return None;
        }

        let mut latest_input: Option<(u32, CombatInputKey)> = None;
        for data in received {
            if let Ok((frame, input)) = self.serializer.deserialize(&data) {
                // 受信した確定入力をNetcodeに反映（ロールバック判定に使用）
                netcode.confirm_remote_input(frame, input);

                match latest_input {
                    Some((latest_frame, _)) if frame > latest_frame => {
                        latest_input = Some((frame, input));
                    }
                    None => {
                        latest_input = Some((frame, input));
                    }
                    _ => {}
                }
            }
        }

        if let Some((frame, input)) = latest_input
            && frame >= current_frame
        {
            Some(input)
        } else {
            None
        }
    }
}

#[async_trait]
impl<N: NetcodeService, S: InputSerializer> CombatInputPort for CombatInteractor<N, S> {
    async fn start_combat(&self, request: StartCombatRequest) -> Result<StartCombatResponse> {
        let user1 = User::new(
            format!("player-{}", request.player1_id),
            request.player1_id.clone(),
        );
        let user2 = User::new(
            format!("player-{}", request.player2_id),
            request.player2_id.clone(),
        );

        let (fighter1, fighter2) = MatchService::initialize_match(&user1, &user2);

        *self.fighter1.lock().await = Some(fighter1);
        *self.fighter2.lock().await = Some(fighter2);

        let mut state = self.state.lock().await;
        state.phase = CombatPhase::Countdown;
        state.countdown_frames = 180;
        state.total_frames = request.round_time_seconds * 60;
        state.elapsed_frames = 0;

        // Netcodeをクリア
        self.netcode.lock().await.clear();

        Ok(StartCombatResponse {
            countdown_frames: state.countdown_frames,
            total_frames: state.total_frames,
            message: "Combat starting! 3... 2... 1... FIGHT!".to_string(),
        })
    }

    async fn update_frame(&self, request: CombatFrameRequest) -> Result<CombatFrameResponse> {
        let mut fighter1_guard = self.fighter1.lock().await;
        let mut fighter2_guard = self.fighter2.lock().await;
        let mut state = self.state.lock().await;
        let mut netcode = self.netcode.lock().await;

        let fighter1 = fighter1_guard.as_mut().ok_or_else(|| {
            ApplicationError::InvalidState("Fighter1 not initialized".to_string())
        })?;
        let fighter2 = fighter2_guard.as_mut().ok_or_else(|| {
            ApplicationError::InvalidState("Fighter2 not initialized".to_string())
        })?;

        // カウントダウン中または終了時は早期リターン
        match state.phase {
            CombatPhase::Countdown => {
                if state.countdown_frames > 0 {
                    state.countdown_frames -= 1;
                } else {
                    state.phase = CombatPhase::Fighting;
                }
                let phase = state.phase;
                let elapsed = state.elapsed_frames;
                let total = state.total_frames;
                return Ok(Self::create_response(
                    fighter1, fighter2, phase, elapsed, total,
                ));
            }
            CombatPhase::Finished => {
                let phase = state.phase;
                let elapsed = state.elapsed_frames;
                let total = state.total_frames;
                return Ok(Self::create_response(
                    fighter1, fighter2, phase, elapsed, total,
                ));
            }
            CombatPhase::Fighting => {}
        }

        // ロールバック・ネットコード処理
        let current_frame = netcode.current_frame();

        // ローカル入力をP2P経由で送信（先に送信してデッドロックを防ぐ）
        let local_input = if self.is_host {
            request.player1_input
        } else {
            request.player2_input
        };
        self.send_local_input(local_input, current_frame).await?;

        // P2P通信でリモート入力を受信し、確定入力をNetcodeに反映
        let remote_input = self.receive_remote_input(&mut netcode, current_frame).await;

        // 入力を追加（ローカル入力は確定、リモート入力は予測）
        netcode.add_input(local_input, remote_input);

        // 現在の状態を保存
        netcode.save_state(fighter1.clone(), fighter2.clone());

        // ロールバックが必要かチェック
        if let Some(rollback_frame) = netcode.get_rollback_frame() {
            // 予測が外れた！ロールバックして再シミュレート
            if let Some((f1, f2)) = netcode.get_state(rollback_frame) {
                let mut f1 = f1;
                let mut f2 = f2;
                // rollback_frameから現在のフレームまで再シミュレート
                for frame in rollback_frame..=current_frame {
                    if let Some((input1, input2_opt)) = netcode.get_input(frame) {
                        let input2 = input2_opt.unwrap_or_else(CombatInputKey::empty);
                        Self::simulate_frame(
                            &mut f1,
                            &mut f2,
                            input1,
                            input2,
                            &state.physics_constants,
                        );
                    }
                }

                // 再シミュレート結果を反映
                *fighter1 = f1;
                *fighter2 = f2;
            }
        } else {
            // 通常のフレーム更新
            let physics_constants = state.physics_constants;
            Self::simulate_frame(
                fighter1,
                fighter2,
                request.player1_input,
                request.player2_input,
                &physics_constants,
            );
        }

        state.elapsed_frames += 1;
        netcode.advance_frame();

        // 勝敗判定
        let outcome = MatchService::determine_outcome(fighter1, fighter2);
        if outcome.state == MatchState::Finished
            || outcome.state == MatchState::Draw
            || state.elapsed_frames >= state.total_frames
        {
            state.phase = CombatPhase::Finished;
            // 戦闘終了時にP2P接続を閉じる
            let _ = self.p2p.close().await;
        }

        let phase = state.phase;
        let elapsed = state.elapsed_frames;
        let total = state.total_frames;

        Ok(Self::create_response(
            fighter1, fighter2, phase, elapsed, total,
        ))
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::application::dto::request::{CombatFrameRequest, StartCombatRequest};
    use crate::domain::domain_services::MatchState;
    use crate::domain::services::netcode_service::{MockInputSerializer, MockNetcodeService};
    use crate::domain::services::p2p_service::MockP2PService;
    use crate::domain::value_objects::CombatInputKey;

    fn create_mock_netcode() -> MockNetcodeService {
        let mut mock = MockNetcodeService::new();
        mock.expect_clear().returning(|| ());
        mock
    }

    fn create_mock_p2p() -> MockP2PService {
        let mut mock = MockP2PService::new();
        mock.expect_is_connected().returning(|| false);
        mock.expect_close().returning(|| Ok(()));
        mock
    }

    fn create_combat_interactor(
        netcode: MockNetcodeService,
        p2p: MockP2PService,
        serializer: MockInputSerializer,
        is_host: bool,
    ) -> CombatInteractor<MockNetcodeService, MockInputSerializer> {
        CombatInteractor::new(netcode, Arc::new(p2p), Arc::new(serializer), is_host)
    }

    fn start_request() -> StartCombatRequest {
        StartCombatRequest {
            player1_id: "player-1-test-id".to_string(),
            player2_id: "player-2-test-id".to_string(),
            round_time_seconds: 99,
        }
    }

    fn empty_frame_request() -> CombatFrameRequest {
        CombatFrameRequest {
            player1_input: CombatInputKey::empty(),
            player2_input: CombatInputKey::empty(),
            frame_number: 0,
        }
    }

    #[tokio::test]
    async fn start_combat_正常系_初期化成功() {
        let netcode = create_mock_netcode();
        let p2p = create_mock_p2p();
        let serializer = MockInputSerializer::new();

        let interactor = create_combat_interactor(netcode, p2p, serializer, true);
        let result = interactor.start_combat(start_request()).await;

        assert!(result.is_ok());
        let resp = result.unwrap();
        assert_eq!(resp.countdown_frames, 180);
        assert_eq!(resp.total_frames, 99 * 60);
        assert!(resp.message.contains("FIGHT"));
    }

    #[tokio::test]
    async fn update_frame_カウントダウン中はPreparing() {
        let netcode = create_mock_netcode();
        let p2p = create_mock_p2p();
        let serializer = MockInputSerializer::new();

        let interactor = create_combat_interactor(netcode, p2p, serializer, true);
        interactor.start_combat(start_request()).await.unwrap();

        let resp = interactor
            .update_frame(empty_frame_request())
            .await
            .unwrap();
        assert_eq!(resp.match_state, MatchState::Preparing);
        // HP初期値
        assert_eq!(resp.player1_hp, 255);
        assert_eq!(resp.player2_hp, 255);
    }

    #[tokio::test]
    async fn update_frame_カウントダウン終了後にFighting開始() {
        let mut netcode = create_mock_netcode();
        // カウントダウン終了後、Fighting中に呼ばれるNetcodeメソッド群
        netcode.expect_current_frame().returning(|| 0);
        netcode.expect_add_input().returning(|_, _| ());
        netcode.expect_save_state().returning(|_, _| ());
        netcode.expect_get_rollback_frame().returning(|| None);
        netcode.expect_advance_frame().returning(|| ());

        let p2p = create_mock_p2p();
        let serializer = MockInputSerializer::new();

        let interactor = create_combat_interactor(netcode, p2p, serializer, true);
        interactor.start_combat(start_request()).await.unwrap();

        // カウントダウン消化（180回でcountdown_frames=0、181回目でFightingに遷移して早期リターン）
        for _ in 0..181 {
            interactor
                .update_frame(empty_frame_request())
                .await
                .unwrap();
        }

        // 182フレーム目: Fighting開始
        let resp = interactor
            .update_frame(empty_frame_request())
            .await
            .unwrap();
        assert_eq!(resp.match_state, MatchState::InProgress);
    }

    #[tokio::test]
    async fn update_frame_KOでFinished() {
        let mut netcode = create_mock_netcode();
        netcode.expect_current_frame().returning(|| 0);
        netcode.expect_add_input().returning(|_, _| ());
        netcode.expect_save_state().returning(|_, _| ());
        netcode.expect_get_rollback_frame().returning(|| None);
        netcode.expect_advance_frame().returning(|| ());

        let p2p = create_mock_p2p();
        let serializer = MockInputSerializer::new();

        let interactor = create_combat_interactor(netcode, p2p, serializer, true);
        interactor.start_combat(start_request()).await.unwrap();

        // カウントダウン消化（180回でcountdown_frames=0、181回目でFightingに遷移して早期リターン）
        for _ in 0..181 {
            interactor
                .update_frame(empty_frame_request())
                .await
                .unwrap();
        }

        // Fighter2のHPを0にする（直接操作）
        {
            let mut f2 = interactor.fighter2.lock().await;
            if let Some(ref mut fighter) = *f2 {
                fighter.take_damage(255);
            }
        }

        // 次のフレームでFinished判定
        let resp = interactor
            .update_frame(empty_frame_request())
            .await
            .unwrap();
        assert_eq!(resp.match_state, MatchState::Finished);
        assert_eq!(resp.winner, 1); // Player1の勝利
        assert_eq!(resp.player2_hp, 0);
    }

    #[tokio::test]
    async fn update_frame_パンチ入力で攻撃状態に遷移() {
        let mut netcode = create_mock_netcode();
        netcode.expect_current_frame().returning(|| 0);
        netcode.expect_add_input().returning(|_, _| ());
        netcode.expect_save_state().returning(|_, _| ());
        netcode.expect_get_rollback_frame().returning(|| None);
        netcode.expect_advance_frame().returning(|| ());

        let p2p = create_mock_p2p();
        let serializer = MockInputSerializer::new();

        let interactor = create_combat_interactor(netcode, p2p, serializer, true);
        interactor.start_combat(start_request()).await.unwrap();

        // カウントダウン消化（180回でcountdown_frames=0、181回目でFightingに遷移して早期リターン）
        for _ in 0..181 {
            interactor
                .update_frame(empty_frame_request())
                .await
                .unwrap();
        }

        // パンチ入力（182フレーム目: 最初の実戦フレーム）
        let mut punch_input = CombatInputKey::empty();
        punch_input.press(CombatInputKey::PUNCH);
        let request = CombatFrameRequest {
            player1_input: punch_input,
            player2_input: CombatInputKey::empty(),
            frame_number: 0,
        };

        let resp = interactor.update_frame(request).await.unwrap();
        assert!(resp.player1_state.contains("Attacking"));
    }

    #[tokio::test]
    async fn update_frame_未初期化でエラー() {
        let netcode = create_mock_netcode();
        let p2p = create_mock_p2p();
        let serializer = MockInputSerializer::new();

        let interactor = create_combat_interactor(netcode, p2p, serializer, true);
        // start_combatを呼ばずにupdate_frame
        let result = interactor.update_frame(empty_frame_request()).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not initialized"));
    }
}
