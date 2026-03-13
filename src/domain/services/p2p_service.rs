use anyhow::Result;

/// P2P通信サービス
///
/// WebRTC DataChannelを使用した入力データの送受信
#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait P2PService: Send + Sync {
    /// データを送信
    async fn send(&self, data: &[u8]) -> Result<()>;

    /// 受信したデータを取得（すべて取得してクリア）
    async fn receive_all(&self) -> Vec<Vec<u8>>;

    /// 接続が確立されているか
    async fn is_connected(&self) -> bool;

    /// リモートのAnswer SDPを設定（ホスト側がマッチング成功後に呼ぶ）
    async fn set_remote_answer(&self, answer_sdp: &str) -> Result<()>;

    /// DataChannelが開通するまで待機（タイムアウト付き）
    async fn wait_until_connected(&self, timeout_secs: u64) -> Result<()>;

    /// 接続を閉じる
    async fn close(&self) -> Result<()>;
}
