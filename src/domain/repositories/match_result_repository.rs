use crate::domain::entities::MatchResult;
use crate::domain::value_objects::PublicKey;
use anyhow::Result;
use async_trait::async_trait;

/// 戦績リポジトリ
///
/// 署名付き対戦結果の永続化と検索を担当する。
#[async_trait]
#[allow(dead_code)]
pub trait MatchResultRepository: Send + Sync {
    /// 対戦結果を保存
    async fn save(&self, result: &MatchResult) -> Result<()>;

    /// プレイヤーの戦績を取得（最新順）
    async fn find_by_pubkey(&self, pubkey: &PublicKey, limit: u32) -> Result<Vec<MatchResult>>;
}
