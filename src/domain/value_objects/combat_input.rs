/// 戦闘入力キー
///
/// エンティティ間を流れる「意思」の最小単位。
/// 上下左右、パンチ、キックを1バイトに凝縮。
///
/// ビットレイアウト（下位ビットから）:
/// - bit 0: 上
/// - bit 1: 下
/// - bit 2: 左
/// - bit 3: 右
/// - bit 4: パンチ
/// - bit 5: キック
/// - bit 6-7: 予約（将来の拡張用）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CombatInputKey(u8);

impl CombatInputKey {
    pub const UP: u8 = 1 << 0;
    pub const DOWN: u8 = 1 << 1;
    pub const LEFT: u8 = 1 << 2;
    pub const RIGHT: u8 = 1 << 3;
    pub const PUNCH: u8 = 1 << 4;
    pub const KICK: u8 = 1 << 5;

    /// 空の入力（何も押されていない）
    pub fn empty() -> Self {
        Self(0)
    }

    /// 生のバイト値から作成
    pub fn from_byte(byte: u8) -> Self {
        Self(byte)
    }

    /// バイト値を取得
    pub fn as_byte(&self) -> u8 {
        self.0
    }

    /// 特定のキーが押されているか確認
    pub fn is_pressed(&self, key: u8) -> bool {
        (self.0 & key) != 0
    }

    /// キーを押す
    pub fn press(&mut self, key: u8) {
        self.0 |= key;
    }

    /// キーを離す
    #[allow(dead_code)]
    pub fn release(&mut self, key: u8) {
        self.0 &= !key;
    }

    /// 上が押されているか
    pub fn is_up(&self) -> bool {
        self.is_pressed(Self::UP)
    }

    /// 下が押されているか
    #[allow(dead_code)]
    pub fn is_down(&self) -> bool {
        self.is_pressed(Self::DOWN)
    }

    /// 左が押されているか
    pub fn is_left(&self) -> bool {
        self.is_pressed(Self::LEFT)
    }

    /// 右が押されているか
    pub fn is_right(&self) -> bool {
        self.is_pressed(Self::RIGHT)
    }

    /// パンチが押されているか
    pub fn is_punch(&self) -> bool {
        self.is_pressed(Self::PUNCH)
    }

    /// キックが押されているか
    pub fn is_kick(&self) -> bool {
        self.is_pressed(Self::KICK)
    }

    /// 水平方向の入力を取得（-1: 左, 0: なし, 1: 右）
    pub fn horizontal(&self) -> i8 {
        match (self.is_left(), self.is_right()) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        }
    }

    /// 垂直方向の入力を取得（-1: 上, 0: なし, 1: 下）
    #[allow(dead_code)]
    pub fn vertical(&self) -> i8 {
        match (self.is_up(), self.is_down()) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        }
    }
}

/// 入力履歴（コマンド判定用）
///
/// 隠しコマンド（波動拳コマンドなど）を検出するため、
/// 直近の入力履歴を保持する。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct InputHistory {
    /// 入力履歴（最大16フレーム分）
    pub(crate) history: Vec<CombatInputKey>,

    /// 最大保持フレーム数
    max_frames: usize,
}

#[allow(dead_code)]
impl InputHistory {
    /// 新しい入力履歴を作成
    ///
    /// # 引数
    /// * `max_frames` - 保持する最大フレーム数（通常16-30フレーム）
    pub fn new(max_frames: usize) -> Self {
        Self {
            history: Vec::with_capacity(max_frames),
            max_frames,
        }
    }

    /// 入力を追加
    pub fn push(&mut self, input: CombatInputKey) {
        if self.history.len() >= self.max_frames {
            self.history.remove(0);
        }
        self.history.push(input);
    }

    /// 履歴をクリア
    pub fn clear(&mut self) {
        self.history.clear();
    }

    /// 特定のコマンドパターンが入力されたか判定
    ///
    /// # 引数
    /// * `pattern` - コマンドパターン（例: [DOWN, DOWN_RIGHT, RIGHT, PUNCH]）
    /// * `tolerance_frames` - パターン間の許容フレーム数
    ///
    /// # 戻り値
    /// * `true` - パターンが検出された
    /// * `false` - パターンが検出されなかった
    pub fn matches_pattern(&self, pattern: &[u8], tolerance_frames: usize) -> bool {
        if pattern.is_empty() || self.history.len() < pattern.len() {
            return false;
        }

        let mut pattern_idx = 0;
        let mut tolerance_count = 0;

        for input in self.history.iter().rev() {
            if pattern_idx >= pattern.len() {
                return true;
            }

            if input.as_byte() == pattern[pattern.len() - 1 - pattern_idx] {
                pattern_idx += 1;
                tolerance_count = 0;
            } else {
                tolerance_count += 1;
                if tolerance_count > tolerance_frames {
                    return false;
                }
            }
        }

        pattern_idx == pattern.len()
    }

    /// 直近の入力を取得
    pub fn latest(&self) -> Option<CombatInputKey> {
        self.history.last().copied()
    }

    /// 履歴の長さを取得
    pub fn len(&self) -> usize {
        self.history.len()
    }

    /// 履歴が空か確認
    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_combat_input_basic() {
        let mut input = CombatInputKey::empty();
        assert_eq!(input.as_byte(), 0);

        input.press(CombatInputKey::UP);
        assert!(input.is_up());
        assert!(!input.is_down());

        input.press(CombatInputKey::PUNCH);
        assert!(input.is_up());
        assert!(input.is_punch());

        input.release(CombatInputKey::UP);
        assert!(!input.is_up());
        assert!(input.is_punch());
    }

    #[test]
    fn test_directional_input() {
        let mut input = CombatInputKey::empty();

        input.press(CombatInputKey::LEFT);
        assert_eq!(input.horizontal(), -1);
        assert_eq!(input.vertical(), 0);

        input.release(CombatInputKey::LEFT);
        input.press(CombatInputKey::RIGHT);
        input.press(CombatInputKey::UP);
        assert_eq!(input.horizontal(), 1);
        assert_eq!(input.vertical(), -1);
    }

    #[test]
    fn test_input_history() {
        let mut history = InputHistory::new(4);

        let mut input1 = CombatInputKey::empty();
        input1.press(CombatInputKey::DOWN);
        history.push(input1);

        let mut input2 = CombatInputKey::empty();
        input2.press(CombatInputKey::RIGHT);
        history.push(input2);

        let mut input3 = CombatInputKey::empty();
        input3.press(CombatInputKey::PUNCH);
        history.push(input3);

        assert_eq!(history.len(), 3);
        assert_eq!(history.latest().unwrap().as_byte(), input3.as_byte());
    }

    #[test]
    fn test_pattern_matching() {
        let mut history = InputHistory::new(16);

        // 波動拳コマンド: 下、右下、右、パンチ
        let mut down = CombatInputKey::empty();
        down.press(CombatInputKey::DOWN);
        history.push(down);

        let mut down_right = CombatInputKey::empty();
        down_right.press(CombatInputKey::DOWN);
        down_right.press(CombatInputKey::RIGHT);
        history.push(down_right);

        let mut right = CombatInputKey::empty();
        right.press(CombatInputKey::RIGHT);
        history.push(right);

        let mut punch = CombatInputKey::empty();
        punch.press(CombatInputKey::PUNCH);
        history.push(punch);

        let pattern = vec![
            down.as_byte(),
            down_right.as_byte(),
            right.as_byte(),
            punch.as_byte(),
        ];

        assert!(history.matches_pattern(&pattern, 2));
    }
}
