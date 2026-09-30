//! Minimal HTTPS/1.1 static file server for the web client. GET/HEAD only,
//! confined to the web root, one request per connection.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::{ServerConfig, crypto::ring};

use crate::config::Config;

const MAX_REQUEST_HEAD: usize = 8 * 1024;
const MAX_FILE: u64 = 32 * 1024 * 1024;
/// Keeps downloads from starving the game loop's runtime.
const MAX_CONCURRENT: usize = 32;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Assets {
    listener: TcpListener,
    acceptor: TlsAcceptor,
    root: PathBuf,
    transport_config: Vec<u8>,
}

impl Assets {
    pub async fn bind(cfg: &Config) -> Result<Assets, String> {
        let certs = CertificateDer::pem_file_iter(&cfg.tls_cert)
            .and_then(|i| i.collect::<Result<Vec<_>, _>>())
            .map_err(|e| format!("HTTPS certificate: {e}"))?;
        let key =
            PrivateKeyDer::from_pem_file(&cfg.tls_key).map_err(|e| format!("HTTPS key: {e}"))?;
        // HTTPS authenticates this configuration. This only changes the
        // WebTransport verifier; HTTPS itself still verifies its certificate.
        let transport_config = if cfg.pin_web_certificate {
            let leaf = certs.first().ok_or("HTTPS certificate chain is empty")?;
            let cert = boring::x509::X509::from_der(leaf.as_ref()).map_err(|e| e.to_string())?;
            let digest = cert
                .digest(boring::hash::MessageDigest::sha256())
                .map_err(|e| e.to_string())?;
            format!("{{\"certificateSha256\":{:?}}}", digest.as_ref()).into_bytes()
        } else {
            b"{}".to_vec()
        };
        let mut tls = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| format!("HTTPS TLS: {e}"))?;
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let root = cfg
            .web_root
            .canonicalize()
            .map_err(|e| format!("PQPQ_WEB_ROOT: {e}"))?;
        let listener = TcpListener::bind(cfg.https_addr)
            .await
            .map_err(|e| format!("HTTPS TCP {}: {e}", cfg.https_addr))?;
        Ok(Assets {
            listener,
            acceptor: TlsAcceptor::from(Arc::new(tls)),
            root,
            transport_config,
        })
    }

    pub async fn run(self, mut shutdown: watch::Receiver<bool>) {
        let limit = Arc::new(Semaphore::new(MAX_CONCURRENT));
        let this = Arc::new(self);
        loop {
            let accepted = tokio::select! {
                a = this.listener.accept() => a,
                _ = shutdown.changed() => return,
            };
            let Ok((tcp, _)) = accepted else { continue };
            let Ok(permit) = limit.clone().try_acquire_owned() else {
                continue;
            };
            let this = this.clone();
            tokio::spawn(async move {
                let _ = tokio::time::timeout(REQUEST_TIMEOUT, this.serve(tcp)).await;
                drop(permit);
            });
        }
    }

    async fn serve(&self, tcp: tokio::net::TcpStream) -> std::io::Result<()> {
        let mut tls = self.acceptor.accept(tcp).await?;
        let mut head = Vec::new();
        let mut buf = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = tls.read(&mut buf).await?;
            if n == 0 || head.len() + n > MAX_REQUEST_HEAD {
                return Ok(());
            }
            head.extend_from_slice(&buf[..n]);
        }
        let line = head.split(|b| *b == b'\r').next().unwrap_or_default();
        let line = String::from_utf8_lossy(line);
        let mut parts = line.split(' ');
        let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let response = match method {
            "GET" | "HEAD" => self.respond(target).await,
            _ => Response::status(405, "Method Not Allowed"),
        };
        tls.write_all(&response.head()).await?;
        if method == "GET" {
            tls.write_all(&response.body).await?;
        }
        tls.shutdown().await
    }

    async fn respond(&self, target: &str) -> Response {
        let path = target.split(['?', '#']).next().unwrap_or("");
        if path == "/transport-config.json" {
            return Response::ok("application/json", self.transport_config.clone());
        }
        let path = if path == "/" { "/index.html" } else { path };
        let Some(file) = resolve(&self.root, path) else {
            return Response::status(404, "Not Found");
        };
        let Ok(meta) = tokio::fs::metadata(&file).await else {
            return Response::status(404, "Not Found");
        };
        if !meta.is_file() || meta.len() > MAX_FILE {
            return Response::status(404, "Not Found");
        }
        match tokio::fs::read(&file).await {
            Ok(body) => Response::ok(content_type(&file), body),
            Err(_) => Response::status(404, "Not Found"),
        }
    }
}

/// Maps a URL path into the web root. Rejects anything but plain relative
/// names, so `..`, backslashes, drive letters and encoded tricks never match.
fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let rel = url_path.strip_prefix('/')?;
    let safe = |seg: &str| {
        !seg.is_empty()
            && !seg.starts_with('.')
            && seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    };
    if !rel.split('/').all(safe) {
        return None;
    }
    let file = root.join(rel).canonicalize().ok()?;
    file.starts_with(root).then_some(file)
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

struct Response {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn ok(content_type: &'static str, body: Vec<u8>) -> Self {
        Response {
            status: 200,
            reason: "OK",
            content_type,
            body,
        }
    }

    fn status(status: u16, reason: &'static str) -> Self {
        Response {
            status,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: reason.as_bytes().to_vec(),
        }
    }

    fn head(&self) -> Vec<u8> {
        // Assets are not content-hashed, so the browser always revalidates.
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\n\
             Content-Security-Policy: default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
             connect-src 'self'; style-src 'self'; img-src 'self'; object-src 'none'; base-uri 'none'; \
             frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n\
             Connection: close\r\n\r\n",
            self.status,
            self.reason,
            self.content_type,
            self.body.len()
        )
        .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_paths_inside_the_root_resolve() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../web")
            .canonicalize()
            .unwrap();
        assert!(resolve(&root, "/index.html").is_some());
        for bad in [
            "/../Cargo.toml",
            "/..%2fCargo.toml",
            "/pkg/../../Cargo.toml",
            "/.git/config",
            "/C:/Windows/win.ini",
            "/a\\..\\b",
            "//etc/passwd",
            "index.html",
            "/missing.html",
        ] {
            assert!(resolve(&root, bad).is_none(), "{bad}");
        }
    }
}
