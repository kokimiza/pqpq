use anyhow::{Result, anyhow};
use bytes::Bytes;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, Notify};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::ice_transport::ice_gatherer_state::RTCIceGathererState;
use webrtc::ice_transport::ice_gathering_state::RTCIceGatheringState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;

use crate::domain::services::P2PService;

/// P2P接続マネージャー
///
/// WebRTC DataChannelを使用して、格闘ゲームの入力データをピア間で低遅延送受信する。
///
/// ## 設計方針
/// - v0.17.x のコールバックAPIを使用（v0.20.0-alpha.1 Sans-I/O版は安定後に移行予定）
/// - ICE gathering完了を `Notify` で正確に待機（固定sleep廃止）
/// - DataChannel の `on_open` を監視して接続確立を検知
/// - 受信バッファは `Vec<Vec<u8>>` で管理し、毎フレーム drain
/// - v0.17.x既知のメモリリーク（接続あたり約109KiB）は許容
///   （格ゲーは1対1の短時間接続のため影響軽微）
pub struct P2PConnection {
    peer_connection: Arc<RTCPeerConnection>,
    data_channel: Arc<Mutex<Option<Arc<RTCDataChannel>>>>,
    received_data: Arc<Mutex<Vec<Vec<u8>>>>,
    dc_open: Arc<AtomicBool>,
}

impl P2PConnection {
    /// WebRTC APIとPeerConnectionを構築する共通処理
    async fn build_peer_connection() -> Result<Arc<RTCPeerConnection>> {
        let mut media_engine = MediaEngine::default();
        let registry = webrtc::interceptor::registry::Registry::new();
        let registry = register_default_interceptors(registry, &mut media_engine)?;

        let api = APIBuilder::new()
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        let config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec![
                    "stun:stun.l.google.com:19302".to_owned(),
                    "stun:stun1.l.google.com:19302".to_owned(),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        Ok(Arc::new(api.new_peer_connection(config).await?))
    }

    /// 受信ハンドラーをDataChannelに設定
    fn attach_message_handler(dc: &Arc<RTCDataChannel>, buffer: Arc<Mutex<Vec<Vec<u8>>>>) {
        dc.on_message(Box::new(move |msg: DataChannelMessage| {
            let buffer = Arc::clone(&buffer);
            Box::pin(async move {
                buffer.lock().await.push(msg.data.to_vec());
            })
        }));
    }

    /// DataChannelの `on_open` を監視するハンドラーを設定
    fn attach_open_handler(dc: &Arc<RTCDataChannel>, flag: Arc<AtomicBool>) {
        dc.on_open(Box::new(move || {
            flag.store(true, Ordering::Release);
            Box::pin(async {})
        }));
    }

    /// ICE gathering完了を待機（Notifyベース、タイムアウト付き）
    async fn wait_for_ice_gathering(pc: &Arc<RTCPeerConnection>) -> Result<String> {
        let gather_done = Arc::new(Notify::new());
        let gather_done_clone = Arc::clone(&gather_done);

        pc.on_ice_gathering_state_change(Box::new(move |state: RTCIceGathererState| {
            if state == RTCIceGathererState::Complete {
                gather_done_clone.notify_one();
            }
            Box::pin(async {})
        }));

        // 既にCompleteの場合はスキップ
        if pc.ice_gathering_state() != RTCIceGatheringState::Complete {
            tokio::select! {
                _ = gather_done.notified() => {}
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(5)) => {
                    // タイムアウトしても現在のSDPで続行（候補が不完全な可能性あり）
                }
            }
        }

        pc.local_description()
            .await
            .map(|desc| desc.sdp)
            .ok_or_else(|| anyhow!("Failed to get local description after ICE gathering"))
    }

    /// 新しいP2P接続を作成（ホスト側 = Offer側）
    pub async fn new_host() -> Result<Self> {
        let peer_connection = Self::build_peer_connection().await?;
        let data_channel: Arc<Mutex<Option<Arc<RTCDataChannel>>>> = Arc::new(Mutex::new(None));
        let received_data = Arc::new(Mutex::new(Vec::new()));
        let dc_open = Arc::new(AtomicBool::new(false));

        // DataChannelを作成（ホスト側が作成者）
        let dc = peer_connection.create_data_channel("input", None).await?;
        *data_channel.lock().await = Some(dc.clone());

        Self::attach_message_handler(&dc, Arc::clone(&received_data));
        Self::attach_open_handler(&dc, Arc::clone(&dc_open));

        Ok(Self {
            peer_connection,
            data_channel,
            received_data,
            dc_open,
        })
    }

