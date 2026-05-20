//! `ProxyCoordinator` — public stateful-proxy entry point.
//!
//! Parallel to [`UnifiedCoordinator`](crate::api::unified::UnifiedCoordinator)
//! but **dialog-agnostic**: a `ProxyCoordinator` runs a
//! [`StatefulProxy`](rvoip_sip_proxy::StatefulProxy) on top of a
//! [`TransactionManager`], wiring the transport bind and the proxy
//! event loop together so applications get a one-line setup:
//!
//! ```ignore
//! use rvoip_sip::api::proxy_coordinator::{ProxyCoordinator, RouteDecision};
//! use std::sync::Arc;
//!
//! let route = Arc::new(|_req: &_| Some(RouteDecision::to("10.0.0.10:5060".parse().unwrap())));
//! let proxy = ProxyCoordinator::bind("127.0.0.1:5060".parse().unwrap(), route)
//!     .await
//!     .expect("proxy bind");
//! // proxy now forwards INVITEs from the bound port to the route_fn target.
//! ```
//!
//! Mixed-mode (proxy + dialog UA on the same coordinator) is out of
//! scope: a `ProxyCoordinator` owns the primary `TransactionEvent`
//! stream of its `TransactionManager`. To run both a proxy and a UA
//! in one process, bind them to **different** transports.

use std::net::SocketAddr;
use std::sync::Arc;

use rvoip_sip_dialog::transaction::TransactionManager;
use rvoip_sip_proxy::{ProxyConfig, RouteFn, StatefulProxy};
use rvoip_sip_transport::transport::tcp::TcpTransport;
use rvoip_sip_transport::{Transport, TransportEvent, UdpTransport};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::info;

pub use rvoip_sip_proxy::{
    ForkMode, ProxyError, ProxyEvent, ProxyResult, RouteDecision,
};

/// High-level handle for a running stateful proxy.
///
/// Construction binds a transport, builds the transaction manager, and
/// spawns the proxy event loop. The handle keeps these alive — drop it
/// (or call [`Self::shutdown`]) to tear the proxy down.
pub struct ProxyCoordinator {
    proxy: Arc<StatefulProxy>,
    transport: Arc<dyn Transport>,
    local_addr: SocketAddr,
    proxy_task: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for ProxyCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyCoordinator")
            .field("local_addr", &self.local_addr)
            .field("proxy", &self.proxy)
            .finish_non_exhaustive()
    }
}

impl ProxyCoordinator {
    /// Bind a UDP listener at `bind_addr`, wire up a transaction
    /// manager + stateful proxy, and start the proxy event loop.
    ///
    /// `route_fn` is invoked for every inbound request the proxy
    /// receives; return `Some(RouteDecision)` to forward, `None` to
    /// reject with 404.
    pub async fn bind(bind_addr: SocketAddr, route_fn: RouteFn) -> ProxyResult<Arc<Self>> {
        Self::bind_with_config(bind_addr, route_fn, ProxyConfig::default()).await
    }

    /// Like [`bind`](Self::bind) but with an explicit [`ProxyConfig`]
    /// (Timer C duration, Max-Forwards policy).
    pub async fn bind_with_config(
        bind_addr: SocketAddr,
        route_fn: RouteFn,
        config: ProxyConfig,
    ) -> ProxyResult<Arc<Self>> {
        let (udp, transport_rx) = UdpTransport::bind(bind_addr, None)
            .await
            .map_err(|e| ProxyError::Transport(format!("UDP bind: {}", e)))?;
        let local_addr = udp
            .local_addr()
            .map_err(|e| ProxyError::Transport(format!("local_addr: {}", e)))?;
        let transport: Arc<dyn Transport> = Arc::new(udp);

        Self::new(transport, local_addr, transport_rx, route_fn, config).await
    }

    /// Bind a TCP listener at `bind_addr` and start a stateful proxy
    /// over it. Equivalent to [`Self::bind`] but uses TCP instead of
    /// UDP — the right choice when carrier-fronting proxies need
    /// connection-oriented transport (large SIP messages, NAT
    /// pinholes, mutual-auth deployments).
    pub async fn bind_tcp(bind_addr: SocketAddr, route_fn: RouteFn) -> ProxyResult<Arc<Self>> {
        Self::bind_tcp_with_config(bind_addr, route_fn, ProxyConfig::default()).await
    }

    /// Like [`Self::bind_tcp`] but with an explicit [`ProxyConfig`].
    pub async fn bind_tcp_with_config(
        bind_addr: SocketAddr,
        route_fn: RouteFn,
        config: ProxyConfig,
    ) -> ProxyResult<Arc<Self>> {
        let (tcp, transport_rx) = TcpTransport::bind(bind_addr, None, None)
            .await
            .map_err(|e| ProxyError::Transport(format!("TCP bind: {}", e)))?;
        let local_addr = tcp
            .local_addr()
            .map_err(|e| ProxyError::Transport(format!("local_addr: {}", e)))?;
        let transport: Arc<dyn Transport> = Arc::new(tcp);

        Self::new(transport, local_addr, transport_rx, route_fn, config).await
    }

    /// Construct a coordinator over a pre-built transport. Use this
    /// when the application wants to share a transport with other
    /// machinery, or when running over TCP/TLS instead of UDP.
    pub async fn new(
        transport: Arc<dyn Transport>,
        local_addr: SocketAddr,
        transport_rx: mpsc::Receiver<TransportEvent>,
        route_fn: RouteFn,
        config: ProxyConfig,
    ) -> ProxyResult<Arc<Self>> {
        let (tm, events) = TransactionManager::new(transport.clone(), transport_rx, Some(64))
            .await
            .map_err(|e| ProxyError::Transaction(e.to_string()))?;
        let tm = Arc::new(tm);

        let proxy = StatefulProxy::with_config(tm, route_fn, config);
        let proxy_task = proxy.clone().run(events);

        info!("ProxyCoordinator bound at {}", local_addr);
        Ok(Arc::new(Self {
            proxy,
            transport,
            local_addr,
            proxy_task: Some(proxy_task),
        }))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn transport(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }

    pub fn proxy(&self) -> Arc<StatefulProxy> {
        self.proxy.clone()
    }

    /// Subscribe to application-observable proxy events
    /// ([`ProxyEvent`]). See `ProxyEvent` for the emitted variants
    /// (currently `RedirectReceived` for 3xx).
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<ProxyEvent> {
        self.proxy.subscribe_events()
    }

    /// Stop the proxy event loop and close the transport. Idempotent.
    pub async fn shutdown(self: &Arc<Self>) -> ProxyResult<()> {
        // tokio::task::JoinHandle::abort is fine here — the event loop
        // is purely an mpsc consumer, no critical-section work to drain.
        if let Some(task) = self
            .proxy_task
            .as_ref()
            .map(|t| t.abort_handle())
        {
            task.abort();
        }
        self.transport.close().await.ok();
        Ok(())
    }
}
