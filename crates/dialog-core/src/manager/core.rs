//! Core Dialog Manager Implementation
//!
//! This module contains the main DialogManager struct and its core lifecycle methods.
//! It serves as the central coordinator for SIP dialog management.

use dashmap::DashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::transaction::{TransactionEvent, TransactionKey, TransactionManager};
use rvoip_sip_core::{Method, Request, Response};

use crate::config::DialogManagerConfig;
use crate::dialog::{Dialog, DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::{DialogEvent, FlowFailureReason, SessionCoordinationEvent};
use crate::manager::outbound_flow::OutboundFlow;
use crate::subscription::SubscriptionManager;

#[derive(Debug, Clone)]
pub struct DialogManager {
    /// Reference to transaction manager (handles transport for us)
    pub(crate) transaction_manager: Arc<TransactionManager>,

    /// Local address for this dialog manager (used in Via headers)
    pub(crate) local_address: SocketAddr,

    /// **NEW**: Optional unified configuration for behavioral modes
    /// When present, enables mode-specific behavior (auto-responses, etc.).
    ///
    /// Wrapped in `Arc<RwLock<...>>` so that `set_config` propagates to every
    /// `DialogManager` clone — notably the background event-processor task
    /// spawned during construction, which otherwise would never see the
    /// config set later by `UnifiedDialogManager` (RFC 3262 420 + RFC 4028
    /// negotiation both rely on this config on the incoming-request path).
    pub(crate) config: Arc<std::sync::RwLock<Option<DialogManagerConfig>>>,

    /// Active dialogs by dialog ID
    pub(crate) dialogs: Arc<DashMap<DialogId, Dialog>>,

    /// Dialog lookup by call-id + tags (key: "call-id:local-tag:remote-tag")
    pub(crate) dialog_lookup: Arc<DashMap<String, DialogId>>,

    /// Transaction to dialog mapping
    pub(crate) transaction_to_dialog: Arc<DashMap<TransactionKey, DialogId>>,

    /// Session to dialog mapping for cross-crate coordination
    pub(crate) session_to_dialog: Arc<DashMap<String, DialogId>>,

    /// Dialog to session mapping
    pub(crate) dialog_to_session: Arc<DashMap<DialogId, String>>,

    /// Event hub for global event coordination
    pub(crate) event_hub: Arc<tokio::sync::RwLock<Option<Arc<crate::events::DialogEventHub>>>>,

    /// Channel for sending session coordination events to session-core
    pub(crate) session_coordinator:
        Arc<tokio::sync::RwLock<Option<mpsc::Sender<SessionCoordinationEvent>>>>,

    /// Channel for sending dialog events to external consumers (session-core)
    pub(crate) dialog_event_sender: Arc<tokio::sync::RwLock<Option<mpsc::Sender<DialogEvent>>>>,

    /// Channel for receiving dialog events (for shutdown coordination)
    pub(crate) dialog_event_receiver: Arc<tokio::sync::RwLock<Option<mpsc::Receiver<DialogEvent>>>>,

    /// Shutdown signal for global event processor
    pub(crate) shutdown_signal: Arc<tokio::sync::Notify>,

    /// Subscription manager for handling SUBSCRIBE/NOTIFY
    pub(crate) subscription_manager: Option<Arc<SubscriptionManager>>,

    /// Abort handles for in-flight UAS reliable-provisional retransmit tasks
    /// (RFC 3262 §3). Keyed by `(dialog_id, rseq)`. On PRACK arrival the
    /// matching entry is removed and aborted so the 18x stops retransmitting;
    /// on dialog termination every entry for that dialog is aborted.
    pub(crate) reliable_provisional_tasks: Arc<DashMap<(DialogId, u32), tokio::task::AbortHandle>>,

    /// Abort handles for per-dialog RFC 4028 session-timer refresh tasks.
    /// Populated when the UAC or UAS is designated refresher; one entry per
    /// dialog. Aborted on dialog termination.
    pub(crate) session_refresh_tasks: Arc<DashMap<DialogId, tokio::task::AbortHandle>>,

    /// Discovered public address from RFC 3581 `received=` / `rport=`
    /// echoed back on responses.
    ///
    /// On every inbound response we peek at the top `Via` header; when
    /// it carries `received=<ip>` plus a populated `rport=<port>` (set
    /// because we put `;rport` on the outgoing Via per RFC 3581 §4),
    /// we treat that as our externally-visible address as observed by
    /// the immediate hop. This lets a UA behind NAT discover its
    /// public address without STUN, then advertise it in subsequent
    /// `Contact:` headers (RFC 5626 §5).
    ///
    /// `None` until the first qualifying response arrives. Most-
    /// recent observation wins — if multiple peers see us through
    /// different NAT mappings, the latest update is authoritative.
    /// (Per-peer mapping would be a richer model; not yet justified
    /// by real-world traffic.)
    pub(crate) nat_discovered_addr: Arc<tokio::sync::RwLock<Option<SocketAddr>>>,

    /// Registrar-returned Service-Route (RFC 3608) keyed by AoR.
    ///
    /// Populated on successful REGISTER 2xx responses: the registrar
    /// echoes the ordered list of URIs that the UA MUST pre-load as
    /// Route headers for subsequent out-of-dialog requests within the
    /// registration binding. The key is the AoR (To URI, which for a
    /// UAC-originated REGISTER equals the From URI) normalized to its
    /// string form.
    ///
    /// Most recent REGISTER 2xx wins per AoR. Empty `Vec` means "we
    /// saw a REGISTER 2xx without Service-Route" (distinct from "no
    /// registration yet"); callers that care about the distinction
    /// should use `service_route_for_aor` and match on `None`.
    pub(crate) service_route_by_aor: Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<String, Vec<rvoip_sip_core::types::uri::Uri>>,
        >,
    >,

    /// Registrar-assigned GRUU URIs (RFC 5627 §5.3) keyed by AoR.
    ///
    /// Populated on successful REGISTER 2xx responses when the
    /// registrar echoes the Contact with `pub-gruu="..."` and/or
    /// `temp-gruu="..."` parameters. Most recent REGISTER 2xx wins
    /// per AoR. `None` from
    /// [`Self::gruu_for_aor`](crate::manager::DialogManager::gruu_for_aor)
    /// means "no REGISTER 2xx with GRUU observed yet" (distinct from
    /// "registrar didn't assign a GRUU on this binding"). A registrar
    /// may assign only `pub-gruu` or only `temp-gruu` — the cached
    /// `GruuContactParams` carries `Option`s for each independently.
    pub(crate) gruu_by_aor: Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<String, rvoip_sip_core::types::outbound::GruuContactParams>,
        >,
    >,

    /// RFC 5626 §3.5.1 outbound-flow state machines, keyed by
    /// `(AoR, reg-id, instance-id)` per RFC 5626 §4.2.
    ///
    /// Each successful outbound-aware REGISTER 2xx spawns one task (see
    /// [`Self::outbound_flow_tasks`]) that pings every
    /// [`outbound_keepalive_interval`](Self::outbound_keepalive_interval)
    /// and monitors the pong window. A pong timeout, `ConnectionClosed`
    /// event, or send error flips the [`OutboundFlow`] into
    /// `FlowState::Failed` and emits a
    /// [`SessionCoordinationEvent::OutboundFlowFailed`] (once) so
    /// session-core can trigger a fresh REGISTER (RFC 5626 §4.4.1).
    ///
    /// Idempotent: starting a flow for a key that already has one stops
    /// the prior flow first.
    pub(crate) outbound_flows: Arc<DashMap<(String, u32, String), Arc<OutboundFlow>>>,

    /// Abort handles for the spawned ping/monitor task of each entry in
    /// [`Self::outbound_flows`]. Split from the flow state so the state
    /// can be inspected (e.g. by pong/close handlers) without touching
    /// the task handle.
    pub(crate) outbound_flow_tasks: Arc<DashMap<(String, u32, String), tokio::task::AbortHandle>>,

    /// Secondary index mapping destination `SocketAddr` →
    /// `(aor, reg_id, instance)` flow keys, populated when
    /// `start_outbound_ping` installs a flow. Lets transport-side events
    /// (`KeepAlivePongReceived`, `ConnectionClosed`) — which arrive
    /// keyed only by IP:port — locate the flow(s) to update in O(1).
    pub(crate) flow_by_destination: Arc<DashMap<SocketAddr, Vec<(String, u32, String)>>>,

    /// Keep-alive interval for RFC 5626 outbound flows, threaded from
    /// `session-core::Config::outbound_keepalive_interval_secs`. `None`
    /// disables keep-alive entirely — `start_outbound_ping` becomes a
    /// no-op.
    pub(crate) outbound_keepalive_interval: Arc<std::sync::RwLock<Option<std::time::Duration>>>,
}

impl DialogManager {
    /// Create a new dialog manager
    ///
    /// **ARCHITECTURE**: dialog-core receives TransactionManager via dependency injection.
    /// The application level is responsible for creating the transaction layer.
    ///
    /// # Arguments
    /// * `transaction_manager` - The transaction manager to use for SIP message reliability
    /// * `local_address` - The local address to use in Via headers and Contact headers
    ///
    /// # Returns
    /// A new DialogManager instance ready for use
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        local_address: SocketAddr,
    ) -> DialogResult<Self> {
        info!(
            "Creating new DialogManager with local address {}",
            local_address
        );

        // Create shared stores
        let dialogs = Arc::new(DashMap::new());
        let dialog_lookup = Arc::new(DashMap::new());

        // Create dialog event channel for subscription manager
        let (event_tx, _) = mpsc::channel(100);

        // Create subscription manager with shared stores
        let subscription_manager =
            SubscriptionManager::new(dialogs.clone(), dialog_lookup.clone(), event_tx);

        Ok(Self {
            transaction_manager,
            local_address,
            config: Arc::new(std::sync::RwLock::new(None)),
            dialogs,
            dialog_lookup,
            transaction_to_dialog: Arc::new(DashMap::new()),
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            event_hub: Arc::new(tokio::sync::RwLock::new(None)),
            session_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
            dialog_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            dialog_event_receiver: Arc::new(tokio::sync::RwLock::new(None)),
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
            subscription_manager: Some(Arc::new(subscription_manager)),
            reliable_provisional_tasks: Arc::new(DashMap::new()),
            session_refresh_tasks: Arc::new(DashMap::new()),
            nat_discovered_addr: Arc::new(tokio::sync::RwLock::new(None)),
            service_route_by_aor: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            gruu_by_aor: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            outbound_flows: Arc::new(DashMap::new()),
            outbound_flow_tasks: Arc::new(DashMap::new()),
            flow_by_destination: Arc::new(DashMap::new()),
            outbound_keepalive_interval: Arc::new(std::sync::RwLock::new(None)),
        })
    }

    /// Configure the RFC 5626 §3.5.1 keep-alive interval for this
    /// DialogManager. `None` (or not calling this at all) disables
    /// outbound keep-alive; subsequent REGISTER 2xx responses will not
    /// spawn ping tasks. The session-core coordinator wires this from
    /// its `outbound_keepalive_interval_secs` config at boot.
    pub fn set_outbound_keepalive_interval(&self, interval: Option<std::time::Duration>) {
        if let Ok(mut guard) = self.outbound_keepalive_interval.write() {
            *guard = interval;
        }
    }

    /// Read the currently-configured RFC 5626 keep-alive interval.
    pub fn outbound_keepalive_interval(&self) -> Option<std::time::Duration> {
        self.outbound_keepalive_interval
            .read()
            .ok()
            .and_then(|g| *g)
    }

    /// Spawn (or replace) a RFC 5626 §3.5.1 CRLFCRLF keep-alive flow
    /// targeting `destination` via the DialogManager's transport.
    ///
    /// `flow_key = (AoR, reg-id, instance-id)` is the outbound flow
    /// identity per RFC 5626 §4.2; a second call for the same key
    /// stops the prior flow first (idempotent refresh on re-REGISTER).
    ///
    /// Phase 2c: the spawned task drives an [`OutboundFlow`] state
    /// machine — after each ping it arms a pong deadline, and on
    /// pong-timeout / connection-closed / send-error it emits a single
    /// [`SessionCoordinationEvent::OutboundFlowFailed`] so session-core
    /// can trigger a fresh REGISTER without waiting for registration
    /// expiry.
    ///
    /// No-op when `outbound_keepalive_interval` is `None`.
    pub fn start_outbound_ping(&self, flow_key: (String, u32, String), destination: SocketAddr) {
        let Some(interval) = self.outbound_keepalive_interval() else {
            return;
        };
        if interval.is_zero() {
            return;
        }

        // Replace any prior flow for this key (idempotent on re-REGISTER).
        self.stop_outbound_ping(&flow_key);

        let flow = Arc::new(OutboundFlow::new(flow_key.clone(), destination, interval));
        let transport = self.transaction_manager.transport().clone();
        let manager = self.clone();
        let flow_for_task = flow.clone();

        let handle = tokio::spawn(async move {
            run_outbound_flow_loop(manager, flow_for_task, transport).await;
        })
        .abort_handle();

        self.outbound_flows.insert(flow_key.clone(), flow);
        self.outbound_flow_tasks.insert(flow_key.clone(), handle);
        // Secondary index for transport-event lookups by destination.
        self.flow_by_destination
            .entry(destination)
            .or_insert_with(Vec::new)
            .push(flow_key);
    }

    /// Stop (and forget) the RFC 5626 keep-alive flow for this key, if
    /// any. Aborts the monitor task and tears down both the primary
    /// flow map and the destination secondary index. Does **not** emit
    /// an `OutboundFlowFailed` event — explicit teardown is not a flow
    /// failure; callers that want the failure event must call
    /// `mark_failed` on the `OutboundFlow` first.
    pub fn stop_outbound_ping(&self, flow_key: &(String, u32, String)) {
        if let Some((_, handle)) = self.outbound_flow_tasks.remove(flow_key) {
            handle.abort();
        }
        if let Some((_, flow)) = self.outbound_flows.remove(flow_key) {
            if let Some(mut entry) = self.flow_by_destination.get_mut(&flow.destination) {
                entry.value_mut().retain(|k| k != flow_key);
            }
            // Drop empty destination buckets so the secondary index
            // doesn't leak entries across many reg/dereg cycles.
            self.flow_by_destination
                .remove_if(&flow.destination, |_, v| v.is_empty());
        }
    }

    /// Transport reported `KeepAlivePongReceived` from `source`. Update
    /// every outbound flow that's aimed at that peer so the pong is
    /// treated as an answer to the in-flight ping (if any). No-op when
    /// no flow is registered for the address.
    pub async fn on_pong_received(&self, source: SocketAddr) {
        let keys: Vec<(String, u32, String)> = match self.flow_by_destination.get(&source) {
            Some(entry) => entry.value().clone(),
            None => return,
        };
        for key in keys {
            if let Some(flow) = self.outbound_flows.get(&key).map(|e| e.value().clone()) {
                flow.on_pong().await;
                tracing::trace!(
                    flow_key = ?key, src = %source,
                    "RFC 5626 pong received — flow reset to Idle"
                );
            }
        }
    }

    /// Transport reported `ConnectionClosed` to `remote_addr`. Every
    /// outbound flow aimed at that peer is marked failed (once), emits
    /// an `OutboundFlowFailed` event with `ConnectionClosed` reason,
    /// and has its monitor task torn down. The peer reconnect is
    /// session-core's problem (trigger re-REGISTER) — dialog-core only
    /// reports the flow death.
    pub async fn on_connection_closed(&self, remote_addr: SocketAddr) {
        let keys: Vec<(String, u32, String)> = match self.flow_by_destination.get(&remote_addr) {
            Some(entry) => entry.value().clone(),
            None => return,
        };
        for key in keys {
            let flow = match self.outbound_flows.get(&key).map(|e| e.value().clone()) {
                Some(f) => f,
                None => continue,
            };
            if flow.mark_failed().await {
                tracing::info!(
                    flow_key = ?key, dest = %remote_addr,
                    "RFC 5626 connection closed — flow failed"
                );
                self.emit_outbound_flow_failed(&flow, FlowFailureReason::ConnectionClosed)
                    .await;
            }
            // Explicit stop — the monitor task would have exited on its
            // own once it noticed `Failed`, but we don't need to wait.
            self.stop_outbound_ping(&key);
        }
    }

    /// Emit `SessionCoordinationEvent::OutboundFlowFailed` for a flow
    /// that just transitioned to `FlowState::Failed`. Callers are
    /// responsible for the idempotency check — only the thread that
    /// observed `mark_failed() == true` should call this.
    pub(crate) async fn emit_outbound_flow_failed(
        &self,
        flow: &OutboundFlow,
        reason: FlowFailureReason,
    ) {
        let (aor, reg_id, instance) = flow.key.clone();
        self.emit_session_coordination_event(SessionCoordinationEvent::OutboundFlowFailed {
            aor,
            reg_id,
            instance,
            reason,
        })
        .await;
    }

    /// Create a new dialog manager with global transaction events (RECOMMENDED)
    ///
    /// This constructor follows the working pattern from transaction-core examples
    /// by receiving global transaction events for proper event consumption.
    ///
    /// # Arguments
    /// * `transaction_manager` - The transaction manager to use for SIP message reliability
    /// * `transaction_events` - Global transaction event receiver
    /// * `local_address` - The local address to use in Via headers and Contact headers
    ///
    /// # Returns
    /// A new DialogManager instance with proper event consumption
    pub async fn with_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        local_address: SocketAddr,
    ) -> DialogResult<Self> {
        info!(
            "Creating new DialogManager with global transaction events and local address {}",
            local_address
        );

        // Create shared stores
        let dialogs = Arc::new(DashMap::new());
        let dialog_lookup = Arc::new(DashMap::new());

        // Create dialog event channel for subscription manager
        let (event_tx, _) = mpsc::channel(100);

        // Create subscription manager with shared stores
        let subscription_manager =
            SubscriptionManager::new(dialogs.clone(), dialog_lookup.clone(), event_tx);

        let manager = Self {
            transaction_manager,
            local_address,
            config: Arc::new(std::sync::RwLock::new(None)),
            dialogs,
            dialog_lookup,
            transaction_to_dialog: Arc::new(DashMap::new()),
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            event_hub: Arc::new(tokio::sync::RwLock::new(None)),
            session_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
            dialog_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            dialog_event_receiver: Arc::new(tokio::sync::RwLock::new(None)),
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
            subscription_manager: Some(Arc::new(subscription_manager)),
            reliable_provisional_tasks: Arc::new(DashMap::new()),
            session_refresh_tasks: Arc::new(DashMap::new()),
            nat_discovered_addr: Arc::new(tokio::sync::RwLock::new(None)),
            service_route_by_aor: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            gruu_by_aor: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            outbound_flows: Arc::new(DashMap::new()),
            outbound_flow_tasks: Arc::new(DashMap::new()),
            flow_by_destination: Arc::new(DashMap::new()),
            outbound_keepalive_interval: Arc::new(std::sync::RwLock::new(None)),
        };

        // Spawn global transaction event processor
        let event_processor = manager.clone();
        tokio::spawn(async move {
            event_processor
                .process_global_transaction_events(transaction_events)
                .await;
        });

        // Wire up the RFC 5626 flow-event channel: the transaction
        // manager forwards transport-side pong + connection-closed
        // events into `flow_rx`; a dedicated consumer task then drives
        // the per-flow state machines in the dialog manager. The
        // channel is modest (64) — one event per flow per ping or
        // close, and the consumer is lightweight.
        let (flow_tx, mut flow_rx) =
            mpsc::channel::<crate::manager::outbound_flow::FlowTransportEvent>(64);
        manager
            .transaction_manager
            .set_flow_event_sender(flow_tx)
            .await;
        let flow_consumer = manager.clone();
        tokio::spawn(async move {
            while let Some(event) = flow_rx.recv().await {
                match event {
                    crate::manager::outbound_flow::FlowTransportEvent::PongReceived { source } => {
                        flow_consumer.on_pong_received(source).await;
                    }
                    crate::manager::outbound_flow::FlowTransportEvent::ConnectionClosed {
                        remote_addr,
                    } => {
                        flow_consumer.on_connection_closed(remote_addr).await;
                    }
                }
            }
            debug!("RFC 5626 flow-event consumer channel closed");
        });

        Ok(manager)
    }

    /// Process global transaction events (similar to working transaction-core examples)
    ///
    /// This follows the exact pattern from working examples that use global event consumption
    /// instead of individual transaction subscriptions.
    async fn process_global_transaction_events(
        &self,
        mut events: mpsc::Receiver<TransactionEvent>,
    ) {
        info!("🔄 Starting global transaction event processor for dialog-core");

        loop {
            tokio::select! {
                // Process transaction events
                event = events.recv() => {
                    match event {
                        Some(event) => {
                            // Extract transaction ID from the event
                            let transaction_id = self.extract_transaction_id(&event);

                            // Find the dialog associated with this transaction
                            if let Some(dialog_id) = self.find_dialog_for_transaction_event(&transaction_id) {
                                if let Err(e) = self.process_transaction_event(&transaction_id, &dialog_id, event).await {
                                    error!("Failed to process transaction event for dialog {}: {}", dialog_id, e);
                                }
                            } else {
                                // No dialog found using transaction-to-dialog mapping

                                // Special handling for AckReceived events: use dialog-based matching
                                if let TransactionEvent::AckReceived { request, .. } = &event {
                                    // Find dialog using Call-ID, From tag, To tag from the ACK request
                                    if let Some(dialog_id) = self.find_dialog_for_request(request).await {
                                        if let Err(e) = self.process_transaction_event(&transaction_id, &dialog_id, event).await {
                                            error!("Failed to process AckReceived event for dialog {}: {}", dialog_id, e);
                                        }
                                    } else {
                                        // Still treat as unassociated event
                                        if let Err(e) = self.handle_unassociated_transaction_event(&transaction_id, event).await {
                                            error!("Failed to handle unassociated AckReceived event {}: {}", transaction_id, e);
                                        }
                                    }
                                } else {
                                    // Event for transaction not associated with any dialog
                                    // Check if this is a new incoming INVITE that should create a dialog
                                    if let Err(e) = self.handle_unassociated_transaction_event(&transaction_id, event).await {
                                        error!("Failed to handle unassociated transaction event {}: {}", transaction_id, e);
                                    }
                                }
                            }
                        },
                        None => {
                            // Channel closed
                            debug!("Transaction events channel closed");
                            break;
                        }
                    }
                },

                // Wait for shutdown signal
                _ = self.shutdown_signal.notified() => {
                    info!("🛑 Global transaction event processor received shutdown signal");
                    break;
                }
            }
        }

        info!("🏁 Global transaction event processor for dialog-core stopped");
    }

    /// Extract transaction ID from any TransactionEvent variant
    fn extract_transaction_id(&self, event: &TransactionEvent) -> TransactionKey {
        match event {
            TransactionEvent::AckReceived { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::CancelReceived { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::ProvisionalResponse { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::SuccessResponse { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::FailureResponse { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::ProvisionalResponseSent { transaction_id, .. } => {
                transaction_id.clone()
            }
            TransactionEvent::FinalResponseSent { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::TransactionTimeout { transaction_id } => transaction_id.clone(),
            TransactionEvent::AckTimeout { transaction_id } => transaction_id.clone(),
            TransactionEvent::TransportError { transaction_id } => transaction_id.clone(),
            TransactionEvent::Error { transaction_id, .. } => transaction_id
                .clone()
                .unwrap_or_else(|| TransactionKey::new("unknown".to_string(), Method::Info, false)),
            TransactionEvent::TransactionTerminated { transaction_id } => transaction_id.clone(),
            TransactionEvent::StateChanged { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::TimerTriggered { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::CancelRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::AckRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::InviteRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::NonInviteRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::StrayRequest { .. } => {
                TransactionKey::new("stray".to_string(), Method::Info, false)
            }
            TransactionEvent::StrayResponse { .. } => {
                TransactionKey::new("stray".to_string(), Method::Info, false)
            }
            TransactionEvent::StrayAck { .. } => {
                TransactionKey::new("stray".to_string(), Method::Info, false)
            }
            TransactionEvent::StrayCancel { .. } => {
                TransactionKey::new("stray".to_string(), Method::Info, false)
            }
            TransactionEvent::StrayAckRequest { .. } => {
                TransactionKey::new("stray".to_string(), Method::Info, false)
            }

            // Shutdown events don't have transaction IDs
            TransactionEvent::ShutdownRequested
            | TransactionEvent::ShutdownReady
            | TransactionEvent::ShutdownNow
            | TransactionEvent::ShutdownComplete => {
                TransactionKey::new("shutdown".to_string(), Method::Info, false)
            }
        }
    }

    /// Find dialog associated with a transaction event
    fn find_dialog_for_transaction_event(
        &self,
        transaction_id: &TransactionKey,
    ) -> Option<DialogId> {
        self.transaction_to_dialog
            .get(transaction_id)
            .map(|entry| entry.clone())
    }

    /// Handle transaction events not associated with any existing dialog
    ///
    /// This handles new incoming requests that should create dialogs.
    async fn handle_unassociated_transaction_event(
        &self,
        transaction_id: &TransactionKey,
        event: TransactionEvent,
    ) -> DialogResult<()> {
        match event {
            TransactionEvent::InviteRequest {
                request, source, ..
            } => {
                // RFC 3261 §14: an INVITE on an existing dialog is a
                // re-INVITE. Every inbound INVITE spins up a fresh server
                // transaction, so the transaction-to-dialog mapping is
                // always empty at this point. We must dialog-match on
                // (Call-ID, From-tag, To-tag) before falling through to
                // initial INVITE handling. Same pattern as the REFER arm
                // below.
                if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
                    debug!(
                        "INVITE request belongs to existing dialog {} — treating as re-INVITE",
                        dialog_id
                    );
                    self.handle_reinvite(transaction_id.clone(), request, dialog_id)
                        .await?;
                    return Ok(());
                }

                tracing::debug!(
                    "🎯 FOUND UNASSOCIATED INVITE: Processing new incoming INVITE from {}",
                    source
                );
                debug!(
                    "Processing new incoming INVITE request from transaction {}",
                    transaction_id
                );

                // This is a new incoming INVITE - create dialog and process it
                self.handle_initial_invite(transaction_id.clone(), request, source)
                    .await?;

                debug!("Successfully processed new incoming INVITE from {}", source);
                Ok(())
            }

            TransactionEvent::NonInviteRequest {
                request, source, ..
            } => {
                debug!(
                    "Processing new incoming {} request from transaction {}",
                    request.method(),
                    transaction_id
                );

                // For REFER requests, check if they belong to an existing dialog
                if request.method() == Method::Refer {
                    // Try to find the dialog using Call-ID, From tag, and To tag
                    if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
                        debug!("REFER request belongs to existing dialog {}", dialog_id);

                        // Store the transaction-to-dialog mapping
                        self.transaction_to_dialog
                            .insert(transaction_id.clone(), dialog_id.clone());

                        // REFER within a dialog should be handled by the protocol handler
                        // which will emit the TransferRequest event to session-core
                        return self.handle_refer(request, source).await;
                    } else {
                        debug!("REFER request does not match any existing dialog");
                    }
                }

                // Handle non-INVITE requests (REGISTER, OPTIONS, etc.) or REFER without dialog
                self.handle_request(request, source).await
            }

            // UAS-side CANCEL. The transaction manager emits this when an
            // inbound CANCEL finds a matching INVITE server transaction.
            // The CANCEL request itself has no dialog mapping of its own,
            // so it arrives here as "unassociated" — route it to the
            // protocol handler so we send 200 OK to CANCEL, 487 to the
            // pending INVITE, and terminate the dialog.
            TransactionEvent::CancelRequest {
                request, source, ..
            } => {
                debug!(
                    "Processing unassociated CANCEL request from transaction {}",
                    transaction_id
                );
                self.handle_cancel(request).await
            }

            _ => {
                // Other unassociated events (responses, timeouts, etc.) - just log them
                debug!("Received unassociated transaction event: {:?}", event);
                Ok(())
            }
        }
    }

    /// Get the configured local address
    ///
    /// Returns the local address that this DialogManager uses for Via headers
    /// and Contact headers when creating SIP requests.
    pub fn local_address(&self) -> SocketAddr {
        self.local_address
    }

    /// Configured local Contact URI, if the application supplied one.
    pub fn local_contact_uri(&self) -> Option<String> {
        self.config.read().ok().and_then(|g| {
            g.as_ref()
                .and_then(|c| c.local_contact_uri().map(str::to_string))
        })
    }

    // REMOVED: set_session_coordinator() - Use GlobalEventCoordinator instead
    // REMOVED: set_dialog_event_sender() - Use GlobalEventCoordinator instead
    // REMOVED: setup_dialog_event_channel() - Use GlobalEventCoordinator instead
    // REMOVED: process_dialog_events() and handle_shutdown_requested() - Use GlobalEventCoordinator instead
    // REMOVED: subscribe_to_dialog_events() - Use GlobalEventCoordinator instead

    /// Emit a dialog event to external consumers
    ///
    /// Sends dialog events to session-core for high-level dialog state management.
    /// This maintains the proper architectural separation where dialog-core handles
    /// SIP protocol details and session-core handles session logic.
    pub async fn emit_dialog_event(&self, event: DialogEvent) {
        // Try event hub first (new global event bus)
        if let Some(hub) = self.event_hub.read().await.as_ref() {
            if let Err(e) = hub.publish_dialog_event(event.clone()).await {
                warn!("Failed to publish dialog event to global bus: {}", e);
            } else {
                debug!("Published dialog event to global bus: {:?}", event);
                return;
            }
        }

        // Fall back to channel (legacy)
        if let Some(sender) = self.dialog_event_sender.read().await.as_ref() {
            if let Err(e) = sender.send(event.clone()).await {
                warn!("Failed to send dialog event to session-core: {}", e);
            } else {
                debug!("Emitted dialog event: {:?}", event);
            }
        }
    }

    /// Emit a session coordination event
    ///
    /// Sends session coordination events for legacy compatibility and specific
    /// session management operations.
    pub async fn emit_session_coordination_event(&self, event: SessionCoordinationEvent) {
        info!(
            "📤 emit_session_coordination_event called with event: {:?}",
            event
        );

        // Try event hub first (new global event bus)
        if let Some(hub) = self.event_hub.read().await.as_ref() {
            info!("📤 Event hub exists, publishing to global bus");
            if let Err(e) = hub.publish_session_coordination_event(event.clone()).await {
                warn!(
                    "Failed to publish session coordination event to global bus: {}",
                    e
                );
            } else {
                info!(
                    "📤 Published session coordination event to global bus: {:?}",
                    event
                );
                return;
            }
        } else {
            info!("📤 Event hub is None, trying legacy channel");
        }

        // Fall back to channel (legacy)
        if let Some(sender) = self.session_coordinator.read().await.as_ref() {
            info!("📤 Legacy channel exists, sending event");
            if let Err(e) = sender.send(event.clone()).await {
                warn!("Failed to send session coordination event: {}", e);
            } else {
                info!(
                    "📤 Emitted session coordination event to legacy channel: {:?}",
                    event
                );
            }
        } else {
            warn!("📤 Both event hub and legacy channel are None - event not sent!");
        }
    }

    /// Try to emit a session coordination event and report whether any session
    /// consumer path accepted it. This is intentionally narrower than
    /// `emit_session_coordination_event`: protocol handlers that need a
    /// definite answer can use this to choose a local fallback, while existing
    /// fire-and-forget event paths keep their current behavior.
    pub(crate) async fn try_emit_session_coordination_event(
        &self,
        event: SessionCoordinationEvent,
    ) -> DialogResult<bool> {
        if let Some(hub) = self.event_hub.read().await.as_ref() {
            match hub
                .try_publish_session_coordination_event(event.clone())
                .await
            {
                Ok(true) => return Ok(true),
                Ok(false) => {
                    debug!(
                        "Session coordination event did not map to a cross-crate event: {:?}",
                        event
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to publish session coordination event to global bus: {}",
                        e
                    );
                }
            }
        }

        if let Some(sender) = self.session_coordinator.read().await.as_ref() {
            match sender.send(event.clone()).await {
                Ok(()) => return Ok(true),
                Err(e) => {
                    warn!("Failed to send session coordination event: {}", e);
                }
            }
        }

        Ok(false)
    }

    /// **CENTRAL DISPATCHER**: Handle incoming SIP messages
    ///
    /// This is the main entry point for processing SIP messages in dialog-core.
    /// It routes messages to the appropriate method-specific handlers while maintaining
    /// RFC 3261 compliance for dialog state management.
    ///
    /// # Arguments
    /// * `message` - The SIP message (Request or Response)
    /// * `source` - Source address of the message
    ///
    /// # Returns
    /// Result indicating success or the specific error encountered
    pub async fn handle_message(
        &self,
        message: rvoip_sip_core::Message,
        source: SocketAddr,
    ) -> DialogResult<()> {
        match message {
            rvoip_sip_core::Message::Request(request) => self.handle_request(request, source).await,
            rvoip_sip_core::Message::Response(_response) => {
                // For responses, we need the transaction ID to route properly
                // This would typically come from the transaction layer
                warn!("Response handling requires transaction ID - use handle_response() directly");
                Err(DialogError::protocol_error(
                    "Response handling requires transaction context",
                ))
            }
        }
    }

    /// Handle incoming SIP requests
    ///
    /// Routes requests to appropriate method handlers based on the SIP method.
    /// Implements RFC 3261 Section 12 dialog handling requirements.
    ///
    /// # Arguments
    /// * `request` - The SIP request to handle
    /// * `source` - Source address of the request
    async fn handle_request(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Handling {} request from {}", request.method(), source);

        // Dispatch request to appropriate handler based on method
        match request.method() {
            Method::Invite => self.handle_invite(request, source).await,
            Method::Bye => self.handle_bye(request).await,
            Method::Cancel => self.handle_cancel(request).await,
            Method::Ack => self.handle_ack(request).await,
            Method::Options => self.handle_options(request, source).await,
            Method::Register => self.handle_register(request, source).await,
            Method::Update => self.handle_update(request).await,
            Method::Info => self.handle_info(request, source).await,
            Method::Refer => self.handle_refer(request, source).await,
            Method::Subscribe => self.handle_subscribe(request, source).await,
            Method::Notify => self.handle_notify(request, source).await,
            Method::Prack => self.handle_prack(request).await,
            method => {
                warn!("Unsupported SIP method: {}", method);
                Err(DialogError::protocol_error(&format!(
                    "Unsupported method: {}",
                    method
                )))
            }
        }
    }

    /// Start the dialog manager
    ///
    /// Initializes the dialog manager for processing. This can include starting
    /// background tasks for dialog cleanup, recovery, and maintenance.
    pub async fn start(&self) -> DialogResult<()> {
        info!("DialogManager starting");

        // TODO: Start background processing tasks (cleanup, recovery, etc.)
        // - Dialog timeout monitoring
        // - Orphaned dialog cleanup
        // - Recovery coordination
        // - Statistics collection

        info!("DialogManager started successfully");
        Ok(())
    }

    /// Stop the dialog manager
    ///
    /// Gracefully shuts down the dialog manager in BOTTOM-UP order
    /// This is called when receiving ShutdownNow("DialogManager") event
    ///
    /// Shutdown order (bottom-up):
    /// 1. Shutdown transaction manager (which has already stopped transport)
    /// 2. Signal global event processor to stop
    /// 3. Terminate any remaining dialogs
    /// 4. Clear internal state
    /// 5. Report completion via event
    pub async fn stop(&self) -> DialogResult<()> {
        info!("DialogManager stopping gracefully - responding to shutdown event");

        // Step 0: Abort all RFC 5626 outbound-flow monitor tasks so
        // they don't try to emit `OutboundFlowFailed` against a
        // transport that's about to be torn down.
        let flow_keys: Vec<(String, u32, String)> = self
            .outbound_flow_tasks
            .iter()
            .map(|e| e.key().clone())
            .collect();
        for key in flow_keys {
            self.stop_outbound_ping(&key);
        }

        // Step 1: Shutdown the transaction manager
        // Note: Transport should already be stopped by now via events
        info!("Shutting down transaction manager...");
        self.transaction_manager.shutdown().await;
        debug!("Transaction manager shut down");

        // Step 2: Signal shutdown to global event processor
        self.shutdown_signal.notify_one();
        debug!("Sent shutdown signal to global event processor");

        // Give event processor time to process final messages
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Step 3: Now terminate any remaining dialogs
        let dialog_ids: Vec<DialogId> = self
            .dialogs
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        if !dialog_ids.is_empty() {
            debug!("Found {} remaining dialogs to clean up", dialog_ids.len());
            for dialog_id in dialog_ids {
                if let Some(_) = self.dialogs.remove(&dialog_id) {
                    debug!("Removed dialog {}", dialog_id);
                }
            }
        }

        // Step 4: Clear all mappings
        self.dialogs.clear();
        self.dialog_lookup.clear();
        self.transaction_to_dialog.clear();

        // Step 5: Report completion
        // Since we're in dialog-core, we emit DialogEvent::ShutdownComplete
        self.emit_dialog_event(DialogEvent::ShutdownComplete).await;

        info!("DialogManager stopped successfully");
        Ok(())
    }

    /// Get the transaction manager reference
    ///
    /// Provides access to the underlying transaction manager for cases where
    /// direct transaction operations are needed.
    pub fn transaction_manager(&self) -> &Arc<TransactionManager> {
        &self.transaction_manager
    }

    /// Get dialog count
    ///
    /// Returns the current number of active dialogs.
    pub fn dialog_count(&self) -> usize {
        self.dialogs.len()
    }

    /// Check if a dialog exists
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog ID to check
    ///
    /// # Returns
    /// true if the dialog exists, false otherwise
    pub fn has_dialog(&self, dialog_id: &DialogId) -> bool {
        self.dialogs.contains_key(dialog_id)
    }

    /// Clean up completed transaction event receivers
    ///
    /// This method removes transaction-to-dialog mappings for completed transactions.
    ///
    /// # Arguments
    /// * `transaction_id` - The transaction ID to clean up
    pub fn cleanup_transaction_receiver(&self, transaction_id: &TransactionKey) {
        // Remove from transaction-to-dialog mapping if present
        if self.transaction_to_dialog.remove(transaction_id).is_some() {
            debug!(
                "Cleaned up transaction-dialog mapping for completed transaction {}",
                transaction_id
            );
        }
    }

    /// Find the INVITE transaction associated with a dialog
    ///
    /// This is used for CANCEL operations to find the pending INVITE transaction
    /// that needs to be cancelled.
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog ID to find the INVITE transaction for
    ///
    /// # Returns
    /// The transaction key for the INVITE if found, None otherwise
    pub fn find_invite_transaction_for_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Option<TransactionKey> {
        // Search through transaction-to-dialog mappings to find INVITE transaction
        for entry in self.transaction_to_dialog.iter() {
            let (tx_key, mapped_dialog_id) = entry.pair();

            // Check if this transaction belongs to our dialog and is an INVITE
            if mapped_dialog_id == dialog_id && tx_key.method() == &Method::Invite {
                debug!(
                    "Found INVITE transaction {} for dialog {}",
                    tx_key, dialog_id
                );
                return Some(tx_key.clone());
            }
        }

        debug!("No INVITE transaction found for dialog {}", dialog_id);
        None
    }

    // ========================================
    // **NEW**: UNIFIED CONFIGURATION SUPPORT
    // ========================================

    /// Set the unified configuration for this DialogManager
    ///
    /// Enables mode-specific behavior based on configuration.
    /// This method allows the UnifiedDialogManager to inject configuration.
    ///
    /// # Arguments
    /// * `config` - Unified configuration determining behavior mode
    pub fn set_config(&mut self, config: DialogManagerConfig) {
        debug!(
            "Setting unified configuration to {:?} mode",
            Self::config_mode_name(&config)
        );
        if let Ok(mut guard) = self.config.write() {
            *guard = Some(config);
        }
    }

    /// Get a clone of the current configuration (if any).
    pub fn config(&self) -> Option<DialogManagerConfig> {
        self.config.read().ok().and_then(|g| g.clone())
    }

    /// Check if auto-response to OPTIONS requests is enabled
    pub fn should_auto_respond_to_options(&self) -> bool {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.auto_options_enabled()))
            .unwrap_or(false)
    }

    /// Check if auto-response to REGISTER requests is enabled
    pub fn should_auto_respond_to_register(&self) -> bool {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.auto_register_enabled()))
            .unwrap_or(false)
    }

    /// Check if outgoing calls are supported (defaults to true when no config).
    pub fn supports_outgoing_calls(&self) -> bool {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.supports_outgoing_calls()))
            .unwrap_or(true)
    }

    /// Check if incoming calls are supported (defaults to true when no config).
    pub fn supports_incoming_calls(&self) -> bool {
        self.config
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|c| c.supports_incoming_calls()))
            .unwrap_or(true)
    }

    /// Get configuration mode name for logging
    fn config_mode_name(config: &DialogManagerConfig) -> &'static str {
        match config {
            DialogManagerConfig::Client(_) => "Client",
            DialogManagerConfig::Server(_) => "Server",
            DialogManagerConfig::Hybrid(_) => "Hybrid",
        }
    }
}

