use std::path::PathBuf;

use pqpq_protocol::{validate_room_id, validate_username};

/// Built-in destination; override with PQPQ_SERVER_ADDR.
const DEFAULT_SERVER: &str = "localhost:4433";

/// Startup settings. Nothing here is ever written back to disk.
#[derive(Clone, Debug)]
pub struct Config {
    pub username: String,
    pub room_id: String,
    pub server_addr: String,
    pub server_name: String,
    pub ca_file: Option<PathBuf>,
}

pub enum ArgsError {
    Usage,
    Invalid(String),
}

impl Config {
    /// `pqpq <username> <room_id>`, exactly two positional arguments.
    pub fn from_args(args: &[String]) -> Result<Config, ArgsError> {
        let [username, room_id] = args else {
            return Err(ArgsError::Usage);
        };
        validate_username(username).map_err(|e| ArgsError::Invalid(e.to_string()))?;
        validate_room_id(room_id).map_err(|e| ArgsError::Invalid(e.to_string()))?;
        let server_addr =
            std::env::var("PQPQ_SERVER_ADDR").unwrap_or_else(|_| DEFAULT_SERVER.to_owned());
        let server_name = std::env::var("PQPQ_TLS_SERVER_NAME")
            .unwrap_or_else(|_| host_of(&server_addr).to_owned());
        Ok(Config {
            username: username.clone(),
            room_id: room_id.clone(),
            server_addr,
            server_name,
            ca_file: std::env::var_os("PQPQ_CA_FILE").map(PathBuf::from),
        })
    }
}

/// Host part of `host:port` or `[v6]:port`.
fn host_of(addr: &str) -> &str {
    if let Some(rest) = addr.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    addr.rsplit_once(':').map_or(addr, |(h, _)| h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_parsing() {
        assert_eq!(host_of("localhost:4433"), "localhost");
        assert_eq!(host_of("[::1]:4433"), "::1");
        assert_eq!(host_of("example.com"), "example.com");
    }

    #[test]
    fn exactly_two_valid_arguments() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(Config::from_args(&args(&["foo", "1234"])).is_ok());
        assert!(matches!(
            Config::from_args(&args(&["foo"])),
            Err(ArgsError::Usage)
        ));
        assert!(matches!(
            Config::from_args(&args(&["foo", "1", "x"])),
            Err(ArgsError::Usage)
        ));
        assert!(matches!(
            Config::from_args(&args(&["foo", "12 34"])),
            Err(ArgsError::Invalid(_))
        ));
    }
}
