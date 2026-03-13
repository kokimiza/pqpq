use crate::application::dto::request::{HostRingRequest, WaitForMatchRequest};
use crate::application::dto::response::HostRingResponse;
use crate::application::errors::ApplicationError;
use crate::application::ports::input::HostRingInputPort;
use crate::application::ports::output::{ErrorLayer, HostRingOutputPort};
use crate::domain::entities::Ring;
use crate::domain::repositories::RingRepository;
use crate::domain::services::P2PService;
use crate::domain::value_objects::{PublicKey, Sdp, Token};
use crate::infrastructure::crypto::Ed25519Signer;
use crate::infrastructure::firewall;
use crate::infrastructure::webrtc::SdpGenerator;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep};

pub struct HostRingInteractor<R: RingRepository, O: HostRingOutputPort> {
    ring_repository: R,
    output_port: O,
    /// マッチング成功後に戦闘で使用するP2P接続
    p2p_connection: Mutex<Option<Arc<dyn P2PService>>>,
}

impl<R: RingRepository, O: HostRingOutputPort> HostRingInteractor<R, O> {
    pub fn new(ring_repository: R, output_port: O) -> Self {
        Self {
            ring_repository,
            output_port,
            p2p_connection: Mutex::new(None),
        }
    }

    /// 戦闘用のP2P接続を取り出す（マッチング成功後に呼ぶ）
    pub async fn take_p2p_connection(&self) -> Option<Arc<dyn P2PService>> {
        self.p2p_connection.lock().await.take()
    }

    /// エラーを通知して返す（Rust 2024 Edition対応）
    async fn handle_error(&self, layer: ErrorLayer, error: ApplicationError) -> ApplicationError {
        let _ = self
            .output_port
            .notify_error(layer, &error.to_string())
            .await;
        error
    }
}

#[async_trait]
impl<R: RingRepository, O: HostRingOutputPort> HostRingInputPort for HostRingInteractor<R, O> {
    async fn create_ring(&self, _request: HostRingRequest) -> Result<()> {
        // 1. トークン生成
        self.output_port
            .notify_progress("Generating invitation token...")
            .await?;
        let token = Token::generate();

        // 2. Ed25519キーペア生成
        self.output_port
            .notify_progress("Generating Ed25519 keypair...")
            .await?;

        let (_private_key, public_key) = Ed25519Signer::generate_keypair().map_err(|e| {
            ApplicationError::RingCreationFailed(format!("Keypair generation failed: {}", e))
        })?;

        let host_pubkey = PublicKey::new(public_key).map_err(ApplicationError::Domain)?;

        // 3. WebRTC Offer SDP生成
        self.output_port
            .notify_progress("Generating WebRTC Offer SDP...")
            .await?;

        let (sdp_string, host_connection) =
            SdpGenerator::create_host_offer().await.map_err(|e| {
                ApplicationError::RingCreationFailed(format!("SDP generation failed: {}", e))
            })?;

        // P2P接続を保持（マッチング成功後にCombatInteractorへDIする）
        *self.p2p_connection.lock().await = Some(Arc::new(host_connection));

        let host_sdp = Sdp::new(sdp_string).map_err(ApplicationError::Domain)?;

        // 4. Ringエンティティ組み立て
        self.output_port.notify_progress("Creating ring...").await?;
        let ring = Ring::new(token.clone(), host_sdp, host_pubkey.clone());

        // 5. リポジトリで永続化
        self.output_port
            .notify_progress("Saving to database...")
            .await?;

        self.ring_repository
            .save(&ring)
            .await
            .map_err(|e| ApplicationError::Repository(format!("Failed to save ring: {}", e)))?;

        // 6. レスポンスDTO作成してプレゼンターに通知
        let response = HostRingResponse {
            ring_id: *ring.id(),
            token: token.value().to_string(),
            host_pubkey: host_pubkey.value().to_string(),
        };

        // 7. プレゼンターに結果を通知（非同期的に表示される）
        self.output_port.present(response).await?;

        Ok(())
    }

