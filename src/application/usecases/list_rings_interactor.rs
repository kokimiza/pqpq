use crate::application::dto::request::ListRingsRequest;
use crate::application::dto::response::{ListRingsResponse, RingInfo};
use crate::application::errors::ApplicationError;
use crate::application::ports::input::ListRingsInputPort;
use crate::application::ports::output::{ErrorLayer, ListRingsOutputPort};
use crate::domain::repositories::RingRepository;
use anyhow::Result;
use async_trait::async_trait;

pub struct ListRingsInteractor<R: RingRepository, O: ListRingsOutputPort> {
    ring_repository: R,
    output_port: O,
}

impl<R: RingRepository, O: ListRingsOutputPort> ListRingsInteractor<R, O> {
    pub fn new(ring_repository: R, output_port: O) -> Self {
        Self {
            ring_repository,
            output_port,
        }
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
impl<R: RingRepository, O: ListRingsOutputPort> ListRingsInputPort for ListRingsInteractor<R, O> {
    async fn execute(&self, _request: ListRingsRequest) -> Result<()> {
        // オープンなRing一覧取得
        let rings = match self.ring_repository.find_open_rings().await {
            Ok(rings) => rings,
            Err(e) => {
                let error =
                    ApplicationError::Repository(format!("Failed to find open rings: {}", e));
                return Err(self
                    .handle_error(ErrorLayer::Infrastructure, error)
                    .await
                    .into());
            }
        };

        // レスポンスDTO作成
        let ring_infos: Vec<RingInfo> = rings
            .iter()
            .map(|ring| RingInfo {
                ring_id: *ring.id(),
                token: ring.token().value().to_string(),
                created_at: ring.created_at().to_rfc3339(),
            })
            .collect();

        let response = ListRingsResponse { rings: ring_infos };

        // Output Portへ渡す
        self.output_port.present(response).await?;

        Ok(())
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::application::ports::output::list_rings_output_port::MockListRingsOutputPort;
    use crate::domain::entities::Ring;
    use crate::domain::repositories::ring_repository::MockRingRepository;
    use crate::domain::value_objects::{PublicKey, Sdp, Token};
    use mockall::predicate::*;

    fn make_test_ring(token_str: &str) -> Ring {
        let token = Token::new(token_str.to_string()).unwrap();
        let sdp = Sdp::new("v=0\r\ntest-sdp".to_string()).unwrap();
        let pubkey = PublicKey::from_bytes(&[1u8; 32]);
        Ring::new(token, sdp, pubkey)
    }

    #[tokio::test]
    async fn execute_正常系_リング一覧を返す() {
        let ring1 = make_test_ring("TOKEN001");
        let ring2 = make_test_ring("TOKEN002");
        let rings = vec![ring1.clone(), ring2.clone()];

        let mut mock_repo = MockRingRepository::new();
        mock_repo
            .expect_find_open_rings()
            .times(1)
            .returning(move || Ok(rings.clone()));

        let mut mock_output = MockListRingsOutputPort::new();
        mock_output
            .expect_present()
            .times(1)
            .withf(|resp: &ListRingsResponse| resp.rings.len() == 2)
            .returning(|_| Ok(()));

        let interactor = ListRingsInteractor::new(mock_repo, mock_output);
        let result = interactor.execute(ListRingsRequest).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn execute_正常系_空のリング一覧() {
        let mut mock_repo = MockRingRepository::new();
        mock_repo
            .expect_find_open_rings()
            .times(1)
            .returning(|| Ok(vec![]));

        let mut mock_output = MockListRingsOutputPort::new();
        mock_output
            .expect_present()
            .times(1)
            .withf(|resp: &ListRingsResponse| resp.rings.is_empty())
            .returning(|_| Ok(()));

        let interactor = ListRingsInteractor::new(mock_repo, mock_output);
        let result = interactor.execute(ListRingsRequest).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn execute_異常系_リポジトリエラーでnotify_error呼出() {
        let mut mock_repo = MockRingRepository::new();
        mock_repo
            .expect_find_open_rings()
            .times(1)
            .returning(|| Err(anyhow::anyhow!("DB connection failed")));

        let mut mock_output = MockListRingsOutputPort::new();
        mock_output
            .expect_notify_error()
            .times(1)
            .withf(|layer: &ErrorLayer, msg: &str| {
                matches!(layer, ErrorLayer::Infrastructure)
                    && msg.contains("Failed to find open rings")
            })
            .returning(|_, _| Ok(()));
        // presentは呼ばれない
        mock_output.expect_present().never();

        let interactor = ListRingsInteractor::new(mock_repo, mock_output);
        let result = interactor.execute(ListRingsRequest).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Failed to find open rings"));
    }
}