// Forward declarations for methods that will be implemented in other modules
impl DialogManager {
    // Dialog Operations (delegated to dialog_operations.rs)
    pub async fn create_dialog(&self, request: &Request) -> DialogResult<DialogId> {
        <Self as super::dialog_operations::DialogStore>::create_dialog(self, request).await
    }

    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> DialogResult<()> {
        <Self as super::dialog_operations::DialogStore>::terminate_dialog(self, dialog_id).await
    }

    pub fn get_dialog(&self, dialog_id: &DialogId) -> DialogResult<Dialog> {
        <Self as super::dialog_operations::DialogStore>::get_dialog(self, dialog_id)
    }

    pub fn get_dialog_mut(
        &self,
        dialog_id: &DialogId,
    ) -> DialogResult<dashmap::mapref::one::RefMut<DialogId, Dialog>> {
        <Self as super::dialog_operations::DialogStore>::get_dialog_mut(self, dialog_id)
    }

    pub async fn store_dialog(&self, dialog: Dialog) -> DialogResult<()> {
        <Self as super::dialog_operations::DialogStore>::store_dialog(self, dialog).await
    }

    pub fn list_dialogs(&self) -> Vec<DialogId> {
        <Self as super::dialog_operations::DialogStore>::list_dialogs(self)
    }