    async fn wait_for_match(&self, request: WaitForMatchRequest) -> Result<()> {
        self.output_port
            .notify_progress(&format!(
                "Waiting for opponent (ring_id: {})...",
                request.ring_id
            ))
            .await?;

        // ポーリングでゲストの参加を待つ
        let max_attempts = 300; // 5分間（1秒ごとに300回）
        for attempt in 0..max_attempts {
            // リングの状態を取得
            let ring_opt = self
                .ring_repository
                .find_by_id(&request.ring_id)
                .await
                .map_err(|e| ApplicationError::Repository(format!("Failed to find ring: {}", e)))?;

            let Some(ring) = ring_opt else {
                let error = ApplicationError::RingNotFound(request.ring_id.to_string());
                return Err(self
                    .handle_error(ErrorLayer::Application, error)
                    .await
                    .into());
            };

            // ゲストが参加したかチェック
            let has_guest = ring.guest_sdp().is_some();
            let has_guest_pubkey = ring.guest_pubkey().is_some();
            let status = format!("{:?}", ring.status());
            if attempt % 5 == 0 {
                self.output_port
                    .notify_progress(&format!(
                        "Polling #{}: ring_id={}, status={}, guest_sdp={}, guest_pubkey={}",
                        attempt, request.ring_id, status, has_guest, has_guest_pubkey
                    ))
                    .await?;
            }
            if has_guest {
                self.output_port
                    .notify_progress(&format!(
                        "Guest detected! guest_sdp_len={}, guest_pubkey={}",
                        ring.guest_sdp().map(|s| s.value().len()).unwrap_or(0),
                        has_guest_pubkey
                    ))
                    .await?;
                let guest_sdp = ring.guest_sdp().unwrap();
                self.output_port.notify_progress("MATCHED").await?;

                // ホスト側: ゲストのAnswer SDPを設定してWebRTC接続を完了
                let p2p_guard = self.p2p_connection.lock().await;
                let Some(p2p) = p2p_guard.as_ref() else {
                    let error = ApplicationError::InvalidState(
                        "P2P connection not established".to_string(),
                    );
                    return Err(self
                        .handle_error(ErrorLayer::Application, error)
                        .await
                        .into());
                };

                self.output_port
                    .notify_progress("Setting remote answer SDP...")
                    .await?;
                p2p.set_remote_answer(guest_sdp.value())
                    .await
                    .map_err(|e| {
                        ApplicationError::RingCreationFailed(format!(
                            "Failed to set answer SDP: {}",
                            e
                        ))
                    })?;

                self.output_port
                    .notify_progress("Waiting for DataChannel to open...")
                    .await?;
                p2p.wait_until_connected(30).await.map_err(|e| {
                    let hint = firewall::connection_error_with_hint(&format!(
                        "WebRTC connection failed: {}",
                        e
                    ));
                    ApplicationError::RingCreationFailed(hint)
                })?;
                drop(p2p_guard);

                // プレゼンターに結果を通知
                self.output_port.notify_progress("MATCH_SUCCESS").await?;
                return Ok(());
            }

            // 進捗表示（30秒ごと）
            if attempt % 30 == 0 && attempt > 0 {
                self.output_port
                    .notify_progress(&format!("Still waiting... ({}s)", attempt))
                    .await?;
            }

            sleep(Duration::from_secs(1)).await;
        }

        // タイムアウト
        let error = ApplicationError::RingCreationFailed("Match timeout".to_string());
        Err(self
            .handle_error(ErrorLayer::Application, error)
            .await
            .into())
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::application::dto::request::WaitForMatchRequest;
    use crate::application::ports::output::host_ring_output_port::MockHostRingOutputPort;
    use crate::domain::entities::{Ring, RingStatus};
    use crate::domain::repositories::ring_repository::MockRingRepository;
    use crate::domain::services::p2p_service::MockP2PService;
    use crate::domain::value_objects::{PublicKey, Sdp, Token};
    use chrono::{Duration, Utc};
    use mockall::predicate::*;
    use uuid::Uuid;

    fn make_matched_ring(ring_id: Uuid) -> Ring {
        let now = Utc::now();
        Ring::reconstruct(
            ring_id,
            Token::new("TESTTOKEN".to_string()).unwrap(),
            Sdp::new("v=0\r\nhost-offer-sdp".to_string()).unwrap(),
            Some(Sdp::new("v=0\r\nguest-answer-sdp".to_string()).unwrap()),
            PublicKey::from_bytes(&[1u8; 32]),
            None,
            RingStatus::Matched,
            now,
            now + Duration::minutes(15),
        )
    }

    fn make_open_ring(ring_id: Uuid) -> Ring {
        let now = Utc::now();
        Ring::reconstruct(
            ring_id,
            Token::new("TESTTOKEN".to_string()).unwrap(),
            Sdp::new("v=0\r\nhost-offer-sdp".to_string()).unwrap(),
            None,
            PublicKey::from_bytes(&[1u8; 32]),
            None,
            RingStatus::Open,
            now,
            now + Duration::minutes(15),
        )
    }

    #[tokio::test]
    async fn wait_for_match_正常系_ゲスト参加済みで即マッチ() {
        let ring_id = Uuid::new_v4();
        let matched_ring = make_matched_ring(ring_id);

        let mut mock_repo = MockRingRepository::new();
        mock_repo
            .expect_find_by_id()
            .with(eq(ring_id))
            .times(1)
            .returning(move |_| Ok(Some(matched_ring.clone())));

        let mut mock_output = MockHostRingOutputPort::new();
        // notify_progressは複数回呼ばれる（Waiting... → MATCHED → Setting... → Waiting for DC... → MATCH_SUCCESS）
        mock_output.expect_notify_progress().returning(|_| Ok(()));

        let interactor = HostRingInteractor::new(mock_repo, mock_output);

        // P2P接続をセット（ホスト側はcreate_ringで保持するが、テストでは手動セット）
        let mut mock_p2p = MockP2PService::new();
        mock_p2p
            .expect_set_remote_answer()
            .with(eq("v=0\r\nguest-answer-sdp"))
            .times(1)
            .returning(|_| Ok(()));
        mock_p2p
            .expect_wait_until_connected()
            .with(eq(30u64))
            .times(1)
            .returning(|_| Ok(()));

        *interactor.p2p_connection.lock().await = Some(Arc::new(mock_p2p));

        let request = WaitForMatchRequest { ring_id };
        let result = interactor.wait_for_match(request).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wait_for_match_異常系_リング見つからない() {
        let ring_id = Uuid::new_v4();

        let mut mock_repo = MockRingRepository::new();
        mock_repo
            .expect_find_by_id()
            .with(eq(ring_id))
            .times(1)
            .returning(|_| Ok(None));

        let mut mock_output = MockHostRingOutputPort::new();
        mock_output.expect_notify_progress().returning(|_| Ok(()));
        mock_output
            .expect_notify_error()
            .times(1)
            .withf(|layer: &ErrorLayer, msg: &str| {
                matches!(layer, ErrorLayer::Application) && msg.contains("Ring not found")
            })
            .returning(|_, _| Ok(()));

        let interactor = HostRingInteractor::new(mock_repo, mock_output);
        let request = WaitForMatchRequest { ring_id };
        let result = interactor.wait_for_match(request).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Ring not found"));
    }

    #[tokio::test]
    async fn wait_for_match_正常系_2回目のポーリングでマッチ() {
        let ring_id = Uuid::new_v4();
        let open_ring = make_open_ring(ring_id);
        let matched_ring = make_matched_ring(ring_id);

        let mut mock_repo = MockRingRepository::new();
        let mut call_count = 0u32;
        mock_repo
            .expect_find_by_id()
            .with(eq(ring_id))
            .times(2)
            .returning(move |_| {
                call_count += 1;
                if call_count == 1 {
                    Ok(Some(open_ring.clone()))
                } else {
                    Ok(Some(matched_ring.clone()))
                }
            });

        let mut mock_output = MockHostRingOutputPort::new();
        mock_output.expect_notify_progress().returning(|_| Ok(()));

        let interactor = HostRingInteractor::new(mock_repo, mock_output);

        let mut mock_p2p = MockP2PService::new();
        mock_p2p
            .expect_set_remote_answer()
            .times(1)
            .returning(|_| Ok(()));
        mock_p2p
            .expect_wait_until_connected()
            .times(1)
            .returning(|_| Ok(()));

        *interactor.p2p_connection.lock().await = Some(Arc::new(mock_p2p));

        let request = WaitForMatchRequest { ring_id };
        let result = interactor.wait_for_match(request).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn take_p2p_connection_取り出し後はNone() {
        let mock_repo = MockRingRepository::new();
        let mock_output = MockHostRingOutputPort::new();
        let interactor = HostRingInteractor::new(mock_repo, mock_output);

        let mut mock_p2p = MockP2PService::new();
        mock_p2p.expect_close().returning(|| Ok(()));
        *interactor.p2p_connection.lock().await = Some(Arc::new(mock_p2p));

        // 1回目: 取り出せる
        let conn = interactor.take_p2p_connection().await;
        assert!(conn.is_some());

        // 2回目: Noneになる
        let conn = interactor.take_p2p_connection().await;
        assert!(conn.is_none());
    }
}
