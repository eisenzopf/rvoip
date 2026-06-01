//! TLS SIP client — places a `sips:` call over the TLS transport.
//!
//! The multiplexed transport observes the `sips:` URI scheme and routes the
//! INVITE through the TLS listener instead of UDP. Two modes, selected by the
//! `TLS_INSECURE` env var (set by `run_demo.sh`):
//!
//! - `TLS_INSECURE=1` — dev escape hatch: `tls_insecure_skip_verify=true`, the
//!   server cert is accepted without validation. Requires the
//!   `dev-insecure-tls` Cargo feature (enabled in this example's Cargo.toml).
//! - `TLS_INSECURE=0` — production code path: validate the server cert against
//!   the CA at `TLS_CA_PATH`. Hostname verification runs against the connect
//!   target, so the cert's SAN must include `127.0.0.1`.

use rvoip_sip::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

    let cert_path =
        std::env::var("TLS_CERT_PATH").expect("TLS_CERT_PATH must be set (run_demo.sh does this)");
    let key_path =
        std::env::var("TLS_KEY_PATH").expect("TLS_KEY_PATH must be set (run_demo.sh does this)");
    let insecure = std::env::var("TLS_INSECURE").ok().as_deref() == Some("1");

    let mut config = Config::local("tls_client", 5062).tls_reachable_contact(
        "127.0.0.1:5063".parse()?,
        cert_path,
        key_path,
    );
    config.local_uri = "sips:tls_client@127.0.0.1:5063;transport=tls".to_string();
    config.contact_uri = Some("sips:tls_client@127.0.0.1:5063;transport=tls".to_string());

    if insecure {
        config.tls_insecure_skip_verify = true;
        println!("Mode: insecure (tls_insecure_skip_verify=true) — server cert NOT validated");
    } else {
        let ca_path = std::env::var("TLS_CA_PATH")
            .expect("TLS_CA_PATH must be set for secure mode (run_demo.sh does this)");
        config.tls_extra_ca_path = Some(ca_path.into());
        println!("Mode: secure (CA validation via tls_extra_ca_path) — full cert chain verified");
    }

    let mut peer = StreamPeer::with_config(config).await?;

    println!("Placing TLS call to sips:server@127.0.0.1:5061…");
    let call_id = peer.invite("sips:server@127.0.0.1:5061").send().await?;
    let handle = peer.coordinator().session(&call_id);
    peer.wait_for_answered(handle.id()).await?;
    println!("✅ Call answered over TLS — holding for 500 ms…");

    sleep(Duration::from_millis(500)).await;

    handle.hangup().await?;
    peer.wait_for_ended(handle.id()).await?;
    println!("TLS call done.");

    std::process::exit(0);
}