    pub fn get_dialog_state(&self, dialog_id: &DialogId) -> DialogResult<DialogState> {
        <Self as super::dialog_operations::DialogStore>::get_dialog_state(self, dialog_id)
    }

    pub async fn update_dialog_state(
        &self,
        dialog_id: &DialogId,
        new_state: DialogState,
    ) -> DialogResult<()> {
        <Self as super::dialog_operations::DialogStore>::update_dialog_state(
            self, dialog_id, new_state,
        )
        .await
    }

    pub async fn create_outgoing_dialog(
        &self,
        local_uri: rvoip_sip_core::Uri,
        remote_uri: rvoip_sip_core::Uri,
        call_id: Option<String>,
    ) -> DialogResult<DialogId> {
        <Self as super::dialog_operations::DialogStore>::create_outgoing_dialog(
            self, local_uri, remote_uri, call_id,
        )
        .await
    }

    /// Get a reference to the subscription manager if configured
    pub fn subscription_manager(&self) -> Option<&Arc<SubscriptionManager>> {
        self.subscription_manager.as_ref()
    }

    // ===== Event Hub Helper Methods =====

    /// Set the event hub for global event coordination
    pub async fn set_event_hub(&self, event_hub: Arc<crate::events::DialogEventHub>) {
        *self.event_hub.write().await = Some(event_hub);
    }

