//! Register one rvoip endpoint with the local FreeSWITCH parity profiles.

#[path = "../common.rs"]
mod common;

use common::{endpoint_config, init_tracing, load_env, register_endpoint, ExampleResult};
use rvoip_session_core::StreamPeer;

#[tokio::main]
async fn main() -> ExampleResult<()> {
    load_env();
    init_tracing();

    let transport = std::env::var("SIP_TRANSPORT")
        .or_else(|_| std::env::var("FREESWITCH_TRANSPORT"))
        .unwrap_or_else(|_| "UDP".to_string());
    let username = std::env::var("SIP_USERNAME").unwrap_or_else(|_| {
        if transport.eq_ignore_ascii_case("tls") {
            "1001".to_string()
        } else {
            "2001".to_string()
        }
    });
    let default_port = if transport.eq_ignore_ascii_case("tls") {
        15070
    } else {
        15080
    };
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

    let idle = common::env_duration_secs("IDLE_SECS", 2);
    println!(
        "[registration] Holding {} {} registration for {}s...",
        transport.to_uppercase(),
        username,
        idle.as_secs()
    );
    tokio::time::sleep(idle).await;

    peer.unregister(&registration).await.ok();
    peer.shutdown().await.ok();
    println!("[registration] Done.");
    Ok(())
}
