//! TLS SIP client — places a `sips:` call over the TLS transport.
//!
//! Mirrors the server shape: `Config::tls_cert_path` + `tls_key_path`
//! provides the local cert (needed for mutual-TLS-capable registrars,
//! optional for one-way TLS); the multiplexed transport observes the
//! `sips:` URI scheme and routes through the TLS listener instead of
//! UDP.
//!
//! ## Two modes (selected by `TLS_INSECURE` env var)
//!
//! - `TLS_INSECURE=1` (default — the dev escape hatch from Sprint 2.5
//!   P6) — sets `tls_insecure_skip_verify=true`. Server cert is
//!   accepted without validation. Required when the cert isn't signed
//!   by anything in the system trust store and no CA is supplied.
//!
//! - `TLS_INSECURE=0` (the production code path) — the client
//!   validates the server cert against the CA at `TLS_CA_PATH` via
//!   `Config::tls_extra_ca_path`. Hostname verification runs against
//!   the connect target (so the cert's SAN must include `127.0.0.1`).
//!   Real carrier deployments do exactly this, just with a public CA.
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

    // `TLS_INSECURE=1` → Sprint 2.5 P6 dev escape hatch (skip server
    // cert validation). `TLS_INSECURE=0` → validate against the CA
    // at `TLS_CA_PATH` (production code path).
    let insecure = std::env::var("TLS_INSECURE").ok().as_deref() == Some("1");

    let mut config = Config::local("tls_client", 5062);
    config.tls_cert_path = Some(cert_path.into());
    config.tls_key_path = Some(key_path.into());

    if insecure {
        // Dev-only — server cert is accepted without validation.
        // Production builds should not enable the `dev-insecure-tls`
        // feature at all so this field doesn't compile.
        config.tls_insecure_skip_verify = true;
        println!("Mode: insecure (tls_insecure_skip_verify=true) — server cert NOT validated");
    } else {
        // Secure: validate the server cert against the CA. The
        // server cert's SAN must cover `127.0.0.1` for hostname
        // verification to pass.
        let ca_path = std::env::var("TLS_CA_PATH").expect(
            "TLS_CA_PATH must be set for secure mode (the run.sh script does this)",
        );
        config.tls_extra_ca_path = Some(ca_path.into());
        println!("Mode: secure (CA validation via tls_extra_ca_path) — full cert chain verified");
    }

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
