//! TLS SIP client — places a `sips:` call over the TLS transport.
//!
//! Mirrors the server shape: `Config::tls_cert_path` + `tls_key_path`
//! provides the local cert (needed for mutual-TLS-capable registrars,
//! optional for one-way TLS); the multiplexed transport observes the
//! `sips:` URI scheme and routes through the TLS listener instead of
//! UDP.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_tls_client --features dev-insecure-tls
//! Or with server:  ./examples/streampeer/tls/run.sh

#[cfg(not(feature = "dev-insecure-tls"))]
compile_error!(
    "streampeer/tls example requires --features dev-insecure-tls (self-signed dev cert)"
);

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let cert_path = std::env::var("TLS_CERT_PATH")
        .expect("TLS_CERT_PATH must be set (the run.sh script does this)");
    let key_path = std::env::var("TLS_KEY_PATH")
        .expect("TLS_KEY_PATH must be set (the run.sh script does this)");

    let mut config = Config::local("tls_client", 5062);
    config.tls_cert_path = Some(cert_path.into());
    config.tls_key_path = Some(key_path.into());
    // Dev-only — same warning as the server. For production, remove
    // this and rely on the system trust store.
    config.tls_insecure_skip_verify = true;

    let mut peer = StreamPeer::with_config(config).await?;

    println!("Placing TLS call to sips:server@127.0.0.1:5061…");
    let handle = peer.call("sips:server@127.0.0.1:5061").await?;
    peer.wait_for_answered(handle.id()).await?;
    println!("Call answered over TLS — holding for 500 ms…");

    sleep(Duration::from_millis(500)).await;

    handle.hangup().await?;
    peer.wait_for_ended(handle.id()).await?;
    println!("TLS call done.");

    std::process::exit(0);
}
