//! Comprehensive WebRTC client — connects via [`WebRtcClient`] + [`WsSignaler`].

use std::env;
use std::process;
use std::time::Duration;

use rvoip_webrtc::client::comprehensive::run_client_checks_with_chat;
use rvoip_webrtc::client::{CallTarget, SessionMedium, WebRtcClient, WsSignaler};
use rvoip_webrtc::WebRtcConfig;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let ws_url = env::var("WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:8081".into());
    let medium = parse_medium(env::args().skip(1).next().as_deref());
    let chat_body = env::var("CHAT_MESSAGE").unwrap_or_else(|_| "Hello from comprehensive suite!".into());

    let result = async {
        let client = WebRtcClient::connect(WebRtcConfig::loopback(), &ws_url).await?;
        let signaler = WsSignaler::new(&ws_url);
        let session = client
            .call(
                &signaler,
                CallTarget::Uri("comprehensive-test".into()),
                medium,
            )
            .await?;

        let report = run_client_checks_with_chat(&session, medium, &chat_body).await?;
        tracing::info!(?report, ?medium, chat = %chat_body, "comprehensive checks passed");
        Ok::<(), Box<dyn std::error::Error>>(())
    }
    .await;

    if let Err(e) = result {
        eprintln!("comprehensive client failed: {e}");
        process::exit(1);
    }
}

fn parse_medium(arg: Option<&str>) -> SessionMedium {
    match arg.unwrap_or("audiovideo") {
        "audio" => SessionMedium::Audio,
        "video" => SessionMedium::Video,
        "audiovideo" | "av" => SessionMedium::AudioVideo,
        other => {
            eprintln!("unknown medium '{other}', use audio|video|audiovideo");
            process::exit(2);
        }
    }
}
