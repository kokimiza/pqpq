//! Generate a short-lived test identity without altering the OS trust store.
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "certs".into());
    std::fs::create_dir_all(&dir)?;
    let cert = dir.join("localhost.pem");
    let key = dir.join("localhost-key.pem");
    if cert.exists() || key.exists() {
        return Err("Certificate files already exist; use a new directory to rotate them".into());
    }
    let identity = wtransport::Identity::self_signed(["localhost", "127.0.0.1", "::1"])?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(key)?
        .write_all(identity.private_key().to_secret_pem().as_bytes())?;
    options.open(cert)?.write_all(
        identity.certificate_chain().as_slice()[0]
            .to_pem()
            .as_bytes(),
    )?;
    println!(
        "Created 14-day local test certificate in {} (OS trust unchanged)",
        dir.display()
    );
    Ok(())
}
