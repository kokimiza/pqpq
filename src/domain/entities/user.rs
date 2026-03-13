use uuid::Uuid;

/// エンドユーザーエンティティ
///
/// 一般的なSNSで扱う項目属性を持つ定性的なスキーマ。
/// スタッツデータは含まない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// ユーザーID（主キー）
    id: Uuid,

    /// pqpq-address（ユーザコード、連絡先）
    ///
    /// 他のユーザーがこのアドレスを使って連絡・招待できる。
    /// フォーマット: 英数字とハイフン、8-32文字
    pqpq_address: String,

    /// ユーザー名（表示名）
    ///
    /// 対戦画面やランキングで表示される名前。
    /// 1-50文字、絵文字対応
    username: String,
}

impl User {
    /// 新しいユーザーを作成
    ///
    /// # 引数
    /// * `pqpq_address` - ユーザコード（8-32文字の英数字とハイフン）
    /// * `username` - 表示名（1-50文字）
    ///
    /// # パニック
    /// * pqpq_addressが8文字未満または32文字超の場合
    /// * pqpq_addressに無効な文字が含まれる場合
    /// * usernameが空または50文字超の場合
    pub fn new(pqpq_address: String, username: String) -> Self {
        Self::validate_pqpq_address(&pqpq_address);
        Self::validate_username(&username);

        Self {
            id: Uuid::new_v4(),
            pqpq_address,
            username,
        }
    }

    /// 既存のIDでユーザーを再構築（リポジトリからの復元用）
    #[allow(dead_code)]
    pub fn reconstruct(id: Uuid, pqpq_address: String, username: String) -> Self {
        Self::validate_pqpq_address(&pqpq_address);
        Self::validate_username(&username);

        Self {
            id,
            pqpq_address,
            username,
        }
    }

    /// ユーザー名を変更
    #[allow(dead_code)]
    pub fn change_username(&mut self, new_username: String) {
        Self::validate_username(&new_username);
        self.username = new_username;
    }

    fn validate_pqpq_address(address: &str) {
        assert!(
            address.len() >= 8 && address.len() <= 32,
            "pqpq-address must be 8-32 characters"
        );
        assert!(
            address
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-'),
            "pqpq-address must contain only alphanumeric characters and hyphens"
        );
    }

    fn validate_username(username: &str) {
        let char_count = username.chars().count();
        assert!(
            (1..=50).contains(&char_count),
            "username must be 1-50 characters"
        );
    }

    pub fn id(&self) -> &Uuid {
        &self.id
    }

    #[allow(dead_code)]
    pub fn pqpq_address(&self) -> &str {
        &self.pqpq_address
    }

    #[allow(dead_code)]
    pub fn username(&self) -> &str {
        &self.username
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_valid_user() {
        let user = User::new("test-user-123".to_string(), "TestPlayer".to_string());
        assert_eq!(user.pqpq_address(), "test-user-123");
        assert_eq!(user.username(), "TestPlayer");
    }

    #[test]
    #[should_panic(expected = "pqpq-address must be 8-32 characters")]
    fn test_pqpq_address_too_short() {
        User::new("short".to_string(), "TestPlayer".to_string());
    }

    #[test]
    #[should_panic(expected = "pqpq-address must contain only alphanumeric")]
    fn test_pqpq_address_invalid_chars() {
        User::new("invalid@address".to_string(), "TestPlayer".to_string());
    }

    #[test]
    #[should_panic(expected = "username must be 1-50 characters")]
    fn test_username_empty() {
        User::new("valid-address".to_string(), "".to_string());
    }

    #[test]
    fn test_change_username() {
        let mut user = User::new("test-user-123".to_string(), "OldName".to_string());
        user.change_username("NewName".to_string());
        assert_eq!(user.username(), "NewName");
    }
}
