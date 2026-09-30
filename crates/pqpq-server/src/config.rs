use std::net::SocketAddr;
use std::path::PathBuf;

/// Startup settings from environment variables; never changed while running.
#[derive(Clone, Debug)]
pub struct Config {
    pub native_addr: SocketAddr,
    pub webtransport_addr: SocketAddr,
    pub https_addr: SocketAddr,
    pub web_root: PathBuf,
    pub web_origin: String,
    pub tls_cert: PathBuf,
    pub tls_key: PathBuf,
    /// Publish the short-lived development certificate's hash over HTTPS.
    pub pin_web_certificate: bool,
    pub max_connections: usize,
    pub max_rooms: usize,
    /// Includes spectators.
    pub max_room_players: usize,
    pub min_racers: usize,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let config = Self {
            native_addr: parse("PQPQ_BIND_ADDR", "127.0.0.1:4433")?,
            webtransport_addr: parse("PQPQ_WEBTRANSPORT_ADDR", "127.0.0.1:8443")?,
            https_addr: parse("PQPQ_HTTPS_ADDR", "127.0.0.1:8443")?,
            web_root: parse("PQPQ_WEB_ROOT", "web")?,
            web_origin: parse("PQPQ_WEB_ORIGIN", "https://localhost:8443")?,
            tls_cert: parse("PQPQ_TLS_CERT", "certs/localhost.pem")?,
            tls_key: parse("PQPQ_TLS_KEY", "certs/localhost-key.pem")?,
            pin_web_certificate: parse("PQPQ_PIN_WEB_CERT", "false")?,
            max_connections: parse("PQPQ_MAX_CONNECTIONS", "128")?,
            max_rooms: parse("PQPQ_MAX_ROOMS", "64")?,
            max_room_players: parse("PQPQ_MAX_ROOM_PLAYERS", "32")?,
            min_racers: parse("PQPQ_MIN_RACERS", "2")?,
        };

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
        if self.min_racers == 0
            || self.max_connections == 0
            || self.max_rooms == 0
            || self.max_room_players == 0
        {
            return Err("PQPQ_MIN_RACERS / PQPQ_MAX_* must be at least 1".into());
        }

        if self.max_room_players > u8::MAX as usize {
            return Err("PQPQ_MAX_ROOM_PLAYERS must be 255 or less".into());
        }
        if self.min_racers > self.max_room_players || self.min_racers > self.max_connections {
            return Err("PQPQ_MIN_RACERS exceeds the room or server capacity".into());
        }
        if self.max_connections > 65_536 || self.max_rooms > 65_536 {
            return Err("PQPQ_MAX_CONNECTIONS / PQPQ_MAX_ROOMS must not exceed 65536".into());
        }
        if !self.web_origin.starts_with("https://")
            || self.web_origin[8..].is_empty()
            || self.web_origin[8..].contains(['/', '?', '#', '@', '\r', '\n', ' '])
        {
            return Err("PQPQ_WEB_ORIGIN must be an HTTPS origin without a path".into());
        }

        for (name, path) in [
            ("PQPQ_TLS_CERT", &self.tls_cert),
            ("PQPQ_TLS_KEY", &self.tls_key),
        ] {
            if !path.is_file() {
                return Err(format!("{name}: {} not found", path.display()));
            }
        }

        let index = self.web_root.join("index.html");
        if !index.is_file() {
            return Err(format!(
                "PQPQ_WEB_ROOT: {} has no index.html",
                self.web_root.display()
            ));
        }

        let root = self.web_root.canonicalize().map_err(|e| e.to_string())?;
        for path in [&self.tls_cert, &self.tls_key] {
            if path
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(&root)
            {
                return Err("TLS certificate and key must be outside PQPQ_WEB_ROOT".into());
            }
        }

        Ok(())
    }
}

fn parse<T: std::str::FromStr>(name: &str, default: &str) -> Result<T, String> {
    let value = std::env::var(name).unwrap_or_else(|_| default.to_owned());
    value.parse().map_err(|_| format!("{name}: invalid value"))
}
