//! Register one rvoip endpoint with the Asterisk interop profile.

#[path = "../common.rs"]
mod common;

use common::{endpoint_config, init_tracing, load_env, register_endpoint, ExampleResult};
use rvoip_session_core::StreamPeer;

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let transport = std::env::var("SIP_TRANSPORT").unwrap_or_else(|_| "TLS".to_string());
    if std::env::var_os("SIP_TRANSPORT").is_none() {
        std::env::set_var("SIP_TRANSPORT", &transport);
    }
    let username = std::env::var("SIP_USERNAME").unwrap_or_else(|_| {
        if transport.eq_ignore_ascii_case("tls") {
            "1001".to_string()
        } else {
            "2001".to_string()
        }
    });
    let default_port = std::env::var("LOCAL_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| {
            if transport.eq_ignore_ascii_case("tls") {
                5070
            } else {
                5080
            }
        });
    let default_media_start = if transport.eq_ignore_ascii_case("tls") {
        16000
    } else {
        17000
    };

    let cfg = endpoint_config(
        &username,
        default_port,
        default_media_start,
        default_media_start + 100,
    )?;
    let config = if transport.eq_ignore_ascii_case("tls") {
        cfg.tls_srtp_stream_config()?
    } else {
        cfg.stream_config()
    };
    let mut peer = StreamPeer::with_config(config).await?;
    let registration = register_endpoint(&mut peer, &cfg).await?;

    let idle = std::env::var("IDLE_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2);
    println!(
        "[registration] Holding {} {} registration for {}s...",
        transport.to_uppercase(),
        username,
        idle
    );
    tokio::time::sleep(std::time::Duration::from_secs(idle)).await;

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[registration] Done.");
    Ok(())
}
