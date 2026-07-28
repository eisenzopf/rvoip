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
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::adapter::WebRtcAdapter;
use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};

async fn end_all_routes(adapter: &WebRtcAdapter) {
    let route_ids: Vec<_> = adapter
        .routes()
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    for id in route_ids {
        let _ = rvoip_core::adapter::ConnectionAdapter::end(
            adapter,
            id,
            rvoip_core::adapter::EndReason::Normal,
        )
        .await;
    }
}

/// Builder for [`WebRtcServer`].
pub struct WebRtcServerBuilder {
    config: WebRtcConfig,
    inbound_admission_confirmation_timeout: Option<Duration>,
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
    #[cfg(feature = "signaling-whip")]
    whep_mode: crate::signaling::whip::WhepServerMode,
    /// G2 — optional auth hook enforced during WS upgrade.
    #[cfg(feature = "signaling-ws")]
    ws_auth: Option<Arc<dyn crate::signaling::auth::WsAuthHook>>,
}

impl WebRtcServerBuilder {
    pub fn new(config: WebRtcConfig) -> Self {
        Self {
            config,
            inbound_admission_confirmation_timeout: None,
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
            #[cfg(feature = "signaling-whip")]
            whep_mode: crate::signaling::whip::WhepServerMode::Draft04,
            #[cfg(feature = "signaling-ws")]
            ws_auth: None,
        }
    }

    /// Opt in to fail-closed inbound signaling admission.
    ///
    /// The adapter is configured before any listener is bound or spawned.
    /// WHIP, canonical WHEP, and new inbound WebSocket offers then withhold their protocol
    /// success response until an orchestrator admission gate confirms the
    /// exact connection lifecycle. A zero timeout, or one above 30 seconds,
    /// makes [`Self::build`] fail before listeners start.
    pub fn with_inbound_admission_confirmation(mut self, timeout: Duration) -> Self {
        self.inbound_admission_confirmation_timeout = Some(timeout);
        self
    }

    /// Register a [`WhipAuthHook`](crate::signaling::auth::WhipAuthHook) for the
    /// WHIP/WHEP server (G2). Default = anonymous (every request accepted).
    #[cfg(feature = "signaling-whip")]
    pub fn with_whip_auth(mut self, auth: Arc<dyn crate::signaling::auth::WhipAuthHook>) -> Self {
        self.whip_auth = Some(auth);
        self
    }

    /// Select WHEP draft-04 response policy or explicitly enable the legacy
    /// empty-POST/server-offer exchange. Draft-04 client-offer handling is the
    /// default.
    #[cfg(feature = "signaling-whip")]
    pub fn with_whep_server_mode(mut self, mode: crate::signaling::whip::WhepServerMode) -> Self {
        self.whep_mode = mode;
        self
    }

