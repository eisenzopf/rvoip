//! Batteries-included SIP → Amazon Connect screen-pop server.
//!
//! [`ConnectScreenPopServer`] is the turnkey entry point: give it one
//! [`ScreenPopServerConfig`] and call [`ConnectScreenPopServer::serve`]. It
//! stands up a SIP UAS, and for every inbound INVITE (e.g. a Vapi blind
//! transfer) it:
//!
//! 1. reads the custom SIP headers,
//! 2. translates them to Amazon Connect contact attributes (the screen-pop
//!    channel) via the configured [`AttributeMapping`],
//! 3. answers the SIP leg,
//! 4. places an inbound WebRTC contact into Connect ([`AmazonConnectAdapter`]),
//! 5. bridges the SIP audio (G.711) to the Connect audio (Opus), transcoding.
//!
//! The Connect contact flow + agent CCP then perform the actual screen pop from
//! the attributes (an AWS-side configuration task).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, EndReason};
use rvoip_core::ids::ConnectionId;
use rvoip_core::stream::MediaStream;
use rvoip_sip::{
    Config as SipConfig, Event as SipEvent, IncomingCall, SessionId as SipSessionId,
    UnifiedCoordinator,
};
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderName, TypedHeader};
use tracing::{info, warn};

use crate::adapter::{AmazonConnectAdapter, ContactTarget};
use crate::bridge::{bridge_streams, StreamBridge};
use crate::config::ConnectConfig;
use crate::control::ConnectContactStarter;
use crate::errors::{ConnectError, Result};
use crate::mapping::AttributeMapping;

/// The Connect target a [`ContactRouter`] selected for one inbound call.
///
/// Every `None` field falls back to the server-wide [`ConnectConfig`] /
/// [`AttributeMapping`], so a route only needs to carry what differs per
/// tenant.
#[derive(Clone, Debug, Default)]
pub struct ContactRoute {
    /// Metrics/logging label for this route (e.g. the tenant name). Keyed
    /// into [`ConnectScreenPopServer::route_metrics`].
    pub label: String,
    /// Amazon Connect instance id override.
    pub instance_id: Option<String>,
    /// Contact-flow id override.
    pub contact_flow_id: Option<String>,
    /// Per-route SIP-header → attribute mapping override.
    pub attribute_mapping: Option<AttributeMapping>,
    /// Display-name fallback override (used when the INVITE supplies none).
    pub default_display_name: Option<String>,
}

/// A [`ContactRouter`]'s verdict for one inbound INVITE.
pub enum RouteDecision {
    /// Bridge the call into Connect with these per-call parameters.
    Route(ContactRoute),
    /// Reject the INVITE with this SIP status/reason (e.g. `404 Not Found`
    /// for an unknown tenant).
    Reject {
        /// SIP status code (4xx/5xx/6xx).
        status: u16,
        /// SIP reason phrase.
        reason: String,
    },
}

/// Per-call routing hook: inspect the inbound INVITE (Request-URI / To user
/// part, headers, …) and pick the Connect target — the multi-tenant enabler.
pub type ContactRouter = Arc<dyn Fn(&IncomingCall) -> RouteDecision + Send + Sync>;

/// Configuration for the turnkey screen-pop server — one object, batteries
/// included.
pub struct ScreenPopServerConfig {
    /// SIP UAS settings (bind address, local URI, timers). Build with
    /// `rvoip_sip::Config::local(name, port)` or `Config::on(name, ip, port)`.
    pub sip: SipConfig,
    /// Amazon Connect control + media settings (instance/flow/region, mapping,
    /// timeouts).
    pub connect: ConnectConfig,
    /// The control-plane starter. Use `AwsConnectStarter` (feature
    /// `aws-control`) for the real path, or a mock in tests.
    pub starter: Arc<dyn ConnectContactStarter>,
    /// Optional per-call router. `None` preserves the classic behaviour:
    /// every INVITE goes to `connect`'s instance/flow with its mapping.
    pub router: Option<ContactRouter>,
}

impl ScreenPopServerConfig {
    /// Construct with the three required pieces (no per-call routing).
    pub fn new(
        sip: SipConfig,
        connect: ConnectConfig,
        starter: Arc<dyn ConnectContactStarter>,
    ) -> Self {
        Self {
            sip,
            connect,
            starter,
            router: None,
        }
    }