    /// Get session ID from dialog ID
    pub fn get_session_id(&self, dialog_id: &DialogId) -> Option<String> {
        self.dialog_to_session
            .get(dialog_id)
            .map(|e| e.value().clone())
    }

    /// Store dialog mapping for incoming call
    pub fn store_dialog_mapping(
        &self,
        session_id: &str,
        dialog_id: DialogId,
        transaction_id: TransactionKey,
        request: rvoip_sip_core::Request,
        source: SocketAddr,
    ) {
        self.session_to_dialog
            .insert(session_id.to_string(), dialog_id.clone());
        self.dialog_to_session
            .insert(dialog_id.clone(), session_id.to_string());
        self.transaction_to_dialog.insert(transaction_id, dialog_id);
        // Store additional request data if needed
    }

    // Protocol Handlers (delegated to protocol_handlers.rs)
    pub async fn handle_invite(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_invite_method(
            self, request, source,
        )
        .await
    }

    pub async fn handle_bye(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_bye_method(self, request).await
    }

    pub async fn handle_cancel(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_cancel_method(self, request)
            .await
    }

    pub async fn handle_ack(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_ack_method(self, request).await
    }

    pub async fn handle_options(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_options_method(
            self, request, source,
        )
        .await
    }

    pub async fn handle_register(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_register_method(
            self, request, source,
        )
        .await
    }