    /// 新しいP2P接続を作成（ゲスト側 = Answer側）
    pub async fn new_guest() -> Result<Self> {
        let peer_connection = Self::build_peer_connection().await?;
        let data_channel: Arc<Mutex<Option<Arc<RTCDataChannel>>>> = Arc::new(Mutex::new(None));
        let received_data = Arc::new(Mutex::new(Vec::new()));
        let dc_open = Arc::new(AtomicBool::new(false));

        // ゲスト側はホストからDataChannelが届くのを待つ
        let dc_slot = Arc::clone(&data_channel);
        let recv_buf = Arc::clone(&received_data);
        let open_flag = Arc::clone(&dc_open);

        peer_connection.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
            let dc_slot = Arc::clone(&dc_slot);
            let recv_buf = Arc::clone(&recv_buf);
            let open_flag = Arc::clone(&open_flag);
            Box::pin(async move {
                *dc_slot.lock().await = Some(dc.clone());
                Self::attach_message_handler(&dc, Arc::clone(&recv_buf));
                Self::attach_open_handler(&dc, open_flag);
            })
        }));

        Ok(Self {
            peer_connection,
            data_channel,
            received_data,
            dc_open,
        })
    }

    /// Offer SDPを作成（ICE gathering完了を正確に待機）
    pub async fn create_offer(&self) -> Result<String> {
        let offer = self.peer_connection.create_offer(None).await?;
        self.peer_connection.set_local_description(offer).await?;

        Self::wait_for_ice_gathering(&self.peer_connection).await
    }

    /// リモートのOffer SDPを受け取り、Answer SDPを作成
    pub async fn create_answer(&self, offer_sdp: &str) -> Result<String> {
        let offer = RTCSessionDescription::offer(offer_sdp.to_string())?;
        self.peer_connection.set_remote_description(offer).await?;

        let answer = self.peer_connection.create_answer(None).await?;
        self.peer_connection.set_local_description(answer).await?;

        Self::wait_for_ice_gathering(&self.peer_connection).await
    }

    /// リモートのAnswer SDPを設定（ホスト側が呼ぶ）
    pub async fn set_answer(&self, answer_sdp: &str) -> Result<()> {
        let answer = RTCSessionDescription::answer(answer_sdp.to_string())?;
        self.peer_connection.set_remote_description(answer).await?;
        Ok(())
    }

    /// DataChannelが開通するまで待機（タイムアウト付き）
    ///
    /// dc_openフラグだけでなく、PeerConnectionがConnectedであることも確認する。
    pub async fn wait_for_data_channel_open(&self, timeout_secs: u64) -> Result<()> {
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);

        while !self.dc_open.load(Ordering::Acquire)
            || !matches!(self.connection_state(), RTCPeerConnectionState::Connected)
        {
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!(
                    "DataChannel did not open within {} seconds (dc_open={}, peer_state={:?})",
                    timeout_secs,
                    self.dc_open.load(Ordering::Acquire),
                    self.connection_state()
                ));
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }

        Ok(())
    }

    /// PeerConnectionの接続状態を取得
    pub fn connection_state(&self) -> RTCPeerConnectionState {
        self.peer_connection.connection_state()
    }

    /// DataChannelが開通しているか
    pub fn is_data_channel_open(&self) -> bool {
        self.dc_open.load(Ordering::Acquire)
    }
}

#[async_trait::async_trait]
impl P2PService for P2PConnection {
    async fn send(&self, data: &[u8]) -> Result<()> {
        let dc_guard = self.data_channel.lock().await;
        let dc = dc_guard
            .as_ref()
            .ok_or_else(|| anyhow!("DataChannel not ready"))?;

        let bytes = Bytes::copy_from_slice(data);
        dc.send(&bytes).await?;
        Ok(())
    }

    async fn receive_all(&self) -> Vec<Vec<u8>> {
        let mut buffer = self.received_data.lock().await;
        std::mem::take(&mut *buffer)
    }

    async fn is_connected(&self) -> bool {
        self.is_data_channel_open()
            && matches!(self.connection_state(), RTCPeerConnectionState::Connected)
    }

    async fn set_remote_answer(&self, answer_sdp: &str) -> Result<()> {
        self.set_answer(answer_sdp).await
    }

    async fn wait_until_connected(&self, timeout_secs: u64) -> Result<()> {
        self.wait_for_data_channel_open(timeout_secs).await
    }

    async fn close(&self) -> Result<()> {
        self.peer_connection.close().await?;
        Ok(())
    }
}
