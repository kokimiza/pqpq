use anyhow::Result;

use super::P2PConnection;

/// SDP生成 & P2P接続ファクトリ
///
/// WebRTC Offer/Answer SDPを生成すると同時に、
/// 戦闘で使用するP2PConnectionインスタンスを返す。
/// 呼び出し元はSDPをシグナリング経由で交換し、
/// 返却されたP2PConnectionをCombatInteractorにDIする。
pub struct SdpGenerator;

impl SdpGenerator {
    /// ホスト側: Offer SDPを生成し、P2PConnectionを返す
    ///
    /// 戻り値の`P2PConnection`はAnswer SDP設定後にそのまま戦闘で使用可能。
    pub async fn create_host_offer() -> Result<(String, P2PConnection)> {
        let connection = P2PConnection::new_host().await?;
        let sdp = connection.create_offer().await?;
        Ok((sdp, connection))
    }

    /// ゲスト側: Offer SDPを受け取り、Answer SDPを生成し、P2PConnectionを返す
    ///
    /// 戻り値の`P2PConnection`はそのまま戦闘で使用可能。
    pub async fn create_guest_answer(offer_sdp: &str) -> Result<(String, P2PConnection)> {
        let connection = P2PConnection::new_guest().await?;
        let sdp = connection.create_answer(offer_sdp).await?;
        Ok((sdp, connection))
    }
}