    pub async fn handle_update(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_update_method(self, request)
            .await
    }

    pub async fn handle_prack(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_prack_method(self, request)
            .await
    }

    pub async fn handle_info(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_info_method(self, request, source)
            .await
    }

    pub async fn handle_refer(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_refer_method(
            self, request, source,
        )
        .await
    }

    pub async fn handle_subscribe(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_subscribe_method(
            self, request, source,
        )
        .await
    }

    pub async fn handle_notify(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_notify_method(
            self, request, source,
        )
        .await
    }

    /// Snapshot of the externally-visible address most recently
    /// learned from an inbound response's `Via: …;received=…;rport=…`
    /// (RFC 3581). Returns `None` until the first qualifying response
    /// arrives — i.e. before the first request goes on the wire, or
    /// when no NAT is in the path (in which case the discovered
    /// address would equal the local bind and we suppress the
    /// update).
    ///
    /// Callers can use this to rewrite outbound `Contact:` headers
    /// (RFC 5626 §5) so a registrar's stored binding routes through
    /// the discovered NAT mapping rather than the unreachable
    /// private bind address.
    pub async fn discovered_public_addr(&self) -> Option<SocketAddr> {
        *self.nat_discovered_addr.read().await
    }

    /// Returns the registrar-provided Service-Route (RFC 3608) for the
    /// given AoR, if a REGISTER 2xx has populated the cache. The
    /// returned URIs MUST be pre-loaded as Route headers on subsequent
    /// out-of-dialog requests from the UA for that AoR, in the order
    /// returned.
    ///
    /// `None` → no REGISTER 2xx observed for this AoR yet.
    /// `Some(empty vec)` → REGISTER 2xx observed, registrar declined to
    /// set a Service-Route (caller should not pre-load any Route).
    pub async fn service_route_for_aor(
        &self,
        aor: &str,
    ) -> Option<Vec<rvoip_sip_core::types::uri::Uri>> {
        self.service_route_by_aor.read().await.get(aor).cloned()
    }

