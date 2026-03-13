use crate::application::dto::request::{JoinRingRequest, WaitForMatchRequest};
use crate::application::dto::response::JoinRingResponse;
use crate::application::errors::ApplicationError;
use crate::application::ports::input::JoinRingInputPort;
use crate::application::ports::output::{ErrorLayer, JoinRingOutputPort};
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

pub struct JoinRingInteractor<R: RingRepository, O: JoinRingOutputPort> {
    ring_repository: R,
    output_port: O,
    /// マッチング成功後に戦闘で使用するP2P接続
    p2p_connection: Mutex<Option<Arc<dyn P2PService>>>,
}

impl<R: RingRepository, O: JoinRingOutputPort> JoinRingInteractor<R, O> {
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
impl<R: RingRepository, O: JoinRingOutputPort> JoinRingInputPort for JoinRingInteractor<R, O> {
    async fn join_ring(&self, request: JoinRingRequest) -> Result<()> {
        // 1. トークンからRing検索
        self.output_port
            .notify_progress("Searching for ring...")
            .await?;

        let token = Token::new(request.token).map_err(ApplicationError::Domain)?;

        let ring_opt = self
            .ring_repository
            .find_by_token(&token)
            .await
            .map_err(|e| ApplicationError::Repository(format!("Failed to find ring: {}", e)))?;

        let Some(mut ring) = ring_opt else {
            let error = ApplicationError::RingNotFound(token.value().to_string());
            return Err(self
                .handle_error(ErrorLayer::Application, error)
                .await
                .into());
        };

        // 2. Ed25519キーペア生成（ゲスト側）
        self.output_port
            .notify_progress("Generating Ed25519 keypair...")
            .await?;

        let (_private_key, public_key) = Ed25519Signer::generate_keypair().map_err(|e| {
            ApplicationError::RingCreationFailed(format!("Keypair generation failed: {}", e))
        })?;

        let guest_pubkey = PublicKey::new(public_key).map_err(ApplicationError::Domain)?;

        // 3. WebRTC Answer SDP生成
        self.output_port
            .notify_progress("Generating WebRTC Answer SDP...")
            .await?;

        let (sdp_string, guest_connection) =
            SdpGenerator::create_guest_answer(ring.host_sdp().value())
                .await
                .map_err(|e| {
                    ApplicationError::RingCreationFailed(format!("SDP generation failed: {}", e))
                })?;

        // P2P接続を保持（マッチング成功後にCombatInteractorへDIする）
        *self.p2p_connection.lock().await = Some(Arc::new(guest_connection));

        let guest_sdp = Sdp::new(sdp_string).map_err(ApplicationError::Domain)?;

        // 4. Ringにゲスト情報を追加
        self.output_port
            .notify_progress("Updating ring with answer...")
            .await?;
        ring.with_guest_answer(guest_sdp);
        ring.with_guest_pubkey(guest_pubkey);

        // 5. 更新
        self.output_port
            .notify_progress(&format!(
                "Updating ring {} with guest_sdp and guest_pubkey...",
                ring.id()
            ))
            .await?;
        self.ring_repository
            .update(&ring)
            .await
            .map_err(|e| ApplicationError::Repository(format!("Failed to update ring: {}", e)))?;
        self.output_port
            .notify_progress(&format!(
                "Ring {} updated successfully in database",
                ring.id()
            ))
            .await?;

        // 6. レスポンスDTO作成してプレゼンターに通知
        let response = JoinRingResponse {
            ring_id: *ring.id(),
            host_sdp: ring.host_sdp().value().to_string(),
        };

        // 7. プレゼンターに結果を通知（非同期的に表示される）
        self.output_port.present(response).await?;

        Ok(())
    }

