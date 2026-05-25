//! H6 deferred (#43): headless-Chromium interop test.
//!
//! Drives [`static/whip-publish.html`](../static/whip-publish.html) and
//! [`static/ws-signaling.html`](../static/ws-signaling.html) in headless
//! Chromium against an in-process `WebRtcServer`.
//!
//! Marked `#[ignore]` by default because it requires a Chromium / Chrome
//! binary on `PATH` (or via the `CHROME` env var). To run:
//!
//! ```bash
//! cargo test -p rvoip-webrtc --features interop-browser \
//!     --test browser_interop -- --include-ignored --nocapture
//! ```
//!
//! Install hints:
//! - macOS: `brew install --cask chromium`
//! - Debian/Ubuntu: `apt install chromium`
//! - Arch: `pacman -S chromium`

#![cfg(feature = "interop-browser")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::Path,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::page::EventLoadEventFired;
use futures::StreamExt;
use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};
use tokio::net::TcpListener;
use tokio::sync::Notify;

fn static_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static")
}

/// Serve `static/` over an ephemeral plain-HTTP port (Chromium treats
/// `http://localhost` as a secure context, so `getUserMedia` works).
async fn spawn_static_server() -> (std::net::SocketAddr, Arc<Notify>) {
    let root = static_dir();
    let app = Router::new().route(
        "/:file",
        get(move |Path(file): Path<String>| {
            let root = root.clone();
            async move { serve_file(&root, &file).await }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("static bind");
    let addr = listener.local_addr().expect("static addr");
    let shutdown = Arc::new(Notify::new());
    let shutdown_clone = Arc::clone(&shutdown);
    tokio::spawn(async move {
        let signal = async move { shutdown_clone.notified().await };
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(signal)
            .await;
    });
    (addr, shutdown)
}

async fn serve_file(root: &PathBuf, file: &str) -> impl IntoResponse {
    let safe = file
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if !safe {
        return (StatusCode::BAD_REQUEST, "bad filename").into_response();
    }
    let path = root.join(file);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut headers = HeaderMap::new();
            let ct = if file.ends_with(".html") {
                "text/html; charset=utf-8"
            } else if file.ends_with(".js") {
                "application/javascript"
            } else {
                "application/octet-stream"
            };
            headers.insert(
                "content-type",
                HeaderValue::from_static(ct),
            );
            (StatusCode::OK, headers, bytes).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn launch_browser() -> Result<(Browser, tokio::task::JoinHandle<()>), String> {
    let config = BrowserConfig::builder()
        .arg("--use-fake-ui-for-media-stream")
        .arg("--use-fake-device-for-media-stream")
        .arg("--no-sandbox") // CI containers
        .build()?;
    let (browser, mut handler) = Browser::launch(config)
        .await
        .map_err(|e| format!("chromium launch failed (is the binary on PATH?): {e}"))?;
    let pump = tokio::spawn(async move {
        while let Some(ev) = handler.next().await {
            // Drain the CDP event stream so the browser doesn't backpressure.
            // We don't care about individual events here.
            let _ = ev;
        }
    });
    Ok((browser, pump))
}

#[tokio::test]
#[ignore = "needs Chromium binary on PATH; run with --include-ignored"]
async fn headless_chromium_whip_publish_round_trip() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 1. WebRtcServer with WHIP + WS on ephemeral ports.
    let mut config = WebRtcConfig::loopback();
    config.cors_origins = vec!["*".into()];
    let server = WebRtcServerBuilder::new(config)
        .with_whip("127.0.0.1:0")
        .with_ws("127.0.0.1:0")
        .build()
        .await
        .expect("server");
    let whip = server.whip_addr().expect("whip addr");
    let adapter = server.adapter();

    // 2. Static file server for the demo pages.
    let (static_addr, static_stop) = spawn_static_server().await;

    // 3. Launch headless Chromium and point the publish page at our WHIP server.
    let (mut browser, handler_pump) = match launch_browser().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("SKIP: {e}");
            return;
        }
    };
    let page_url = format!(
        "http://{static_addr}/whip-publish.html?whip=http://{whip}/whip/browser-test"
    );
    let page = browser
        .new_page(&page_url)
        .await
        .expect("open page");

    // Wait for the load event.
    let mut load_events = page
        .event_listener::<EventLoadEventFired>()
        .await
        .expect("subscribe load");
    tokio::time::timeout(Duration::from_secs(10), load_events.next())
        .await
        .expect("page load timeout")
        .expect("page load stream closed");

    // 4. Click the Start button; the page handles getUserMedia + WHIP POST.
    page.find_element("#start")
        .await
        .expect("find #start")
        .click()
        .await
        .expect("click start");

    // 5. Wait up to 20s for the status element to flip to "connected".
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut last_status = String::new();
    let mut connected = false;
    while tokio::time::Instant::now() < deadline {
        if let Ok(el) = page.find_element("#status").await {
            if let Ok(Some(text)) = el.inner_text().await {
                if text != last_status {
                    eprintln!("[browser] status: {text}");
                    last_status = text.clone();
                }
                if text == "connected" {
                    connected = true;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // 6. Assert server-side metrics observed the inbound session.
    let m = adapter.metrics();
    assert!(
        m.inbound_total >= 1,
        "metrics.inbound_total should reflect the browser WHIP POST; got {m:?}"
    );
    assert!(connected, "browser RTCPeerConnection never reached `connected` (last status: {last_status})");

    // Teardown.
    let _ = page.close().await;
    let _ = browser.close().await;
    handler_pump.abort();
    static_stop.notify_waiters();
    server.shutdown().await;
}