    /// Set the per-call router (builder-style).
    pub fn with_router(mut self, router: ContactRouter) -> Self {
        self.router = Some(router);
        self
    }
}

/// Active bridged contact: keeps the SIP↔Connect bridge alive and remembers the
/// Connect connection so it can be torn down when the SIP leg ends.
struct ActiveContact {
    _bridge: StreamBridge,
    connect_conn: ConnectionId,
    /// Route label for per-route metrics (`None` on the unrouted path).
    route_label: Option<String>,
}

/// Per-route (per-tenant) counters, updated by `handle_call`/teardown.
#[derive(Default)]
struct RouteStats {
    contacts_started: AtomicU64,
    failures: AtomicU64,
    active_sessions: AtomicI64,
}

/// Snapshot of one route's counters (see
/// [`ConnectScreenPopServer::route_metrics`]).
#[derive(Clone, Debug, Default)]
pub struct RouteMetrics {
    /// Contacts successfully started (StartWebRTCContact succeeded).
    pub contacts_started: u64,
    /// Calls that failed anywhere between accept and bridge.
    pub failures: u64,
    /// Currently bridged calls.
    pub active_sessions: u64,
}

/// The running server.
pub struct ConnectScreenPopServer {
    coordinator: Arc<UnifiedCoordinator>,
    adapter: Arc<AmazonConnectAdapter>,
    mapping: AttributeMapping,
    router: Option<ContactRouter>,
    /// Per-route-label counters; populated only when a router is configured.
    route_stats: DashMap<String, Arc<RouteStats>>,
    /// Authoritative map of live bridges, keyed by SIP session. Removal from
    /// this map is the single teardown "claim" so the two directions
    /// (SIP-ended, Connect-ended) never double-tear-down.
    active: Arc<DashMap<SipSessionId, ActiveContact>>,
    /// Reverse index: Connect connection → SIP session, so an adapter `Ended`
    /// event can find the SIP leg to hang up.
    by_connect: Arc<DashMap<ConnectionId, SipSessionId>>,
}

impl ConnectScreenPopServer {
    /// Build the server: start the SIP coordinator and the Connect adapter.
    pub async fn build(config: ScreenPopServerConfig) -> Result<Arc<Self>> {
        let mapping = config.connect.attribute_mapping.clone();
        let coordinator = UnifiedCoordinator::new(config.sip)
            .await
            .map_err(|e| ConnectError::Signaling(format!("SIP coordinator: {e}")))?;
        let adapter = AmazonConnectAdapter::new(config.connect, config.starter);

        Ok(Arc::new(Self {
            coordinator,
            adapter,
            mapping,
            router: config.router,
            route_stats: DashMap::new(),
            active: Arc::new(DashMap::new()),
            by_connect: Arc::new(DashMap::new()),
        }))
    }

    /// The underlying Connect adapter (e.g. to read metrics).
    pub fn adapter(&self) -> &Arc<AmazonConnectAdapter> {
        &self.adapter
    }

    /// Snapshot of the per-route counters, keyed by [`ContactRoute::label`].
    /// Empty when no router is configured (use
    /// [`AmazonConnectAdapter::metrics`] for the process-wide view).
    pub fn route_metrics(&self) -> BTreeMap<String, RouteMetrics> {
        self.route_stats
            .iter()
            .map(|e| {
                (
                    e.key().clone(),
                    RouteMetrics {
                        contacts_started: e.contacts_started.load(Ordering::Relaxed),
                        failures: e.failures.load(Ordering::Relaxed),
                        active_sessions: e.active_sessions.load(Ordering::Relaxed).max(0) as u64,
                    },
                )
            })
            .collect()
    }

    fn stats_for(&self, label: &str) -> Arc<RouteStats> {
        self.route_stats
            .entry(label.to_string())
            .or_default()
            .clone()
    }

