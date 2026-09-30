use std::fmt;

pub const MAX_USERNAME_BYTES: usize = 48;
pub const MAX_ROOM_ID_LEN: usize = 32;

/// Assigned by the server and never reused within a process; display names are
/// not identifiers.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlayerId(pub u64);

impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputError {
    UsernameEmpty,
    UsernameTooLong,
    UsernameBlank,
    UsernameControl,
    RoomEmpty,
    RoomTooLong,
    RoomChars,
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InputError::UsernameEmpty => "ユーザ名を入力してください",
            InputError::UsernameTooLong => "ユーザ名はUTF-8で48バイト以内にしてください",
            InputError::UsernameBlank => "空白だけのユーザ名は使用できません",
            InputError::UsernameControl => "ユーザ名に制御文字は使用できません",
            InputError::RoomEmpty => "部屋IDを入力してください",
            InputError::RoomTooLong => "部屋IDは32文字以内にしてください",
            InputError::RoomChars => "部屋IDは英数字・-・_ だけで指定してください",
        })
    }
}

impl std::error::Error for InputError {}

/// Display name: 1..=48 UTF-8 bytes, no control or bidi-override characters,
/// not only whitespace. Duplicates are allowed.
pub fn validate_username(name: &str) -> Result<(), InputError> {
    if name.is_empty() {
        return Err(InputError::UsernameEmpty);
    }
    if name.len() > MAX_USERNAME_BYTES {
        return Err(InputError::UsernameTooLong);
    }
    let bidi = |c: char| matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    if name.chars().any(|c| c.is_control() || bidi(c)) {
        return Err(InputError::UsernameControl);
    }
    if name.trim().is_empty() {
        return Err(InputError::UsernameBlank);
    }
    Ok(())
}

/// Case-sensitive ASCII `[A-Za-z0-9_-]{1,32}`; "0012" and "12" differ.
pub fn validate_room_id(room: &str) -> Result<(), InputError> {
    if room.is_empty() {
        return Err(InputError::RoomEmpty);
    }
    if room.len() > MAX_ROOM_ID_LEN {
        return Err(InputError::RoomTooLong);
    }
    if !room
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(InputError::RoomChars);
    }
    Ok(())
}

macro_rules! wire_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident = $v:literal),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum $name { $($variant = $v),* }

        impl $name {
            pub fn from_u8(v: u8) -> Option<Self> {
                match v {
                    $($v => Some($name::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

wire_enum!(Role { Racer = 0, Spectator = 1 });
wire_enum!(RacePhase { Waiting = 0, Racing = 1, Finished = 2 });
wire_enum!(CarStatus { Racing = 0, Finished = 1, Dnf = 2 });
wire_enum!(LeaveReason { Left = 0, Disconnected = 1 });
wire_enum!(CancelReason { RosterChanged = 0 });
wire_enum!(ErrorCode {
    InvalidUsername = 1,
    InvalidRoomId = 2,
    VersionMismatch = 3,
    ServerFull = 4,
    RoomFull = 5,
    ProtocolViolation = 6,
    InvalidState = 7,
    DatagramUnsupported = 8,
    Timeout = 9,
    ServerShutdown = 10,
});

impl ErrorCode {
    /// Short reason for players; never carries internal details.
    pub fn message(self) -> &'static str {
        match self {
            ErrorCode::InvalidUsername => "ユーザ名が不正です",
            ErrorCode::InvalidRoomId => "部屋IDが不正です",
            ErrorCode::VersionMismatch => {
                "対応するクライアントが必要です。最新版を使用してください"
            },
            ErrorCode::ServerFull => "サーバが混雑しています",
            ErrorCode::RoomFull => "部屋が満室です",
            ErrorCode::ProtocolViolation => "通信形式が一致しません",
            ErrorCode::InvalidState => "現在の状態では操作できません",
            ErrorCode::DatagramUnsupported => "この接続はDatagramに対応していません",
            ErrorCode::Timeout => "応答がありません",
            ErrorCode::ServerShutdown => "サーバが停止しました",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert!(validate_username("foo").is_ok());
        assert!(validate_username("ふー 🏎").is_ok());
        assert!(validate_username(&"a".repeat(48)).is_ok());
        assert_eq!(
            validate_username(&"a".repeat(49)),
            Err(InputError::UsernameTooLong)
        );
        assert_eq!(validate_username(""), Err(InputError::UsernameEmpty));
        assert_eq!(validate_username("   "), Err(InputError::UsernameBlank));
        assert_eq!(validate_username("a\nb"), Err(InputError::UsernameControl));
        assert_eq!(
            validate_username("a\u{1b}[2J"),
            Err(InputError::UsernameControl)
        );
        assert_eq!(
            validate_username("a\u{202E}b"),
            Err(InputError::UsernameControl)
        );
    }

    #[test]
    fn room_ids() {
        assert!(validate_room_id("1234").is_ok());
        assert!(validate_room_id("A-b_9").is_ok());
        assert_eq!(validate_room_id(""), Err(InputError::RoomEmpty));
        assert_eq!(
            validate_room_id(&"1".repeat(33)),
            Err(InputError::RoomTooLong)
        );
        assert_eq!(validate_room_id("12 3"), Err(InputError::RoomChars));
        assert_eq!(validate_room_id("１２"), Err(InputError::RoomChars));
    }
}
