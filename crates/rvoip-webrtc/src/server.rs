//! Unified WebRTC server facade — WHIP/WHEP + WebSocket signaling on one adapter.
//!
//! Requires `signaling-whip` and/or `signaling-ws` features.
//!
//! # Usage
//!
//! ```ignore
//! use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};
//!
//! let server = WebRtcServerBuilder::new(WebRtcConfig::default())
//!     .with_whip("0.0.0.0:8080")
//!     .with_ws("0.0.0.0:8081")
//!     .build()
//!     .await?;
//!
//! let adapter = server.adapter(); // register with rvoip_core::Orchestrator
//! ```

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::adapter::WebRtcAdapter;
use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};

/// Builder for [`WebRtcServer`].
pub struct WebRtcServerBuilder {
    config: WebRtcConfig,
    #[cfg(feature = "signaling-whip")]
    whip_bind: Option<String>,
    #[cfg(feature = "signaling-ws")]
    ws_bind: Option<String>,
}

impl WebRtcServerBuilder {
    pub fn new(config: WebRtcConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "signaling-whip")]
            whip_bind: None,
            #[cfg(feature = "signaling-ws")]
            ws_bind: None,
        }
    }

    /// Bind WHIP/WHEP HTTP on `addr` (e.g. `"0.0.0.0:8080"` or `"127.0.0.1:0"`).
    #[cfg(feature = "signaling-whip")]
    pub fn with_whip(mut self, addr: impl Into<String>) -> Self {
        self.whip_bind = Some(addr.into());
        self
    }

    /// Bind WebSocket JSON signaling on `addr`.
    #[cfg(feature = "signaling-ws")]
    pub fn with_ws(mut self, addr: impl Into<String>) -> Self {
        self.ws_bind = Some(addr.into());
        self
    }

    /// Spawn listeners and return a running [`WebRtcServer`].
    pub async fn build(self) -> Result<WebRtcServer> {
        let adapter = WebRtcAdapter::new(self.config);
        let mut tasks = Vec::new();
        #[cfg(feature = "signaling-whip")]
        let mut whip_addr = None;
        #[cfg(feature = "signaling-ws")]
        let mut ws_addr = None;

        #[cfg(feature = "signaling-whip")]
        if let Some(bind) = self.whip_bind {
            let listener = TcpListener::bind(&bind)
                .await
                .map_err(|e| WebRtcError::Signaling(format!("bind WHIP {bind}: {e}")))?;
            whip_addr = Some(
                listener
                    .local_addr()
                    .map_err(|e| WebRtcError::Signaling(format!("{e}")))?,
            );
            let whip_adapter = Arc::clone(&adapter);
            tasks.push(spawn_signaling_task(async move {
                crate::signaling::whip::serve_listener(listener, whip_adapter).await
            }));
        }

        #[cfg(feature = "signaling-ws")]
        if let Some(bind) = self.ws_bind {
            let listener = TcpListener::bind(&bind)
                .await
                .map_err(|e| WebRtcError::Signaling(format!("bind WS {bind}: {e}")))?;
            ws_addr = Some(
                listener
                    .local_addr()
                    .map_err(|e| WebRtcError::Signaling(format!("{e}")))?,
            );
            let ws_adapter = Arc::clone(&adapter);
            tasks.push(spawn_signaling_task(async move {
                crate::signaling::websocket::serve_listener(listener, ws_adapter).await
            }));
        }

        if tasks.is_empty() {
            return Err(WebRtcError::Signaling(
                "WebRtcServerBuilder: enable signaling-whip and/or signaling-ws and set at least one bind address".into(),
            ));
        }

        Ok(WebRtcServer {
            adapter,
            tasks,
            #[cfg(feature = "signaling-whip")]
            whip_addr,
            #[cfg(feature = "signaling-ws")]
            ws_addr,
        })
    }
}

fn spawn_signaling_task(
    fut: impl std::future::Future<Output = Result<()>> + Send + 'static,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = fut.await {
            tracing::error!("WebRTC signaling server stopped: {e}");
        }
    })
}

/// Running server — shares one [`WebRtcAdapter`] across WHIP/WHEP and/or WS listeners.
pub struct WebRtcServer {
    adapter: Arc<WebRtcAdapter>,
    tasks: Vec<JoinHandle<()>>,
    #[cfg(feature = "signaling-whip")]
    whip_addr: Option<SocketAddr>,
    #[cfg(feature = "signaling-ws")]
    ws_addr: Option<SocketAddr>,
}

impl WebRtcServer {
    /// Shared adapter — register with [`rvoip_core::Orchestrator::register`] before accepting traffic.
    pub fn adapter(&self) -> Arc<WebRtcAdapter> {
        Arc::clone(&self.adapter)
    }

    /// Resolved WHIP/WHEP listen address (after `127.0.0.1:0` style binds).
    #[cfg(feature = "signaling-whip")]
    pub fn whip_addr(&self) -> Option<SocketAddr> {
        self.whip_addr
    }

    /// Resolved WebSocket listen address.
    #[cfg(feature = "signaling-ws")]
    pub fn ws_addr(&self) -> Option<SocketAddr> {
        self.ws_addr
    }

    /// Abort background listener tasks.
    pub async fn shutdown(self) {
        for task in self.tasks {
            task.abort();
        }
    }
}