    /// Run the accept loop forever: each inbound INVITE is translated, the
    /// Connect contact is placed, and the two legs are bridged. Per-call
    /// failures are logged and skipped; the loop continues.
    pub async fn serve(self: Arc<Self>) -> Result<()> {
        // Bidirectional teardown:
        //  • SIP leg ends  → LEAVE the Chime meeting (spawn_teardown_watcher).
        //  • Connect leg ends (agent hangup) → BYE the SIP carrier
        //    (spawn_connect_end_watcher).
        self.spawn_teardown_watcher().await?;
        self.spawn_connect_end_watcher();

        let mut events = self
            .coordinator
            .events()
            .await
            .map_err(|e| ConnectError::Signaling(format!("SIP events: {e}")))?;
        info!("ConnectScreenPopServer listening for inbound SIP calls");

        loop {
            let incoming = match self.coordinator.next_incoming_call(&mut events).await {
                Ok(Some(call)) => call,
                Ok(None) => {
                    info!("SIP event stream ended; stopping server");
                    return Ok(());
                }
                Err(e) => {
                    warn!(error = %e, "error waiting for incoming SIP call");
                    continue;
                }
            };

            let me = Arc::clone(&self);
            // Handle each call on its own task so a slow Connect handshake
            // doesn't block the next inbound INVITE.
            tokio::spawn(async move {
                if let Err(e) = me.handle_call(incoming).await {
                    warn!(error = %e, "failed to bridge inbound call to Amazon Connect");
                }
            });
        }
    }

    /// Route the call, then translate → answer → originate → bridge. A
    /// configured router can divert the call to a per-tenant Connect target
    /// or reject it outright (e.g. `404` for an unknown tenant).
    async fn handle_call(self: &Arc<Self>, call: IncomingCall) -> Result<()> {
        // 0. Per-call routing decision (multi-tenant hook).
        let route = match &self.router {
            Some(router) => match router(&call) {
                RouteDecision::Route(route) => Some(route),
                RouteDecision::Reject { status, reason } => {
                    info!(
                        to = %call.to,
                        status,
                        reason = %reason,
                        "router rejected inbound SIP call"
                    );
                    call.reject(status, &reason);
                    return Ok(());
                }
            },
            None => None,
        };

        let stats = route.as_ref().map(|r| self.stats_for(&r.label));
        let result = self.bridge_call(call, route).await;
        if result.is_err() {
            if let Some(stats) = stats {
                stats.failures.fetch_add(1, Ordering::Relaxed);
            }
        }
        result
    }

    /// Translate headers → attributes, answer SIP, originate Connect, bridge.
    async fn bridge_call(
        self: &Arc<Self>,
        call: IncomingCall,
        route: Option<ContactRoute>,
    ) -> Result<()> {
        let session_id = call.call_id.clone();
        let display_name = Some(call.from.clone());
        let route_label = route.as_ref().map(|r| r.label.clone());

        // 1. Extract custom headers and translate to Connect attributes.
        let headers = extract_headers(&call);
        // Diagnostic: the full inbound header set + the resulting attributes.
        // Enable with `RUST_LOG=rvoip_amazon_connect::sip_headers=debug` — this is
        // how you confirm whether a carrier preserved the custom `X-` headers
        // across a Vapi REFER/transfer (the crux of the end-to-end test).
        tracing::debug!(
            target: "rvoip_amazon_connect::sip_headers",
            count = headers.len(),
            headers = ?headers,
            "inbound INVITE headers"
        );
        let mapping = route
            .as_ref()
            .and_then(|r| r.attribute_mapping.as_ref())
            .unwrap_or(&self.mapping);
        let mapped = mapping.translate(headers);
        tracing::debug!(
            target: "rvoip_amazon_connect::sip_headers",
            attributes = ?mapped.attributes,
            skipped = ?mapped.skipped,
            "mapped Connect contact attributes"
        );
        info!(
            from = %call.from,
            route = route_label.as_deref().unwrap_or("-"),
            attributes = mapped.attributes.len(),
            "inbound SIP call → Amazon Connect screen pop"
        );

        // 2. Answer the SIP leg.
        let handle = call
            .accept()
            .await
            .map_err(|e| ConnectError::Signaling(format!("SIP accept: {e}")))?;
        let sip_session: SipSessionId = handle.id().clone();

        // 3. Build the SIP media stream (inbound G.711).
        let sip_stream = rvoip_sip::media_stream::SipMediaStream::new(
            Arc::clone(&self.coordinator),
            sip_session.clone(),
            rvoip_core::connection::Direction::Inbound,
        )
        .await
        .map_err(|e| ConnectError::Signaling(format!("SIP media stream: {e}")))?
            as Arc<dyn MediaStream>;

        // 4. Place the inbound WebRTC contact into Amazon Connect, honouring
        //    the route's per-call instance/flow override.
        let target = route
            .as_ref()
            .map(|r| ContactTarget {
                instance_id: r.instance_id.clone(),
                contact_flow_id: r.contact_flow_id.clone(),
                default_display_name: r.default_display_name.clone(),
            })
            .unwrap_or_default();
        let connect_conn = self
            .adapter
            .originate_contact_to(target, mapped.attributes, display_name, None)
            .await?;
        if let Some(label) = &route_label {
            self.stats_for(label)
                .contacts_started
                .fetch_add(1, Ordering::Relaxed);
        }

        let connect_streams = self
            .adapter
            .streams_for(&connect_conn)
            .ok_or(ConnectError::UnknownConnection(connect_conn.to_string()))?;
        let connect_stream = connect_streams
            .into_iter()
            .next()
            .ok_or_else(|| ConnectError::WebRtc("Connect contact has no media stream".into()))?;

        // 5. Bridge the two legs (transcoding G.711 ⟷ Opus).
        let bridge = bridge_streams(sip_stream, connect_stream)?;
        self.by_connect
            .insert(connect_conn.clone(), session_id.clone());
        self.active.insert(
            session_id.clone(),
            ActiveContact {
                _bridge: bridge,
                connect_conn,
                route_label: route_label.clone(),
            },
        );
        if let Some(label) = &route_label {
            self.stats_for(label)
                .active_sessions
                .fetch_add(1, Ordering::Relaxed);
        }
        info!(
            session = %session_id,
            route = route_label.as_deref().unwrap_or("-"),
            "bridged SIP ⟷ Amazon Connect"
        );

        Ok(())
    }

