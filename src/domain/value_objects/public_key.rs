use crate::domain::errors::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicKey(String);

impl PublicKey {
    pub fn new(value: String) -> Result<Self, DomainError> {
        if value.is_empty() {
            return Err(DomainError::InvalidPublicKey(
                "Public key cannot be empty".to_string(),
            ));
        }
        // 基本的な長さチェック（Ed25519は32バイト = 64文字のhex）
        if value.len() < 32 {
            return Err(DomainError::InvalidPublicKey(
                "Public key too short".to_string(),
            ));
        }
        Ok(Self(value))
    }

    pub fn value(&self) -> &str {
        &self.0
    }

    /// バイト配列から公開鍵を作成（テスト・スナップショット用）
    ///
    /// # 引数
    /// * `bytes` - 32バイトの公開鍵データ
    #[allow(dead_code)]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        assert_eq!(bytes.len(), 32, "Public key must be 32 bytes");
        Self(hex::encode(bytes))
    }

    /// 公開鍵をバイト配列として取得（スナップショット用）
    ///
    /// # 戻り値
    /// * 32バイトの公開鍵データ
    #[allow(dead_code)]
    pub fn as_bytes(&self) -> Vec<u8> {
        hex::decode(&self.0).unwrap_or_else(|_| vec![0u8; 32])
    }
}