    /// Register a [`WsAuthHook`](crate::signaling::auth::WsAuthHook) for the
    /// WebSocket server (G2). Default = anonymous.
    #[cfg(feature = "signaling-ws")]
    pub fn with_ws_auth(mut self, auth: Arc<dyn crate::signaling::auth::WsAuthHook>) -> Self {
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
        let adapter = match self.inbound_admission_confirmation_timeout {
            Some(timeout) => {
                WebRtcAdapter::new_with_inbound_admission_confirmation(self.config, timeout)?
            }
            None => WebRtcAdapter::new(self.config),
        };
        let mut tasks = Vec::new();
        let (shutdown, _) = watch::channel(false);

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
            let mut signal = shutdown.subscribe();
            let auth = self
                .whip_auth
                .clone()
                .unwrap_or_else(|| Arc::new(crate::signaling::auth::AnonymousAuth));
            let whep_mode = self.whep_mode;
            tasks.push(spawn_signaling_task(async move {
                let shutdown = async move {
                    if !*signal.borrow() {
                        let _ = signal.changed().await;
                    }
                };
                crate::signaling::whip::serve_listener_with_auth_mode_and_shutdown(
                    listener,
                    whip_adapter,
                    auth,
                    whep_mode,
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
            let mut signal = shutdown.subscribe();
            let auth = self
                .whip_auth
                .clone()
                .unwrap_or_else(|| Arc::new(crate::signaling::auth::AnonymousAuth));
            let whep_mode = self.whep_mode;
            tasks.push(spawn_signaling_task(async move {
                let shutdown = async move {
                    if !*signal.borrow() {
                        let _ = signal.changed().await;
                    }
                };
                crate::signaling::whip::serve_tls_with_auth_mode_and_shutdown(
                    std_listener,
                    tls,
                    whip_adapter,
                    auth,
                    whep_mode,
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
            let mut signal = shutdown.subscribe();
            let auth = self
                .ws_auth
                .clone()
                .unwrap_or_else(|| Arc::new(crate::signaling::auth::AnonymousAuth));
            tasks.push(spawn_signaling_task(async move {
                let shutdown = async move {
                    if !*signal.borrow() {
                        let _ = signal.changed().await;
                    }
                };
                crate::signaling::websocket::serve_listener_with_auth_and_shutdown(
                    listener, ws_adapter, auth, shutdown,
                )
                .await
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
            let mut signal = shutdown.subscribe();
            let auth = self
                .ws_auth
                .clone()
                .unwrap_or_else(|| Arc::new(crate::signaling::auth::AnonymousAuth));
            tasks.push(spawn_signaling_task(async move {
                let shutdown = async move {
                    if !*signal.borrow() {
                        let _ = signal.changed().await;
                    }
                };
                crate::signaling::websocket::serve_tls_listener_with_auth_and_shutdown(
                    listener, tls, ws_adapter, auth, shutdown,
                )
                .await
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
    shutdown: watch::Sender<bool>,
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
        let deadline = tokio::time::Instant::now() + deadline;
        self.shutdown.send_replace(true);

        // End active routes (let downstream consumers see `Ended`).
        end_all_routes(&self.adapter).await;

        if !self
            .adapter
            .drain_outbound_signaling(
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            )
            .await
        {
            tracing::warn!(
                outbound_drivers = self.adapter.outbound_signaling_task_count(),
                "WebRtcServer: outbound signaling drain required forced cancellation"
            );
        }

        while self.adapter.metrics().http_resource_tasks > 0
            && tokio::time::Instant::now() < deadline
        {
            tokio::task::yield_now().await;
        }

        // Drain listener tasks with one shared deadline. If an in-flight HTTP
        // request or TLS connection ignores graceful shutdown, retain its
        // JoinHandle, abort it, and join the cancellation instead of dropping
        // a timed-out handle (which would detach the task).
        let mut tasks = self.tasks;
        let mut timed_out_at = None;
        for (index, task) in tasks.iter_mut().enumerate() {
            if tokio::time::timeout_at(deadline, task).await.is_err() {
                timed_out_at = Some(index);
                break;
            }
        }
        if let Some(index) = timed_out_at {
            tracing::warn!("WebRtcServer: graceful shutdown deadline exceeded; aborting");
            for task in tasks.iter().skip(index) {
                if !task.is_finished() {
                    task.abort();
                }
            }
            for task in tasks.into_iter().skip(index) {
                let _ = task.await;
            }
        }

        // A request that was already in flight when shutdown began can publish
        // a route after the first snapshot. Once every listener and accepted
        // connection task has been joined or aborted, no new route can appear,
        // so make one final deterministic cleanup pass for that race window.
        end_all_routes(&self.adapter).await;

        let remaining_routes = self.adapter.routes().len();

        let metrics = self.adapter.metrics();
        if remaining_routes > 0
            || metrics.peer_session_tasks > 0
            || metrics.media_tasks > 0
            || metrics.inbound_ws_connection_tasks > 0
            || metrics.http_resource_tasks > 0
        {
            tracing::warn!(
                remaining_routes,
                peer_session_tasks = metrics.peer_session_tasks,
                media_tasks = metrics.media_tasks,
                inbound_ws_connection_tasks = metrics.inbound_ws_connection_tasks,
                http_resource_tasks = metrics.http_resource_tasks,
                "WebRtcServer: supervised tasks remain after shutdown"
            );
        }
    }
}
