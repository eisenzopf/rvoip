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
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::adapter::WebRtcAdapter;
use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};

/// Builder for [`WebRtcServer`].
pub struct WebRtcServerBuilder {
    config: WebRtcConfig,
    #[cfg(feature = "signaling-whip")]
    whip_bind: Option<String>,
    #[cfg(all(feature = "signaling-whip", feature = "tls-rustls"))]
    whips_bind: Option<(String, crate::tls::TlsConfig)>,
    #[cfg(feature = "signaling-ws")]
    ws_bind: Option<String>,
    #[cfg(all(feature = "signaling-ws", feature = "tls-rustls"))]
    wss_bind: Option<(String, crate::tls::TlsConfig)>,
    /// G2 — optional Bearer auth hook for WHIP/WHEP.
    #[cfg(feature = "signaling-whip")]
    whip_auth: Option<Arc<dyn crate::signaling::auth::WhipAuthHook>>,
    /// G2 — optional auth hook enforced during WS upgrade.
    #[cfg(feature = "signaling-ws")]
    ws_auth: Option<Arc<dyn crate::signaling::auth::WsAuthHook>>,
}

impl WebRtcServerBuilder {
    pub fn new(config: WebRtcConfig) -> Self {
        Self {
            config,
            #[cfg(feature = "signaling-whip")]
            whip_bind: None,
            #[cfg(all(feature = "signaling-whip", feature = "tls-rustls"))]
            whips_bind: None,
            #[cfg(feature = "signaling-ws")]
            ws_bind: None,
            #[cfg(all(feature = "signaling-ws", feature = "tls-rustls"))]
            wss_bind: None,
            #[cfg(feature = "signaling-whip")]
            whip_auth: None,
            #[cfg(feature = "signaling-ws")]
            ws_auth: None,
        }
    }

    /// Register a [`WhipAuthHook`](crate::signaling::auth::WhipAuthHook) for the
    /// WHIP/WHEP server (G2). Default = anonymous (every request accepted).
    #[cfg(feature = "signaling-whip")]
    pub fn with_whip_auth(
        mut self,
        auth: Arc<dyn crate::signaling::auth::WhipAuthHook>,
    ) -> Self {
        self.whip_auth = Some(auth);
        self
    }

    /// Register a [`WsAuthHook`](crate::signaling::auth::WsAuthHook) for the
    /// WebSocket server (G2). Default = anonymous.
    #[cfg(feature = "signaling-ws")]
    pub fn with_ws_auth(
        mut self,
        auth: Arc<dyn crate::signaling::auth::WsAuthHook>,
    ) -> Self {
        self.ws_auth = Some(auth);
        self
    }

    /// Bind WHIP/WHEP HTTP on `addr` (e.g. `"0.0.0.0:8080"` or `"127.0.0.1:0"`).
    #[cfg(feature = "signaling-whip")]
    pub fn with_whip(mut self, addr: impl Into<String>) -> Self {
        self.whip_bind = Some(addr.into());
        self
    }

    /// Bind WHIP/WHEP HTTPS on `addr`. Requires the `tls-rustls` feature.
    #[cfg(all(feature = "signaling-whip", feature = "tls-rustls"))]
    pub fn with_whips(mut self, addr: impl Into<String>, tls: crate::tls::TlsConfig) -> Self {
        self.whips_bind = Some((addr.into(), tls));
        self
    }

    /// Bind WebSocket JSON signaling on `addr`.
    #[cfg(feature = "signaling-ws")]
    pub fn with_ws(mut self, addr: impl Into<String>) -> Self {
        self.ws_bind = Some(addr.into());
        self
    }

    /// Bind WSS (TLS WebSocket) signaling on `addr`. Requires `tls-rustls`.
    #[cfg(all(feature = "signaling-ws", feature = "tls-rustls"))]
    pub fn with_wss(mut self, addr: impl Into<String>, tls: crate::tls::TlsConfig) -> Self {
        self.wss_bind = Some((addr.into(), tls));
        self
    }

    /// Spawn listeners and return a running [`WebRtcServer`].
    pub async fn build(self) -> Result<WebRtcServer> {
        let adapter = WebRtcAdapter::new(self.config);
        let mut tasks = Vec::new();
        let shutdown = Arc::new(Notify::new());

        #[cfg(feature = "signaling-whip")]
        let mut whip_addr = None;
        #[cfg(feature = "signaling-ws")]
        let mut ws_addr = None;
        #[cfg(all(feature = "signaling-whip", feature = "tls-rustls"))]
        let mut whips_addr = None;
        #[cfg(all(feature = "signaling-ws", feature = "tls-rustls"))]
        let mut wss_addr = None;

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
            let signal = Arc::clone(&shutdown);
            let auth = self
                .whip_auth
                .clone()
                .unwrap_or_else(|| Arc::new(crate::signaling::auth::AnonymousAuth));
            tasks.push(spawn_signaling_task(async move {
                let shutdown = async move { signal.notified().await };
                crate::signaling::whip::serve_listener_with_auth_and_shutdown(
                    listener,
                    whip_adapter,
                    auth,
                    shutdown,
                )
                .await
            }));
        }