    /// Subscribe a dedicated event stream and end the Connect leg whenever the
    /// matching SIP leg terminates (`CallEnded`/`CallFailed`/`CallCancelled`).
    /// Uses its own broadcast subscription so it never competes with the
    /// incoming-call loop.
    async fn spawn_teardown_watcher(self: &Arc<Self>) -> Result<()> {
        let mut events = self
            .coordinator
            .events()
            .await
            .map_err(|e| ConnectError::Signaling(format!("SIP teardown events: {e}")))?;
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let call_id = match event {
                    SipEvent::CallEnded { call_id, .. }
                    | SipEvent::CallFailed { call_id, .. }
                    | SipEvent::CallCancelled { call_id } => call_id,
                    _ => continue,
                };
                me.on_sip_ended(&call_id).await;
            }
        });
        Ok(())
    }

    /// Subscribe the Connect adapter's event stream and BYE the SIP carrier when
    /// the Connect/agent leg ends (`Ended`/`Failed`) — the reverse direction.
    fn spawn_connect_end_watcher(self: &Arc<Self>) {
        let mut events = self.adapter.subscribe_events();
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                let connect_conn = match event {
                    AdapterEvent::Ended { connection_id, .. }
                    | AdapterEvent::Failed { connection_id, .. } => connection_id,
                    _ => continue,
                };
                me.on_connect_ended(&connect_conn).await;
            }
        });
    }

    /// SIP leg ended → LEAVE the Chime meeting. Claims teardown by removing the
    /// `active` entry (so the reverse watcher no-ops).
    async fn on_sip_ended(&self, sip_session: &SipSessionId) {
        if let Some((_, active)) = self.active.remove(sip_session) {
            self.by_connect.remove(&active.connect_conn);
            self.release_route_slot(&active);
            info!(session = %sip_session, "SIP leg ended — leaving Amazon Connect meeting");
            let _ = self
                .adapter
                .end(active.connect_conn, EndReason::Normal)
                .await;
            // Dropping `active` aborts the bridge pumps.
        }
    }

    /// Connect/agent leg ended → BYE the SIP carrier. Resolves the SIP session
    /// from the reverse index, then claims teardown via the same `active`
    /// removal so the two directions can't double-fire.
    async fn on_connect_ended(&self, connect_conn: &ConnectionId) {
        let Some(sip_session) = self.by_connect.get(connect_conn).map(|e| e.value().clone()) else {
            return;
        };
        if let Some((_, active)) = self.active.remove(&sip_session) {
            self.by_connect.remove(&active.connect_conn);
            self.release_route_slot(&active);
            info!(session = %sip_session, "Amazon Connect leg ended — hanging up SIP carrier (BYE)");
            // BYE the carrier.
            let _ = self.coordinator.hangup(&sip_session).await;
            // Release the adapter route + close the (already-ended) Chime peer.
            let _ = self
                .adapter
                .end(active.connect_conn, EndReason::Normal)
                .await;
        }
    }

    /// Tear down a bridged contact by SIP session (public manual teardown).
    pub async fn end(&self, sip_session: &SipSessionId) {
        self.on_sip_ended(sip_session).await;
    }

    /// Decrement the per-route active-session gauge for a torn-down contact.
    fn release_route_slot(&self, active: &ActiveContact) {
        if let Some(label) = &active.route_label {
            self.stats_for(label)
                .active_sessions
                .fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// User part of the INVITE Request-URI — the primary multi-tenant routing
/// key (CONTRACTS B.4: R-URI user, then To user, then default tenant).
pub fn request_uri_user(call: &IncomingCall) -> Option<String> {
    call.raw_request().and_then(|r| r.uri().user.clone())
}

/// User part of the To-header URI — the fallback routing key. Reads the
/// typed To header when the parsed request is available, else parses the
/// legacy `call.to` string.
pub fn to_uri_user(call: &IncomingCall) -> Option<String> {
    if let Some(to) = call.raw_request().and_then(|r| r.to()) {
        return to.0.uri.user.clone();
    }
    uri_user_part(&call.to).map(str::to_string)
}

/// Extract the user part from a SIP URI string, tolerating a display name,
/// angle brackets, and URI/header params: `"Bob" <sip:sales@x.y;tag=1>` →
/// `sales`. Returns `None` when there is no user part.
pub fn uri_user_part(uri: &str) -> Option<&str> {
    let s = uri.trim();
    // Strip a display name + angle brackets if present.
    let s = match (s.find('<'), s.find('>')) {
        (Some(open), Some(close)) if open < close => &s[open + 1..close],
        _ => s,
    };
    let s = s
        .strip_prefix("sips:")
        .or_else(|| s.strip_prefix("sip:"))
        .unwrap_or(s);
    let user = &s[..s.find('@')?];
    // Drop password / params that legally precede '@' only via ';'/':'.
    let user = user.split(|c| c == ':' || c == ';').next().unwrap_or(user);
    (!user.is_empty()).then_some(user)
}

#[cfg(test)]
mod tests {
    use super::uri_user_part;

    #[test]
    fn parses_plain_and_bracketed_uris() {
        assert_eq!(uri_user_part("sip:banking@10.0.0.1"), Some("banking"));
        assert_eq!(uri_user_part("sips:sales@example.com:5061"), Some("sales"));
        assert_eq!(
            uri_user_part("\"Vapi\" <sip:support@example.com;transport=udp>;tag=abc"),
            Some("support")
        );
        assert_eq!(uri_user_part("<sip:a@b>"), Some("a"));
    }

    #[test]
    fn strips_password_and_uri_params_from_user() {
        assert_eq!(uri_user_part("sip:bob:secret@example.com"), Some("bob"));
        assert_eq!(uri_user_part("sip:bob;p=1@example.com"), Some("bob"));
    }

    #[test]
    fn no_user_part_yields_none() {
        assert_eq!(uri_user_part("sip:10.0.0.1"), None);
        assert_eq!(uri_user_part("sip:@example.com"), None);
        assert_eq!(uri_user_part(""), None);
        assert_eq!(uri_user_part("tel:+14155550100"), None);
    }
}

/// Pull every custom (`Other`) header off the inbound INVITE as
/// `(name, value)` pairs, preserving original-case names and clean values
/// (`raw_header_value`, not the `"Name: value"` `Display`). Falls back to the
/// legacy `headers` map (lowercased keys, `"Name: value"` values stripped) when
/// the parsed request is unavailable.
fn extract_headers(call: &IncomingCall) -> Vec<(String, String)> {
    if let Some(req) = call.raw_request() {
        let mut out = Vec::new();
        for hdr in &req.headers {
            if let TypedHeader::Other(name @ HeaderName::Other(key), _) = hdr {
                if let Some(value) = req.raw_header_value(name) {
                    out.push((key.clone(), value));
                }
            }
        }
        return out;
    }
    // Legacy fallback: values are "Name: value"; strip the prefix.
    call.headers
        .iter()
        .map(|(k, v)| {
            let value = v.splitn(2, ": ").nth(1).unwrap_or(v).to_string();
            (k.clone(), value)
        })
        .collect()
}