    /// Returns the registrar-assigned GRUU URIs (RFC 5627 §5.3) for
    /// the given AoR, if a REGISTER 2xx has populated the cache.
    /// Either `pub_gruu` or `temp_gruu` may be `None` independently —
    /// a registrar may assign only one. `None` from this accessor
    /// means no REGISTER 2xx with GRUU has been observed for this AoR
    /// yet (or the registrar declined to assign either GRUU).
    pub async fn gruu_for_aor(
        &self,
        aor: &str,
    ) -> Option<rvoip_sip_core::types::outbound::GruuContactParams> {
        self.gruu_by_aor.read().await.get(aor).cloned()
    }

    pub async fn handle_response(
        &self,
        response: Response,
        transaction_id: TransactionKey,
    ) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_response_message(
            self,
            response,
            transaction_id,
        )
        .await
    }

    // Message Routing (delegated to message_routing.rs)
    pub async fn find_dialog_for_request(&self, request: &Request) -> Option<DialogId> {
        <Self as super::dialog_operations::DialogLookup>::find_dialog_for_request(self, request)
            .await
    }

    pub fn find_dialog_for_transaction(
        &self,
        transaction_id: &TransactionKey,
    ) -> DialogResult<DialogId> {
        <Self as super::message_routing::DialogMatcher>::match_transaction(self, transaction_id)
    }

    // Transaction Integration (delegated to transaction_integration.rs)
    pub async fn send_request(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> DialogResult<TransactionKey> {
        <Self as super::transaction_integration::TransactionIntegration>::send_request_in_dialog(
            self, dialog_id, method, body,
        )
        .await
    }

    /// Send a BYE request carrying a `Reason:` header (RFC 3326).
    ///
    /// Used by the session-timer refresh-failure path (RFC 4028 §10) to
    /// communicate `Reason: SIP ;cause=408 ;text="Session expired"` on the
    /// BYE so peer observability is RFC-correct. Mirrors the transport
    /// plumbing of `send_request` for BYE but threads a typed `Reason`
    /// header through `bye_for_dialog`'s `extra_headers` param.
    pub async fn send_bye_with_reason(
        &self,
        dialog_id: &DialogId,
        reason: rvoip_sip_core::types::reason::Reason,
    ) -> DialogResult<TransactionKey> {
        use crate::transaction::dialog::quick as dialog_quick;
        use rvoip_sip_core::types::TypedHeader;

        debug!("Sending BYE with Reason header for dialog {}", dialog_id);

        let (destination, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let destination = dialog
                .get_remote_target_address()
                .await
                .ok_or_else(|| DialogError::routing_error("No remote target address available"))?;

            let template = dialog.create_request_template(Method::Bye);

            let local_tag = match template.local_tag {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            let remote_tag = template
                .remote_tag
                .filter(|t| !t.is_empty())
                .ok_or_else(|| {
                    DialogError::protocol_error("BYE requires remote tag in established dialog")
                })?;

            let request = dialog_quick::bye_for_dialog(
                &template.call_id,
                &template.local_uri.to_string(),
                &local_tag,
                &template.remote_uri.to_string(),
                &remote_tag,
                template.cseq_number,
                self.local_address,
                if template.route_set.is_empty() {
                    None
                } else {
                    Some(template.route_set.clone())
                },
                Some(vec![TypedHeader::Reason(reason)]),
            )
            .map_err(|e| DialogError::InternalError {
                message: format!("Failed to build BYE request: {}", e),
                context: None,
            })?;

            (destination, request)
        };

        let transaction_id = self
            .transaction_manager
            .create_non_invite_client_transaction(request, destination)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create BYE transaction: {}", e),
            })?;

        self.transaction_to_dialog
            .insert(transaction_id.clone(), dialog_id.clone());
        debug!(
            "Associated BYE-with-Reason transaction {} with dialog {}",
            transaction_id, dialog_id
        );

        self.transaction_manager
            .send_request(&transaction_id)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send BYE: {}", e),
            })?;

        Ok(transaction_id)
    }

    /// Send an INFO request carrying a caller-chosen `Content-Type` (RFC 6086).
    ///
    /// The generic [`send_request_in_dialog`](Self::send_request) path always
    /// tags INFO bodies as `application/info`. This method lets the caller
    /// pick any content type — `application/dtmf-relay` for DTMF-over-INFO,
    /// `application/sipfrag` for fax flow control, etc.
    pub async fn send_info_with_content_type(
        &self,
        dialog_id: &DialogId,
        content_type: String,
        body: bytes::Bytes,
    ) -> DialogResult<TransactionKey> {
        use crate::transaction::dialog::quick as dialog_quick;

        debug!(
            "Sending INFO with Content-Type: {} for dialog {}",
            content_type, dialog_id
        );

        let (destination, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;

            let destination = dialog
                .get_remote_target_address()
                .await
                .ok_or_else(|| DialogError::routing_error("No remote target address available"))?;

            let template = dialog.create_request_template(Method::Info);

            let local_tag = match template.local_tag {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };

            let remote_tag = template
                .remote_tag
                .filter(|t| !t.is_empty())
                .ok_or_else(|| {
                    DialogError::protocol_error("INFO requires remote tag in established dialog")
                })?;

            let body_str = String::from_utf8_lossy(&body).into_owned();
            let request = dialog_quick::info_for_dialog(
                &template.call_id,
                &template.local_uri.to_string(),
                &local_tag,
                &template.remote_uri.to_string(),
                &remote_tag,
                body_str,
                Some(content_type),
                template.cseq_number,
                self.local_address,
                if template.route_set.is_empty() {
                    None
                } else {
                    Some(template.route_set.clone())
                },
            )
            .map_err(|e| DialogError::InternalError {
                message: format!("Failed to build INFO request: {}", e),
                context: None,
            })?;

            (destination, request)
        };

        let transaction_id = self
            .transaction_manager
            .create_non_invite_client_transaction(request, destination)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create INFO transaction: {}", e),
            })?;

        self.transaction_to_dialog
            .insert(transaction_id.clone(), dialog_id.clone());

        self.transaction_manager
            .send_request(&transaction_id)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send INFO: {}", e),
            })?;

        Ok(transaction_id)
    }

    pub async fn send_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        <Self as super::transaction_integration::TransactionIntegration>::send_transaction_response(
            self,
            transaction_id,
            response,
        )
        .await
    }

    pub fn associate_transaction_with_dialog(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: &DialogId,
    ) {
        <Self as super::transaction_integration::TransactionHelpers>::link_transaction_to_dialog(
            self,
            transaction_id,
            dialog_id,
        )
    }

    pub async fn send_ack_for_2xx_response(
        &self,
        dialog_id: &DialogId,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> DialogResult<()> {
        debug!("Sending ACK for 2xx response for dialog {}", dialog_id);

        // Use transaction-core's send_ack_for_2xx method to actually send the ACK
        self.transaction_manager
            .send_ack_for_2xx(original_invite_tx_id, response)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to send ACK for 2xx response: {}", e),
            })?;

        debug!(
            "Successfully sent ACK for 2xx response for dialog {}",
            dialog_id
        );
        Ok(())
    }

    pub async fn create_ack_for_2xx_response(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> DialogResult<Request> {
        <Self as super::transaction_integration::TransactionHelpers>::create_ack_for_success_response(self, original_invite_tx_id, response).await
    }

    pub async fn find_transaction_by_message(
        &self,
        message: &rvoip_sip_core::Message,
    ) -> DialogResult<Option<TransactionKey>> {
        debug!("Finding transaction for message using transaction-core");

        self.transaction_manager
            .find_transaction_by_message(message)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to find transaction by message: {}", e),
            })
    }
}