        #[cfg(all(feature = "signaling-whip", feature = "tls-rustls"))]
        if let Some((bind, tls)) = self.whips_bind {
            let parsed: SocketAddr = bind
                .parse()
                .map_err(|e| WebRtcError::Signaling(format!("parse WHIPS bind {bind}: {e}")))?;
            // Bind once via std to discover the actual port (handles :0), then
            // hand the listener to axum-server's TLS bind.
            let std_listener = std::net::TcpListener::bind(parsed)
                .map_err(|e| WebRtcError::Signaling(format!("bind WHIPS {parsed}: {e}")))?;
            std_listener
                .set_nonblocking(true)
                .map_err(|e| WebRtcError::Signaling(format!("set_nonblocking: {e}")))?;
            let resolved = std_listener
                .local_addr()
                .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
            whips_addr = Some(resolved);
            let whip_adapter = Arc::clone(&adapter);
            let signal = Arc::clone(&shutdown);
            tasks.push(spawn_signaling_task(async move {
                let shutdown = async move { signal.notified().await };
                crate::signaling::whip::serve_tls_with_shutdown(
                    std_listener,
                    tls,
                    whip_adapter,
                    shutdown,
                )
                .await
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
            // WS server already loops accept(); honour shutdown via select! in
            // a wrapper future.
            let signal = Arc::clone(&shutdown);
            let auth = self
                .ws_auth
                .clone()
                .unwrap_or_else(|| Arc::new(crate::signaling::auth::AnonymousAuth));
            tasks.push(spawn_signaling_task(async move {
                tokio::select! {
                    _ = signal.notified() => Ok(()),
                    r = crate::signaling::websocket::serve_listener_with_auth(listener, ws_adapter, auth) => r,
                }
            }));
        }

        #[cfg(all(feature = "signaling-ws", feature = "tls-rustls"))]
        if let Some((bind, tls)) = self.wss_bind {
            let listener = TcpListener::bind(&bind)
                .await
                .map_err(|e| WebRtcError::Signaling(format!("bind WSS {bind}: {e}")))?;
            wss_addr = Some(
                listener
                    .local_addr()
                    .map_err(|e| WebRtcError::Signaling(format!("{e}")))?,
            );
            let ws_adapter = Arc::clone(&adapter);
            let signal = Arc::clone(&shutdown);
            tasks.push(spawn_signaling_task(async move {
                tokio::select! {
                    _ = signal.notified() => Ok(()),
                    r = crate::signaling::websocket::serve_tls_listener(listener, tls, ws_adapter) => r,
                }
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
            shutdown,
            #[cfg(feature = "signaling-whip")]
            whip_addr,
            #[cfg(feature = "signaling-ws")]
            ws_addr,
            #[cfg(all(feature = "signaling-whip", feature = "tls-rustls"))]
            whips_addr,
            #[cfg(all(feature = "signaling-ws", feature = "tls-rustls"))]
            wss_addr,
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
    shutdown: Arc<Notify>,
    #[cfg(feature = "signaling-whip")]
    whip_addr: Option<SocketAddr>,
    #[cfg(feature = "signaling-ws")]
    ws_addr: Option<SocketAddr>,
    #[cfg(all(feature = "signaling-whip", feature = "tls-rustls"))]
    whips_addr: Option<SocketAddr>,
    #[cfg(all(feature = "signaling-ws", feature = "tls-rustls"))]
    wss_addr: Option<SocketAddr>,
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

    /// Resolved WHIPS (HTTPS) listen address.
    #[cfg(all(feature = "signaling-whip", feature = "tls-rustls"))]
    pub fn whips_addr(&self) -> Option<SocketAddr> {
        self.whips_addr
    }

    /// Resolved WSS listen address.
    #[cfg(all(feature = "signaling-ws", feature = "tls-rustls"))]
    pub fn wss_addr(&self) -> Option<SocketAddr> {
        self.wss_addr
    }

    /// Gracefully drain listeners, end all active routes, and abort.
    ///
    /// Steps:
    /// 1. Notify the listeners to stop accepting new connections (axum's
    ///    `with_graceful_shutdown` honours in-flight requests).
    /// 2. Walk every route in the adapter and call
    ///    `ConnectionAdapter::end(_, Normal)` so peers get a clean close.
    /// 3. Wait up to `deadline` for listener tasks to exit, then abort.
    pub async fn shutdown(self) {
        self.shutdown_with_deadline(Duration::from_secs(10)).await;
    }

    pub async fn shutdown_with_deadline(self, deadline: Duration) {
        self.shutdown.notify_waiters();

        // End active routes (let downstream consumers see `Ended`).
        let route_ids: Vec<_> = self.adapter.routes().iter().map(|e| e.key().clone()).collect();
        for id in route_ids {
            let _ = rvoip_core::adapter::ConnectionAdapter::end(
                &*self.adapter,
                id,
                rvoip_core::adapter::EndReason::Normal,
            )
            .await;
        }

        // Drain background tasks with a deadline.
        let drain = async {
            for task in self.tasks {
                let _ = task.await;
            }
        };
        if tokio::time::timeout(deadline, drain).await.is_err() {
            tracing::warn!("WebRtcServer: graceful shutdown deadline exceeded; aborting");
        }
    }
}