    async fn wait_for_match(&self, request: WaitForMatchRequest) -> Result<()> {
        self.output_port
            .notify_progress(&format!(
                "Establishing WebRTC connection (ring_id: {})...",
                request.ring_id
            ))
            .await?;

        // リングの状態を確認
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

        self.output_port
            .notify_progress(&format!(
                "After update: ring_id={}, status={:?}, guest_sdp={}, guest_pubkey={}",
                request.ring_id,
                ring.status(),
                ring.guest_sdp().is_some(),
                ring.guest_pubkey().is_some()
            ))
            .await?;

        // ゲスト側: DataChannelが開通するまで待機
        let p2p_guard = self.p2p_connection.lock().await;
        let Some(p2p) = p2p_guard.as_ref() else {
            let error =
                ApplicationError::InvalidState("P2P connection not established".to_string());
            return Err(self
                .handle_error(ErrorLayer::Application, error)
                .await
                .into());
        };

        self.output_port
            .notify_progress("Waiting for DataChannel to open...")
            .await?;
        p2p.wait_until_connected(30).await.map_err(|e| {
            let hint =
                firewall::connection_error_with_hint(&format!("WebRTC connection failed: {}", e));
            ApplicationError::RingCreationFailed(hint)
        })?;
        drop(p2p_guard);

        self.output_port.notify_progress("MATCHED").await?;
        self.output_port.notify_progress("MATCH_SUCCESS").await?;

        Ok(())
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::application::dto::request::WaitForMatchRequest;
    use crate::application::ports::output::join_ring_output_port::MockJoinRingOutputPort;
    use crate::domain::entities::{Ring, RingStatus};
    use crate::domain::repositories::ring_repository::MockRingRepository;
    use crate::domain::services::p2p_service::MockP2PService;
    use crate::domain::value_objects::{PublicKey, Sdp, Token};
    use chrono::{Duration, Utc};
    use mockall::predicate::*;
    use uuid::Uuid;

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
    async fn wait_for_match_正常系_接続成功() {
        let ring_id = Uuid::new_v4();
        let ring = make_open_ring(ring_id);

        let mut mock_repo = MockRingRepository::new();
        mock_repo
            .expect_find_by_id()
            .with(eq(ring_id))
            .times(1)
            .returning(move |_| Ok(Some(ring.clone())));

        let mut mock_output = MockJoinRingOutputPort::new();
        mock_output.expect_notify_progress().returning(|_| Ok(()));

        let interactor = JoinRingInteractor::new(mock_repo, mock_output);

        // P2P接続をセット
        let mut mock_p2p = MockP2PService::new();
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

        let mut mock_output = MockJoinRingOutputPort::new();
        mock_output.expect_notify_progress().returning(|_| Ok(()));
        mock_output
            .expect_notify_error()
            .times(1)
            .withf(|layer: &ErrorLayer, msg: &str| {
                matches!(layer, ErrorLayer::Application) && msg.contains("Ring not found")
            })
            .returning(|_, _| Ok(()));

        let interactor = JoinRingInteractor::new(mock_repo, mock_output);
        let request = WaitForMatchRequest { ring_id };
        let result = interactor.wait_for_match(request).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Ring not found"));
    }

    #[tokio::test]
    async fn wait_for_match_異常系_WebRTC接続タイムアウト() {
        let ring_id = Uuid::new_v4();
        let ring = make_open_ring(ring_id);

        let mut mock_repo = MockRingRepository::new();
        mock_repo
            .expect_find_by_id()
            .with(eq(ring_id))
            .times(1)
            .returning(move |_| Ok(Some(ring.clone())));

        let mut mock_output = MockJoinRingOutputPort::new();
        mock_output.expect_notify_progress().returning(|_| Ok(()));

        let interactor = JoinRingInteractor::new(mock_repo, mock_output);

        let mut mock_p2p = MockP2PService::new();
        mock_p2p
            .expect_wait_until_connected()
            .with(eq(30u64))
            .times(1)
            .returning(|_| Err(anyhow::anyhow!("Connection timed out")));

        *interactor.p2p_connection.lock().await = Some(Arc::new(mock_p2p));

        let request = WaitForMatchRequest { ring_id };
        let result = interactor.wait_for_match(request).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("WebRTC connection failed"));
    }

    #[tokio::test]
    async fn take_p2p_connection_取り出し後はNone() {
        let mock_repo = MockRingRepository::new();
        let mock_output = MockJoinRingOutputPort::new();
        let interactor = JoinRingInteractor::new(mock_repo, mock_output);

        let mut mock_p2p = MockP2PService::new();
        mock_p2p.expect_close().returning(|| Ok(()));
        *interactor.p2p_connection.lock().await = Some(Arc::new(mock_p2p));

        let conn = interactor.take_p2p_connection().await;
        assert!(conn.is_some());

        let conn = interactor.take_p2p_connection().await;
        assert!(conn.is_none());
    }
}