/// Drives a single [`OutboundFlow`] for its lifetime.
///
/// The loop alternates between sending CRLFCRLF pings on the keep-alive
/// interval and waiting for the per-ping pong deadline. Transport-side
/// events (`KeepAlivePongReceived`, `ConnectionClosed`) arrive out of
/// band via [`DialogManager::on_pong_received`] /
/// [`DialogManager::on_connection_closed`] and flip state on the shared
/// [`OutboundFlow`]; this task observes the updates when it next wakes
/// on the deadline arm.
///
/// The task exits — and cleans up its own registration in the manager's
/// maps — when it observes failure or when the manager aborts it.
async fn run_outbound_flow_loop(
    manager: DialogManager,
    flow: Arc<OutboundFlow>,
    transport: Arc<dyn rvoip_sip_transport::Transport>,
) {
    use bytes::Bytes;

    let mut ticker = tokio::time::interval(flow.interval);
    // The first tick fires immediately; skip it so the first ping goes
    // out at `interval` after REGISTER success, not right away (avoids
    // a thundering herd on bulk re-REGISTER).
    ticker.tick().await;

    // Pong deadline. Parked far in the future when no ping is
    // outstanding so the select arm effectively waits forever; reset to
    // `now + pong_timeout` after every successful ping.
    //
    // 365 days is well below `Instant` overflow on every platform we
    // support and low enough that pinning it here is harmless.
    let far_future =
        || tokio::time::Instant::now() + std::time::Duration::from_secs(365 * 24 * 3600);
    let sleep = tokio::time::sleep_until(far_future());
    tokio::pin!(sleep);
    let mut deadline_armed = false;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match transport
                    .send_raw(flow.destination, Bytes::from_static(b"\r\n\r\n"))
                    .await
                {
                    Ok(()) => {
                        flow.record_ping_sent().await;
                        tracing::trace!(
                            flow_key = ?flow.key, dest = %flow.destination,
                            "RFC 5626 keep-alive ping sent"
                        );
                        let when = tokio::time::Instant::now() + flow.pong_timeout;
                        sleep.as_mut().reset(when);
                        deadline_armed = true;
                    }
                    Err(e) => {
                        tracing::debug!(
                            flow_key = ?flow.key, dest = %flow.destination, error = %e,
                            "RFC 5626 keep-alive send failed — marking flow failed"
                        );
                        if flow.mark_failed().await {
                            manager
                                .emit_outbound_flow_failed(&flow, FlowFailureReason::SendError)
                                .await;
                        }
                        break;
                    }
                }
            }
            _ = &mut sleep, if deadline_armed => {
                // Pong deadline fired. Re-check state under the flow's
                // internal locks — if a pong arrived during the wait
                // it already reset state to `Idle` and we just disarm.
                if flow.is_pong_overdue().await {
                    tracing::info!(
                        flow_key = ?flow.key, dest = %flow.destination,
                        pong_timeout_ms = flow.pong_timeout.as_millis() as u64,
                        "RFC 5626 pong timeout — marking flow failed"
                    );
                    if flow.mark_failed().await {
                        manager
                            .emit_outbound_flow_failed(&flow, FlowFailureReason::PongTimeout)
                            .await;
                    }
                    break;
                }
                sleep.as_mut().reset(far_future());
                deadline_armed = false;
            }
        }
    }

    // Clean up both the primary flow and the secondary index so a
    // future REGISTER 2xx for the same AoR can install a fresh flow.
    // Safe if `stop_outbound_ping` already removed us concurrently.
    manager.stop_outbound_ping(&flow.key);
}

#[cfg(test)]
mod outbound_flow_handler_tests {
    //! Tests for the `DialogManager` → `OutboundFlow` plumbing that
    //! lives across `on_pong_received`, `on_connection_closed`, and the
    //! `(outbound_flows, flow_by_destination)` pair. The state machine
    //! itself is unit-tested in `super::outbound_flow::tests`; these
    //! tests drive the handler entry points by pre-populating the maps
    //! so we don't have to boot a real transport or spawn the ping
    //! loop.
    use super::*;
    use crate::manager::outbound_flow::{FlowState, OutboundFlow};
    use rvoip_sip_transport::error::Result as TransportResult;
    use rvoip_sip_transport::{Transport, TransportEvent};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[derive(Debug)]
    struct NoopTransport {
        addr: SocketAddr,
        closed: AtomicBool,
    }

    impl NoopTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                addr: SocketAddr::from_str("127.0.0.1:5060").unwrap(),
                closed: AtomicBool::new(false),
            })
        }
    }

    #[async_trait::async_trait]
    impl Transport for NoopTransport {
        fn local_addr(&self) -> TransportResult<SocketAddr> {
            Ok(self.addr)
        }
        async fn send_message(
            &self,
            _m: rvoip_sip_core::Message,
            _dst: SocketAddr,
        ) -> TransportResult<()> {
            Ok(())
        }
        async fn close(&self) -> TransportResult<()> {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }
    }

    async fn make_manager() -> (DialogManager, mpsc::Receiver<SessionCoordinationEvent>) {
        let transport = NoopTransport::new();
        let (_tx, transport_rx) = mpsc::channel::<TransportEvent>(16);
        let (tm, _events_rx) = TransactionManager::new(transport, transport_rx, Some(16))
            .await
            .expect("build TransactionManager");
        let local = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let manager = DialogManager::new(Arc::new(tm), local)
            .await
            .expect("build DialogManager");

        // Install a legacy session-coordination channel so
        // `emit_session_coordination_event` delivers into the test.
        let (sc_tx, sc_rx) = mpsc::channel::<SessionCoordinationEvent>(16);
        *manager.session_coordinator.write().await = Some(sc_tx);

        (manager, sc_rx)
    }

    fn test_key(n: u8) -> (String, u32, String) {
        (
            format!("sip:alice{n}@example.com"),
            1,
            format!("urn:uuid:{n:032x}"),
        )
    }

    fn dest_addr(port: u16) -> SocketAddr {
        SocketAddr::from_str(&format!("127.0.0.1:{port}")).unwrap()
    }

    fn install_flow(
        manager: &DialogManager,
        key: (String, u32, String),
        dest: SocketAddr,
    ) -> Arc<OutboundFlow> {
        let flow = Arc::new(OutboundFlow::new(
            key.clone(),
            dest,
            Duration::from_secs(25),
        ));
        manager.outbound_flows.insert(key.clone(), flow.clone());
        manager
            .flow_by_destination
            .entry(dest)
            .or_insert_with(Vec::new)
            .push(key);
        flow
    }

    #[tokio::test]
    async fn on_pong_received_resets_existing_flow_state() {
        let (manager, _rx) = make_manager().await;
        let key = test_key(1);
        let dest = dest_addr(5080);
        let flow = install_flow(&manager, key.clone(), dest);
        flow.record_ping_sent().await;
        assert_eq!(flow.state().await, FlowState::AwaitingPong);

        manager.on_pong_received(dest).await;

        assert_eq!(flow.state().await, FlowState::Idle);
    }

    #[tokio::test]
    async fn on_pong_received_is_noop_for_unknown_destination() {
        let (manager, _rx) = make_manager().await;
        let key = test_key(1);
        let flow = install_flow(&manager, key.clone(), dest_addr(5081));
        flow.record_ping_sent().await;

        // A pong from a peer we don't have a flow for must not disturb
        // the existing flow.
        manager.on_pong_received(dest_addr(9999)).await;

        assert_eq!(flow.state().await, FlowState::AwaitingPong);
    }

    #[tokio::test]
    async fn on_connection_closed_emits_event_once_and_clears_maps() {
        let (manager, mut rx) = make_manager().await;
        let key = test_key(2);
        let dest = dest_addr(5082);
        let flow = install_flow(&manager, key.clone(), dest);

        manager.on_connection_closed(dest).await;

        // Exactly one OutboundFlowFailed for this key with
        // `ConnectionClosed` reason.
        let event = tokio::time::timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("event must arrive")
            .expect("channel open");
        match event {
            SessionCoordinationEvent::OutboundFlowFailed {
                aor,
                reg_id,
                instance,
                reason,
            } => {
                assert_eq!(aor, key.0);
                assert_eq!(reg_id, key.1);
                assert_eq!(instance, key.2);
                assert_eq!(reason, FlowFailureReason::ConnectionClosed);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        // Flow state flipped to Failed.
        assert_eq!(flow.state().await, FlowState::Failed);

        // Maps were cleared.
        assert!(!manager.outbound_flows.contains_key(&key));
        assert!(manager.flow_by_destination.get(&dest).is_none());

        // Idempotent: a second close for the same destination must not
        // emit another event (the flow is gone + state is Failed
        // anyway).
        manager.on_connection_closed(dest).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err(),
            "no additional event after second close"
        );
    }

    #[tokio::test]
    async fn stop_outbound_ping_does_not_emit_failure_event() {
        // Explicit teardown is not a flow failure; no event should fire.
        let (manager, mut rx) = make_manager().await;
        let key = test_key(3);
        let dest = dest_addr(5083);
        let _flow = install_flow(&manager, key.clone(), dest);

        manager.stop_outbound_ping(&key);

        assert!(!manager.outbound_flows.contains_key(&key));
        assert!(manager.flow_by_destination.get(&dest).is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), rx.recv())
                .await
                .is_err(),
            "stop must not emit OutboundFlowFailed"
        );
    }
}
