//! Unified DialogManager Implementation
//!
//! This module provides a unified DialogManager that replaces the separate
//! DialogClient and DialogServer implementations with a single, configuration-driven
//! approach. This aligns with SIP standards where endpoints typically act as both
//! UAC and UAS depending on the transaction, not the application type.
//!
//! ## Architecture
//!
//! ```text
//! DialogManager (unified)
//!        │
//!        ├── Configuration-based behavior
//!        │   ├── Client mode (primarily outgoing)
//!        │   ├── Server mode (primarily incoming)
//!        │   └── Hybrid mode (both directions)
//!        │
//!        ├── Core SIP dialog management (shared)
//!        │   ├── Dialog lifecycle
//!        │   ├── Transaction coordination
//!        │   └── RFC 3261 compliance
//!        │
//!        └── High-level operations (shared)
//!            ├── Response building
//!            ├── SIP method helpers
//!            └── Session coordination
//! ```
//!
//! ## Key Benefits
//!
//! - **Standards Aligned**: Matches how SIP actually works (UAC/UAS per transaction)
//! - **Code Reduction**: ~1000 lines less than split implementation
//! - **Simpler Integration**: Single type for session-core to interact with
//! - **Runtime Flexibility**: Can handle both incoming and outgoing calls
//!
//! ## Examples
//!
//! ### Client Mode Usage
//!
//! ```rust,no_run
//! use rvoip_sip_dialog::manager::unified::UnifiedDialogManager;
//! use rvoip_sip_dialog::config::DialogManagerConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
//!     .with_from_uri("sip:alice@example.com")
//!     .with_auth("alice", "secret123")
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let manager = UnifiedDialogManager::new(transaction_manager, config).await?;
//! manager.start().await?;
//!
//! // Make outgoing calls
//! let call = manager.make_call(
//!     "sip:alice@example.com",
//!     "sip:bob@example.com",
//!     Some("SDP offer".to_string())
//! ).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Server Mode Usage
//!
//! ```rust,no_run
//! use rvoip_sip_dialog::manager::unified::UnifiedDialogManager;
//! use rvoip_sip_dialog::config::DialogManagerConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::server("0.0.0.0:5060".parse()?)
//!     .with_domain("sip.company.com")
//!     .with_auto_options()
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let manager = UnifiedDialogManager::new(transaction_manager, config).await?;
//! manager.start().await?;
//!
//! // Handle incoming calls via session coordination events
//! # Ok(())
//! # }
//! ```
//!
//! ### Hybrid Mode Usage
//!
//! ```rust,no_run
//! use rvoip_sip_dialog::manager::unified::UnifiedDialogManager;
//! use rvoip_sip_dialog::config::DialogManagerConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::hybrid("192.168.1.100:5060".parse()?)
//!     .with_from_uri("sip:pbx@company.com")
//!     .with_domain("company.com")
//!     .with_auth("pbx", "pbx_password")
//!     .with_auto_options()
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let manager = UnifiedDialogManager::new(transaction_manager, config).await?;
//! manager.start().await?;
//!
//! // Can both make outgoing calls AND handle incoming calls
//! # Ok(())
//! # }
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::transaction::{TransactionEvent, TransactionKey, TransactionManager};
use rvoip_sip_core::{Method, Request, Response, StatusCode, Uri};

use crate::api::{
    common::{CallHandle, DialogHandle},
    ApiError, ApiResult,
};
use crate::config::DialogManagerConfig;
use crate::dialog::{Dialog, DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::{DialogEvent, SessionCoordinationEvent};
use crate::subscription::SubscriptionManager;

// Import the existing core DialogManager functionality
use super::core::DialogManager;

/// Unified DialogManager that supports client, server, and hybrid modes
///
/// This is the core implementation that replaces separate DialogClient and DialogServer
/// types with a single, configuration-driven approach. The behavior is determined by
/// the DialogManagerConfig provided during construction.
///
/// ## Capabilities by Mode
///
/// ### Client Mode
/// - Make outgoing calls (`make_call`)
/// - Handle authentication challenges
/// - Send in-dialog requests
/// - Build and send responses (when needed)
///
/// ### Server Mode
/// - Handle incoming calls (via session coordination)
/// - Auto-respond to OPTIONS/REGISTER (if configured)
/// - Build and send responses
/// - Send in-dialog requests
///
/// ### Hybrid Mode
/// - All client capabilities
/// - All server capabilities
/// - Full bidirectional SIP support
///
/// ## Thread Safety
///
/// UnifiedDialogManager is fully thread-safe and can be shared across async tasks
/// using Arc<UnifiedDialogManager>.
#[derive(Debug, Clone)]
pub struct UnifiedDialogManager {
    /// Core dialog manager (contains all the actual implementation)
    core: DialogManager,

    /// Configuration determining behavior mode
    config: DialogManagerConfig,

    /// Statistics for this manager instance
    stats: Arc<tokio::sync::RwLock<ManagerStats>>,
}

/// Statistics for the unified dialog manager
#[derive(Debug, Default)]
pub struct ManagerStats {
    /// Number of active dialogs
    pub active_dialogs: usize,

    /// Total dialogs created
    pub total_dialogs: u64,

    /// Successful calls (ended with BYE)
    pub successful_calls: u64,

    /// Failed calls (ended with error)
    pub failed_calls: u64,

    /// Total call duration in seconds
    pub total_call_duration: f64,

    /// Outgoing calls made (client behavior)
    pub outgoing_calls: u64,

    /// Incoming calls handled (server behavior)
    pub incoming_calls: u64,

    /// Authentication challenges handled
    pub auth_challenges: u64,

    /// Auto-responses sent (OPTIONS, REGISTER)
    pub auto_responses: u64,
}

fn set_response_to_tag(response: &mut Response, tag: &str) {
    use rvoip_sip_core::types::{HeaderName, TypedHeader};

    if let Some(to_index) = response
        .headers
        .iter()
        .position(|header| header.name() == HeaderName::To)
    {
        if let TypedHeader::To(to_header) = response.headers[to_index].clone() {
            response.headers[to_index] = TypedHeader::To(to_header.with_tag(tag));
        }
    }
}

impl UnifiedDialogManager {
    /// Get inner dialog manager for event hub setup
    pub fn inner_manager(&self) -> &DialogManager {
        &self.core
    }
    /// Create a new unified dialog manager
    ///
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `config` - Configuration determining the behavior mode
    ///
    /// # Returns
    /// New UnifiedDialogManager instance
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        config: DialogManagerConfig,
    ) -> DialogResult<Self> {
        // Validate configuration first
        config.validate().map_err(|e| {
            DialogError::internal_error(&format!("Invalid configuration: {}", e), None)
        })?;

        let local_address = config.local_address();
        info!(
            "Creating UnifiedDialogManager in {:?} mode at {}",
            Self::mode_name(&config),
            local_address
        );

        // Create core dialog manager with the provided transaction manager
        let mut core = DialogManager::new_with_index_capacity(
            transaction_manager,
            local_address,
            config.dialog_config().max_dialogs.unwrap_or(10_000),
        )
        .await?;

        // **NEW**: Inject the unified configuration into the core manager
        core.set_config(config.clone());

        Ok(Self {
            core,
            config,
            stats: Arc::new(tokio::sync::RwLock::new(ManagerStats::default())),
        })
    }

    /// Create a new unified dialog manager with global events (RECOMMENDED)
    ///
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `transaction_events` - Global transaction event receiver
    /// * `config` - Configuration determining the behavior mode
    ///
    /// # Returns
    /// New UnifiedDialogManager instance with proper event consumption
    pub async fn with_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        config: DialogManagerConfig,
    ) -> DialogResult<Self> {
        // Validate configuration first
        if let Err(e) = config.validate() {
            error!(
                "Failed to create UnifiedDialogManager: Invalid configuration - {}",
                e
            );
            return Err(DialogError::internal_error(
                &format!("Invalid configuration: {}", e),
                None,
            ));
        }

        let local_address = config.local_address();
        info!(
            "Creating UnifiedDialogManager with global events in {:?} mode at {}",
            Self::mode_name(&config),
            local_address
        );

        // Create core dialog manager with global events
        let mut core = DialogManager::with_global_events_and_index_capacity(
            transaction_manager,
            transaction_events,
            local_address,
            config.dialog_config().max_dialogs.unwrap_or(10_000),
        )
        .await?;

        // **NEW**: Inject the unified configuration into the core manager
        core.set_config(config.clone());

        Ok(Self {
            core,
            config,
            stats: Arc::new(tokio::sync::RwLock::new(ManagerStats::default())),
        })
    }

    /// Get the configuration mode name for logging
    fn mode_name(config: &DialogManagerConfig) -> &'static str {
        match config {
            DialogManagerConfig::Client(_) => "Client",
            DialogManagerConfig::Server(_) => "Server",
            DialogManagerConfig::Hybrid(_) => "Hybrid",
        }
    }

    /// Get the current configuration
    pub fn config(&self) -> &DialogManagerConfig {
        &self.config
    }

    /// Get the underlying core dialog manager
    ///
    /// Provides access to the core dialog management functionality.
    /// Useful for advanced operations that bypass the unified API.
    pub fn core(&self) -> &DialogManager {
        &self.core
    }

    /// Get reference to the subscription manager if configured
    pub fn subscription_manager(&self) -> Option<&Arc<SubscriptionManager>> {
        self.core.subscription_manager()
    }

    /// Start the unified dialog manager
    ///
    /// Initializes the manager for processing based on its configuration mode.
    pub async fn start(&self) -> DialogResult<()> {
        info!(
            "Starting UnifiedDialogManager in {:?} mode",
            Self::mode_name(&self.config)
        );

        // Start the core dialog manager
        self.core.start().await?;

        // Log mode-specific capabilities
        match &self.config {
            DialogManagerConfig::Client(client) => {
                info!(
                    "Client mode active - from_uri: {:?}, auto_auth: {}",
                    client.from_uri, client.auto_auth
                );
            }
            DialogManagerConfig::Server(server) => {
                info!(
                    "Server mode active - domain: {:?}, auto_options: {}, auto_register: {}",
                    server.domain, server.auto_options_response, server.auto_register_response
                );
            }
            DialogManagerConfig::Hybrid(hybrid) => {
                info!(
                    "Hybrid mode active - from_uri: {:?}, domain: {:?}, auto_auth: {}, auto_options: {}",
                    hybrid.from_uri, hybrid.domain, hybrid.auto_auth, hybrid.auto_options_response
                );
            }
        }

        info!("UnifiedDialogManager started successfully");
        Ok(())
    }

    /// Stop the unified dialog manager
    ///
    /// Gracefully shuts down the manager and all active dialogs.
    pub async fn stop(&self) -> DialogResult<()> {
        info!("Stopping UnifiedDialogManager");

        // Stop the core dialog manager
        self.core.stop().await?;

        info!("UnifiedDialogManager stopped successfully");
        Ok(())
    }

    // REMOVED: Channel-based methods - use GlobalEventCoordinator instead
    // - set_session_coordinator()
    // - set_dialog_event_sender()
    // - subscribe_to_dialog_events()

    // ========================================
    // CLIENT-MODE OPERATIONS
    // ========================================

    /// Make an outgoing call (Client/Hybrid modes only)
    ///
    /// Creates a new dialog and sends an INVITE request to establish a call.
    /// Only available in Client and Hybrid modes.
    ///
    /// # Arguments
    /// * `from_uri` - Local URI for the call
    /// * `to_uri` - Remote URI to call
    /// * `sdp_offer` - Optional SDP offer for media negotiation
    ///
    /// # Returns
    /// CallHandle for managing the call
    pub async fn make_call(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
    ) -> ApiResult<CallHandle> {
        self.make_call_with_id(from_uri, to_uri, sdp_offer, None)
            .await
    }

    /// Make an outgoing call with specific Call-ID
    /// Variant of `make_call_with_id` that pre-registers a session↔dialog
    /// mapping before sending the INVITE. Eliminates the fast-RTT race where
    /// a 4xx/5xx response can arrive (and be processed by the event loop)
    /// before the caller has a chance to store the mapping.
    pub async fn make_call_for_session(
        &self,
        session_id: &str,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
        call_id: Option<String>,
    ) -> ApiResult<CallHandle> {
        self.make_call_inner(
            from_uri,
            to_uri,
            sdp_offer,
            call_id,
            Some(session_id.to_string()),
            Vec::new(),
        )
        .await
    }

    pub async fn make_call_with_id(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
        call_id: Option<String>,
    ) -> ApiResult<CallHandle> {
        self.make_call_inner(from_uri, to_uri, sdp_offer, call_id, None, Vec::new())
            .await
    }

    /// Send a dialog-creating SUBSCRIBE and pre-register a session↔dialog mapping.
    pub async fn send_subscribe_out_of_dialog_for_session(
        &self,
        session_id: &str,
        target_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        event_package: &str,
        expires: u32,
    ) -> ApiResult<DialogId> {
        use crate::dialog::subscription_state::SubscriptionState;
        use crate::manager::utils::DialogUtils;
        use crate::transaction::dialog::quick;

        let from: Uri = from_uri.parse().map_err(|e| ApiError::Configuration {
            message: format!("Invalid from_uri: {}", e),
        })?;
        let target: Uri = target_uri.parse().map_err(|e| ApiError::Configuration {
            message: format!("Invalid target_uri: {}", e),
        })?;

        let dialog_id = self
            .core
            .create_outgoing_dialog(from, target.clone(), None)
            .await
            .map_err(ApiError::from)?;

        self.core
            .session_to_dialog
            .insert(session_id.to_string(), dialog_id.clone());
        self.core
            .dialog_to_session
            .insert(dialog_id.clone(), session_id.to_string());

        let (destination, request) = {
            let mut dialog = self
                .core
                .get_dialog_mut(&dialog_id)
                .map_err(ApiError::from)?;
            let local_tag = match dialog.local_tag.clone() {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(tag.clone());
                    tag
                }
            };
            dialog.local_cseq += 1;
            dialog.event_package = Some(event_package.to_string());
            let duration = std::time::Duration::from_secs(expires as u64);
            dialog.subscription_state = Some(SubscriptionState::Active {
                remaining_duration: duration,
                original_duration: duration,
            });

            let local_address = self
                .core
                .local_address_for_target_and_routes(&target, &dialog.route_set);
            let request = quick::subscribe_out_of_dialog_with_identity(
                target_uri,
                from_uri,
                contact_uri,
                event_package,
                expires,
                dialog.local_cseq,
                local_address,
                dialog.call_id.clone(),
                local_tag,
            )
            .map_err(|e| ApiError::protocol(format!("Failed to build SUBSCRIBE: {}", e)))?;
            let destination = crate::dialog::dialog_utils::resolve_uri_to_socketaddr(
                &crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request),
            )
            .await
            .ok_or_else(|| {
                ApiError::protocol(format!(
                    "Failed to resolve SUBSCRIBE target URI: {}",
                    target_uri
                ))
            })?;
            (destination, request)
        };

        let transaction_id = self
            .core
            .transaction_manager()
            .create_non_invite_client_transaction(request, destination)
            .await
            .map_err(|e| {
                ApiError::internal(format!("Failed to create SUBSCRIBE transaction: {}", e))
            })?;
        self.core
            .link_transaction_to_dialog_indexed(&transaction_id, &dialog_id);
        self.core
            .transaction_manager()
            .send_request(&transaction_id)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to send SUBSCRIBE: {}", e)))?;

        let response = self
            .core
            .transaction_manager()
            .wait_for_final_response(&transaction_id, std::time::Duration::from_secs(30))
            .await
            .map_err(|e| {
                ApiError::internal(format!("Failed to wait for SUBSCRIBE response: {}", e))
            })?
            .ok_or_else(|| ApiError::network("SUBSCRIBE timed out".to_string()))?;

        if !(200..=299).contains(&response.status_code()) {
            return Err(ApiError::protocol(format!(
                "SUBSCRIBE failed with {} {}",
                response.status_code(),
                response.reason_phrase()
            )));
        }

        {
            let mut dialog = self
                .core
                .get_dialog_mut(&dialog_id)
                .map_err(ApiError::from)?;
            dialog.update_from_2xx(&response);
            if dialog.remote_tag.is_some() {
                dialog.state = DialogState::Confirmed;
            }
            if let Some(tuple) = dialog.dialog_id_tuple() {
                let key = DialogUtils::create_lookup_key(&tuple.0, &tuple.1, &tuple.2);
                self.core.dialog_lookup.insert(key, dialog_id.clone());
            }
        }

        Ok(dialog_id)
    }

    /// Refresh or terminate an existing SUBSCRIBE dialog.
    pub async fn send_subscribe_refresh(
        &self,
        dialog_id: &DialogId,
        event_package: &str,
        expires: u32,
    ) -> ApiResult<()> {
        self.send_subscribe_refresh_with_extras(
            dialog_id,
            event_package,
            expires,
            None,
            None,
            Vec::new(),
        )
        .await
    }

    /// In-dialog SUBSCRIBE refresh with application-staged
    /// `extra_headers` appended after the stack-managed slice. See
    /// SIP_API_DESIGN_2 §5.2.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_subscribe_refresh_with_extras(
        &self,
        dialog_id: &DialogId,
        event_package: &str,
        expires: u32,
        accept: Option<String>,
        authorization: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> ApiResult<()> {
        use crate::transaction::dialog::{
            request_builder_from_dialog_template, DialogRequestTemplate,
        };
        use rvoip_sip_core::types::event::{Event, EventType};
        use rvoip_sip_core::types::expires::Expires;
        use rvoip_sip_core::types::header::{HeaderName, HeaderValue};
        use rvoip_sip_core::types::TypedHeader;

        let (destination, request) = {
            let mut dialog = self
                .core
                .get_dialog_mut(dialog_id)
                .map_err(ApiError::from)?;
            let template = dialog.create_request_template(Method::Subscribe);
            let local_tag = template.local_tag.clone().ok_or_else(|| {
                ApiError::protocol("SUBSCRIBE refresh requires local tag".to_string())
            })?;
            let remote_tag = template.remote_tag.clone().ok_or_else(|| {
                ApiError::protocol("SUBSCRIBE refresh requires remote tag".to_string())
            })?;
            let local_address = self
                .core
                .local_address_for_target_and_routes(&template.target_uri, &template.route_set);
            let template = DialogRequestTemplate {
                call_id: template.call_id,
                from_uri: template.local_uri.to_string(),
                from_tag: local_tag,
                to_uri: template.remote_uri.to_string(),
                to_tag: remote_tag,
                request_uri: template.target_uri.to_string(),
                cseq: template.cseq_number,
                local_address,
                route_set: template.route_set.clone(),
                contact: self.core.local_contact_uri(),
            };
            let mut request = request_builder_from_dialog_template(
                &template,
                Method::Subscribe,
                None,
                None,
                None,
            )
            .map_err(|e| ApiError::protocol(format!("Failed to build SUBSCRIBE refresh: {}", e)))?;
            request
                .headers
                .push(TypedHeader::Event(Event::new(EventType::Token(
                    event_package.to_string(),
                ))));
            request
                .headers
                .push(TypedHeader::Expires(Expires::new(expires)));
            // RFC 6665 §3.1.1 — Accept on SUBSCRIBE refresh advertises
            // body MIME types the subscriber accepts on NOTIFY.
            if let Some(accept_value) = accept {
                request.headers.push(TypedHeader::Other(
                    HeaderName::Accept,
                    HeaderValue::Raw(accept_value.into_bytes()),
                ));
            }
            // Pre-computed Digest / Bearer authorization. Required by
            // the 401 retry path on SUBSCRIBE refresh.
            if let Some(auth) = authorization {
                request.headers.push(TypedHeader::Other(
                    HeaderName::Authorization,
                    HeaderValue::Raw(auth.into_bytes()),
                ));
            }
            // SIP_API_DESIGN_2 §5.2 — append application extras after
            // the stack-managed prefix + dedicated setters (Event,
            // Expires, Accept, Authorization).
            for hdr in extra_headers {
                request.headers.push(hdr);
            }
            let destination = crate::dialog::dialog_utils::resolve_uri_to_socketaddr(
                &crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request),
            )
            .await
            .ok_or_else(|| {
                ApiError::protocol(format!(
                    "Failed to resolve SUBSCRIBE refresh URI: {}",
                    template.request_uri
                ))
            })?;
            (destination, request)
        };

        let transaction_id = self
            .core
            .transaction_manager()
            .create_non_invite_client_transaction(request, destination)
            .await
            .map_err(|e| {
                ApiError::internal(format!(
                    "Failed to create SUBSCRIBE refresh transaction: {}",
                    e
                ))
            })?;
        self.core
            .link_transaction_to_dialog_indexed(&transaction_id, dialog_id);
        self.core
            .transaction_manager()
            .send_request(&transaction_id)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to send SUBSCRIBE refresh: {}", e)))?;

        if expires == 0 {
            self.core
                .terminate_dialog(dialog_id)
                .await
                .map_err(ApiError::from)?;
        }
        Ok(())
    }

    /// Like [`make_call`](Self::make_call) but appends caller-supplied
    /// extra headers to the outgoing INVITE. Intended for headers session-core
    /// can't construct itself: P-Asserted-Identity / P-Preferred-Identity
    /// (RFC 3325) for trunk auth, P-Charging-Vector / X-headers for carrier
    /// integration, etc. Headers are appended verbatim — no validation is
    /// performed against the SIP method or dialog state.
    pub async fn make_call_with_extra_headers(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> ApiResult<CallHandle> {
        self.make_call_inner(from_uri, to_uri, sdp_offer, None, None, extra_headers)
            .await
    }

    /// `make_call_for_session` + extra headers. Use this from session-core
    /// layers that need both the pre-registered session↔dialog mapping and
    /// custom INVITE headers (the typical PAI use case).
    pub async fn make_call_with_extra_headers_for_session(
        &self,
        session_id: &str,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
        call_id: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> ApiResult<CallHandle> {
        self.make_call_inner(
            from_uri,
            to_uri,
            sdp_offer,
            call_id,
            Some(session_id.to_string()),
            extra_headers,
        )
        .await
    }

    async fn make_call_inner(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
        call_id: Option<String>,
        pre_register_session_id: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> ApiResult<CallHandle> {
        // Check if outgoing calls are supported
        if !self.config.supports_outgoing_calls() {
            error!(
                "Cannot make outgoing call: Outgoing calls not supported in {:?} mode",
                Self::mode_name(&self.config)
            );
            return Err(ApiError::Configuration {
                message: "Outgoing calls not supported in Server mode".to_string(),
            });
        }

        info!("Making outgoing call from {} to {}", from_uri, to_uri);

        // Parse URIs
        let from_uri: Uri = from_uri.parse().map_err(|e| {
            error!("Failed to parse from_uri '{}': {}", from_uri, e);
            ApiError::Configuration {
                message: format!("Invalid from_uri: {}", e),
            }
        })?;
        let to_uri: Uri = to_uri.parse().map_err(|e| {
            error!("Failed to parse to_uri '{}': {}", to_uri, e);
            ApiError::Configuration {
                message: format!("Invalid to_uri: {}", e),
            }
        })?;

        // Create outgoing dialog
        let dialog_id = self
            .core
            .create_outgoing_dialog(from_uri, to_uri, call_id)
            .await
            .map_err(|e| {
                error!("Failed to create outgoing dialog: {}", e);
                ApiError::from(e)
            })?;

        // Register the session↔dialog mapping BEFORE sending the INVITE.
        // Otherwise a sub-millisecond RTT failure response (e.g. localhost
        // 420) can race: the event-processor task may pick up the response
        // and try to route it to a session while the caller is still inside
        // this await, before the async `StoreDialogMapping` event has been
        // processed. Pre-registering closes that window with a write that
        // is ordered-before the INVITE goes on the wire.
        if let Some(ref sid) = pre_register_session_id {
            self.core
                .session_to_dialog
                .insert(sid.clone(), dialog_id.clone());
            self.core
                .dialog_to_session
                .insert(dialog_id.clone(), sid.clone());
        }

        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.outgoing_calls += 1;
            stats.active_dialogs += 1;
        }

        // Emit dialog creation event
        self.core
            .emit_dialog_event(DialogEvent::Created {
                dialog_id: dialog_id.clone(),
            })
            .await;

        // Send INVITE request. When the caller supplied extra headers
        // (P-Asserted-Identity etc.), route through the dedicated
        // `send_initial_invite_with_extra_headers` path so the headers ride
        // on the very first wire INVITE; otherwise the generic path is fine.
        let body_bytes = sdp_offer.map(|s| bytes::Bytes::from(s));
        let send_result = if extra_headers.is_empty() {
            self.core
                .send_request(&dialog_id, Method::Invite, body_bytes)
                .await
        } else {
            self.core
                .send_initial_invite_with_extra_headers(&dialog_id, body_bytes, extra_headers)
                .await
        };
        let _transaction_key = match send_result {
            Ok(tx_key) => tx_key,
            Err(e) => {
                // RFC 3261 Section 17.1.1.3: INVITE client transactions terminate after
                // receiving 2xx responses and sending ACK. This is normal behavior, not an error.
                let error_msg = e.to_string();
                if error_msg.contains("Transaction terminated after timeout")
                    || error_msg.contains("Transaction terminated")
                {
                    debug!(
                        "INVITE transaction terminated normally after 2xx response (RFC 3261 compliant): {}",
                        e
                    );
                    // This is expected behavior - the SIP call flow completed successfully
                    info!(
                        "Created outgoing call with dialog ID: {} (transaction completed per RFC 3261)",
                        dialog_id
                    );
                    return Ok(CallHandle::new(
                        dialog_id.clone(),
                        Arc::new(self.core.clone()),
                    ));
                }

                error!("Failed to send INVITE for call {}: {}", dialog_id, e);
                return Err(ApiError::from(e));
            }
        };

        // Create call handle
        let call_handle = CallHandle::new(dialog_id.clone(), Arc::new(self.core.clone()));

        info!(
            "Created outgoing call with dialog ID: {} and sent INVITE",
            dialog_id
        );

        Ok(call_handle)
    }

    /// Create an outgoing dialog without sending INVITE (Client/Hybrid modes only)
    ///
    /// Creates a dialog in preparation for sending requests. Useful for
    /// scenarios where you want to create the dialog before sending the INVITE.
    ///
    /// # Arguments
    /// * `from_uri` - Local URI
    /// * `to_uri` - Remote URI
    ///
    /// # Returns
    /// DialogHandle for the new dialog
    pub async fn create_dialog(&self, from_uri: &str, to_uri: &str) -> ApiResult<DialogHandle> {
        // Check if outgoing calls are supported
        if !self.config.supports_outgoing_calls() {
            error!(
                "Cannot create dialog: Dialog creation not supported in {:?} mode",
                Self::mode_name(&self.config)
            );
            return Err(ApiError::Configuration {
                message: "Dialog creation not supported in Server mode".to_string(),
            });
        }

        debug!("Creating outgoing dialog from {} to {}", from_uri, to_uri);

        // Parse URIs
        let from_uri: Uri = from_uri.parse().map_err(|e| {
            error!(
                "Failed to parse from_uri '{}' for dialog creation: {}",
                from_uri, e
            );
            ApiError::Configuration {
                message: format!("Invalid from_uri: {}", e),
            }
        })?;
        let to_uri: Uri = to_uri.parse().map_err(|e| {
            error!(
                "Failed to parse to_uri '{}' for dialog creation: {}",
                to_uri, e
            );
            ApiError::Configuration {
                message: format!("Invalid to_uri: {}", e),
            }
        })?;

        // Create outgoing dialog
        let dialog_id = self
            .core
            .create_outgoing_dialog(from_uri, to_uri, None)
            .await
            .map_err(|e| {
                error!("Failed to create outgoing dialog: {}", e);
                ApiError::from(e)
            })?;

        // Create dialog handle
        let handle = DialogHandle::new(dialog_id.clone(), Arc::new(self.core.clone()));

        debug!("Created dialog: {}", dialog_id);
        Ok(handle)
    }

    // ========================================
    // SERVER-MODE OPERATIONS
    // ========================================

    /// Handle incoming INVITE request (Server/Hybrid modes only)
    ///
    /// Processes an incoming INVITE to potentially establish a call.
    /// Only available in Server and Hybrid modes.
    ///
    /// # Arguments
    /// * `request` - The INVITE request
    /// * `source` - Source address of the request
    ///
    /// # Returns
    /// CallHandle for managing the incoming call
    pub async fn handle_invite(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> ApiResult<CallHandle> {
        // Check if incoming calls are supported
        if !self.config.supports_incoming_calls() {
            error!(
                "Cannot handle incoming INVITE: Incoming calls not supported in {:?} mode",
                Self::mode_name(&self.config)
            );
            return Err(ApiError::Configuration {
                message: "Incoming calls not supported in Client mode".to_string(),
            });
        }

        info!("Handling incoming INVITE from {}", source);

        // Process the INVITE through core dialog manager
        self.core
            .handle_invite(request.clone(), source)
            .await
            .map_err(|e| {
                error!("Failed to process incoming INVITE from {}: {}", source, e);
                ApiError::from(e)
            })?;

        // Find the dialog that was created for this INVITE
        let dialog_id = self
            .core
            .find_dialog_for_request(&request)
            .await
            .ok_or_else(|| {
                error!("Failed to find dialog for INVITE request from {}", source);
                ApiError::Dialog {
                    message: "Failed to find dialog for INVITE request".to_string(),
                }
            })?;

        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.incoming_calls += 1;
            stats.active_dialogs += 1;
        }

        // Create call handle
        let call_handle = CallHandle::new(dialog_id.clone(), Arc::new(self.core.clone()));

        info!("Created incoming call with dialog ID: {}", dialog_id);
        Ok(call_handle)
    }

    /// Send automatic response to OPTIONS request (Server/Hybrid modes)
    ///
    /// Automatically responds to OPTIONS requests if auto_options is enabled.
    /// This method is intended for future use when request routing is implemented.
    #[allow(dead_code)]
    async fn handle_auto_options(&self, request: Request, source: SocketAddr) -> ApiResult<()> {
        if !self.config.auto_options_enabled() {
            return Ok(()); // Not enabled, skip
        }

        info!("Sending automatic OPTIONS response to {}", source);

        // Process through core dialog manager
        self.core
            .handle_options(request, source)
            .await
            .map_err(ApiError::from)?;

        // Update stats
        {
            let mut stats = self.stats.write().await;
            stats.auto_responses += 1;
        }

        Ok(())
    }

    // ========================================
    // SHARED OPERATIONS (ALL MODES)
    // ========================================

    /// Send a request within an existing dialog
    ///
    /// Available in all modes for sending in-dialog requests.
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog to send the request in
    /// * `method` - SIP method to send
    /// * `body` - Optional request body
    ///
    /// # Returns
    /// Transaction key for tracking the request
    pub async fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> ApiResult<TransactionKey> {
        debug!("Sending {} request in dialog {}", method, dialog_id);

        let method_str = method.to_string(); // Convert to string before move
        self.core
            .send_request(dialog_id, method, body)
            .await
            .map_err(|e| {
                // Log SIP protocol validation errors as WARN (not ERROR) since they're often expected
                if e.to_string().contains("requires remote tag")
                    || e.to_string().contains("protocol error")
                {
                    warn!(
                        "SIP protocol validation failed for {} in dialog {}: {}",
                        method_str, dialog_id, e
                    );
                } else {
                    error!(
                        "Failed to send {} request in dialog {}: {}",
                        method_str, dialog_id, e
                    );
                }
                ApiError::from(e)
            })
    }

    /// Send an INFO request with a caller-chosen `Content-Type` (RFC 6086).
    ///
    /// The generic `send_request_in_dialog` path always tags INFO bodies as
    /// `application/info`. This method lets the caller specify the type —
    /// e.g. `application/dtmf-relay` for DTMF-over-INFO, `application/sipfrag`
    /// for fax flow control.
    pub async fn send_info_with_content_type(
        &self,
        dialog_id: &DialogId,
        content_type: String,
        body: bytes::Bytes,
    ) -> ApiResult<TransactionKey> {
        self.core
            .send_info_with_content_type(dialog_id, content_type, body)
            .await
            .map_err(ApiError::from)
    }

    /// Send a response to a transaction
    ///
    /// Available in all modes for sending responses to received requests.
    ///
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `response` - The response to send
    pub async fn send_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> ApiResult<()> {
        debug!("Sending response for transaction {}", transaction_id);

        self.core
            .send_response(transaction_id, response)
            .await
            .map_err(|e| {
                error!(
                    "Failed to send response for transaction {}: {}",
                    transaction_id, e
                );
                ApiError::from(e)
            })
    }

    /// Build a response for a transaction
    ///
    /// Constructs a properly formatted SIP response.
    ///
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - HTTP-style status code
    /// * `body` - Optional response body (SDP, error details, etc.)
    ///
    /// # Returns
    /// Constructed response ready to send
    pub async fn build_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        body: Option<String>,
    ) -> ApiResult<Response> {
        debug!(
            "Building response for transaction {} with status {}",
            status_code, transaction_id
        );

        // Get the original request from the transaction manager to copy required headers
        let original_request = self
            .core
            .transaction_manager()
            .original_request(transaction_id)
            .await
            .map_err(|e| ApiError::Internal {
                message: format!("Failed to get original request: {}", e),
            })?
            .ok_or_else(|| ApiError::Internal {
                message: "No original request found for transaction".to_string(),
            })?;

        let response_to_tag =
            self.ensure_uas_invite_response_tag(transaction_id, &original_request, status_code)?;

        // Use the proper response builder to create response with all required headers
        let mut response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            &original_request,
            status_code,
            None, // No custom reason phrase
        );

        // Add body if provided
        if let Some(body_content) = body {
            response = response.body(body_content.as_bytes().to_vec());

            // Set content type for SDP content
            if body_content.trim_start().starts_with("v=") {
                response = response.content_type("application/sdp");
            }
        }

        let mut built_response = response.build();
        if let Some(to_tag) = response_to_tag {
            set_response_to_tag(&mut built_response, &to_tag);
        }

        debug!(
            "Successfully built response for transaction {} using proper RFC 3261 compliant headers",
            transaction_id
        );
        Ok(built_response)
    }

    fn ensure_uas_invite_response_tag(
        &self,
        transaction_id: &TransactionKey,
        original_request: &Request,
        status_code: StatusCode,
    ) -> ApiResult<Option<String>> {
        let status = status_code.as_u16();
        if original_request.method() != Method::Invite
            || original_request.to().and_then(|to| to.tag()).is_some()
            || !(101..300).contains(&status)
        {
            return Ok(None);
        }

        let Some(dialog_id_ref) = self.core.transaction_to_dialog.get(transaction_id) else {
            return Ok(None);
        };
        let dialog_id = dialog_id_ref.clone();
        drop(dialog_id_ref);

        let (to_tag, tuple) = {
            let mut dialog = self.core.get_dialog_mut(&dialog_id)?;
            let to_tag = match dialog.local_tag.clone() {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(tag.clone());
                    tag
                }
            };

            (to_tag, dialog.dialog_id_tuple())
        };

        if let Some((call_id, local_tag, remote_tag)) = tuple {
            let key = crate::manager::utils::DialogUtils::create_lookup_key(
                &call_id,
                &local_tag,
                &remote_tag,
            );
            self.core.dialog_lookup.insert(key, dialog_id);
        }

        Ok(Some(to_tag))
    }

    /// Send a status response (convenience method)
    ///
    /// Builds and sends a simple status response.
    ///
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - Status code to send
    /// * `reason` - Optional reason phrase
    pub async fn send_status_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        _reason: Option<String>,
    ) -> ApiResult<()> {
        debug!(
            "Sending status response {} for transaction {}",
            status_code, transaction_id
        );

        let response = self
            .build_response(transaction_id, status_code, None)
            .await?;
        self.send_response(transaction_id, response).await
    }

    // ========================================
    // SIP METHOD HELPERS (ALL MODES)
    // ========================================

    /// Send BYE request to terminate a dialog
    pub async fn send_bye(&self, dialog_id: &DialogId) -> ApiResult<TransactionKey> {
        self.send_request_in_dialog(dialog_id, Method::Bye, None)
            .await
    }

    /// Send REFER request for call transfer
    pub async fn send_refer(
        &self,
        dialog_id: &DialogId,
        target_uri: String,
        refer_body: Option<String>,
    ) -> ApiResult<TransactionKey> {
        let body = if let Some(custom_body) = refer_body {
            custom_body
        } else {
            format!("Refer-To: {}\r\n", target_uri)
        };

        self.send_request_in_dialog(dialog_id, Method::Refer, Some(bytes::Bytes::from(body)))
            .await
    }

    /// Send NOTIFY request for event notifications
    pub async fn send_notify(
        &self,
        dialog_id: &DialogId,
        event: String,
        body: Option<String>,
        subscription_state: Option<String>,
    ) -> ApiResult<TransactionKey> {
        debug!(
            "Sending NOTIFY for event: {} with state: {:?}",
            event, subscription_state
        );

        // Update dialog's event_package and subscription_state before building request
        {
            let mut dialog = self.core.get_dialog_mut(dialog_id)?;

            // Set event package if not already set or if different
            if dialog.event_package.as_ref() != Some(&event) {
                dialog.event_package = Some(event.clone());
            }

            // Set subscription state if provided
            if let Some(state_str) = subscription_state {
                use crate::dialog::subscription_state::{
                    SubscriptionState, SubscriptionTerminationReason,
                };
                use std::time::Duration;

                // Parse simple subscription state strings to SubscriptionState enum
                let sub_state = if state_str.starts_with("active") {
                    // Extract expires value if present
                    let expires = if let Some(pos) = state_str.find("expires=") {
                        let exp_str = &state_str[pos + 8..];
                        exp_str
                            .split(';')
                            .next()
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(3600)
                    } else {
                        3600
                    };
                    SubscriptionState::Active {
                        remaining_duration: Duration::from_secs(expires),
                        original_duration: Duration::from_secs(expires),
                    }
                } else if state_str.starts_with("pending") {
                    SubscriptionState::Pending
                } else if state_str.starts_with("terminated") {
                    // Extract reason if present
                    let reason = if state_str.contains("noresource") {
                        Some(SubscriptionTerminationReason::NoResource)
                    } else if state_str.contains("deactivated") {
                        Some(SubscriptionTerminationReason::ClientRequested)
                    } else if state_str.contains("rejected") {
                        Some(SubscriptionTerminationReason::Rejected)
                    } else if state_str.contains("timeout") {
                        Some(SubscriptionTerminationReason::Expired)
                    } else {
                        None
                    };
                    SubscriptionState::Terminated { reason }
                } else {
                    // Default to terminated if can't parse
                    SubscriptionState::Terminated { reason: None }
                };

                dialog.subscription_state = Some(sub_state);
            }
        }

        let notify_body = body.map(|b| bytes::Bytes::from(b));
        self.send_request_in_dialog(dialog_id, Method::Notify, notify_body)
            .await
    }

    /// Send NOTIFY for REFER implicit subscription (RFC 3515)
    ///
    /// Automatically sets Event: refer and appropriate Subscription-State based on status code
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog with the implicit REFER subscription
    /// * `status_code` - SIP status code to report (100, 180, 200, etc.)
    /// * `reason` - Reason phrase for the status
    pub async fn send_refer_notify(
        &self,
        dialog_id: &DialogId,
        status_code: u16,
        reason: &str,
    ) -> ApiResult<TransactionKey> {
        // RFC 3515: REFER creates implicit subscription that terminates after final response
        let subscription_state = if status_code >= 200 {
            "terminated;reason=noresource".to_string() // Final response terminates subscription
        } else {
            "active;expires=60".to_string() // Provisional response keeps subscription active
        };

        // Body is sipfrag format per RFC 3515
        let sipfrag_body = format!("SIP/2.0 {} {}", status_code, reason);

        self.send_notify(
            dialog_id,
            "refer".to_string(),
            Some(sipfrag_body),
            Some(subscription_state),
        )
        .await
    }

    /// Send UPDATE request for media modifications
    pub async fn send_update(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>,
    ) -> ApiResult<TransactionKey> {
        let update_body = sdp.map(|s| bytes::Bytes::from(s));
        self.send_request_in_dialog(dialog_id, Method::Update, update_body)
            .await
    }

    /// Send PRACK for a reliable provisional response (RFC 3262).
    ///
    /// `rseq` is the `RSeq` value of the 18x being acknowledged. The dialog
    /// must already have its `invite_cseq` recorded (set on initial INVITE)
    /// and must have a remote tag (established by the reliable 18x itself).
    pub async fn send_prack(&self, dialog_id: &DialogId, rseq: u32) -> ApiResult<TransactionKey> {
        self.core.send_prack(dialog_id, rseq).await.map_err(|e| {
            error!(
                "Failed to send PRACK for dialog {} (RSeq={}): {}",
                dialog_id, rseq, e
            );
            ApiError::from(e)
        })
    }

    /// Send INFO request for application-specific information
    pub async fn send_info(
        &self,
        dialog_id: &DialogId,
        info_body: String,
    ) -> ApiResult<TransactionKey> {
        self.send_request_in_dialog(dialog_id, Method::Info, Some(bytes::Bytes::from(info_body)))
            .await
    }

    /// RFC 3261 §22.2 — resend an INVITE with a digest authorization header
    /// after a 401/407 challenge. Reuses the existing dialog's Call-ID and
    /// From tag, bumps CSeq on a new client transaction, preserves SDP, and
    /// attaches the caller-supplied `Authorization` (401) or
    /// `Proxy-Authorization` (407) header value verbatim.
    pub async fn send_invite_with_auth(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>,
        auth_header_name: &str,
        auth_header_value: String,
        extras: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> ApiResult<TransactionKey> {
        let body = sdp.map(bytes::Bytes::from);
        self.core
            .send_invite_with_auth(dialog_id, body, auth_header_name, auth_header_value, extras)
            .await
            .map_err(ApiError::from)
    }

    /// RFC 4028 §6 — resend an INVITE with a per-call `Session-Expires` /
    /// `Min-SE` override after a 422 Session Interval Too Small. The timer
    /// headers on the retry bypass [`DialogManagerConfig`]'s global values
    /// and use the supplied overrides instead — typically derived from the
    /// UAS's Min-SE header on the 422 response.
    pub async fn send_invite_with_session_timer_override(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>,
        session_secs: u32,
        min_se: u32,
    ) -> ApiResult<TransactionKey> {
        let body = sdp.map(bytes::Bytes::from);
        self.core
            .send_invite_with_session_timer_override(dialog_id, body, session_secs, min_se)
            .await
            .map_err(ApiError::from)
    }

    /// Send CANCEL request to cancel a pending INVITE
    ///
    /// This method cancels a pending INVITE transaction that hasn't received a final response.
    /// Only works for dialogs in the Early or Initial state (before 200 OK is received).
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog to cancel
    ///
    /// # Returns
    /// Transaction key for the CANCEL request
    ///
    /// # Errors
    /// Returns an error if:
    /// - Dialog is not found
    /// - Dialog is not in Early or Initial state
    /// - No pending INVITE transaction found
    pub async fn send_cancel(&self, dialog_id: &DialogId) -> ApiResult<TransactionKey> {
        self.send_cancel_with_extras(dialog_id, Vec::new()).await
    }

    /// CANCEL with caller-supplied `extra_headers` appended to the
    /// generated CANCEL after the RFC 3261 §9.1 mandatory header copy
    /// (From / To / Call-ID / CSeq-num / Max-Forwards / Via /
    /// optionally Route). The new CANCEL transaction is created and
    /// sent on the same destination as the targeted INVITE.
    pub async fn send_cancel_with_extras(
        &self,
        dialog_id: &DialogId,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> ApiResult<TransactionKey> {
        // Get the dialog state to verify it can be cancelled
        let dialog_state = self.get_dialog_state(dialog_id).await?;

        match dialog_state {
            DialogState::Initial | DialogState::Early => {
                info!(
                    "Sending CANCEL for dialog {} in state {:?}",
                    dialog_id, dialog_state
                );
            }
            _ => {
                error!(
                    "Cannot send CANCEL for dialog {} in state {:?} - must be in Initial or Early state",
                    dialog_id, dialog_state
                );
                return Err(ApiError::Protocol {
                    message: format!("Cannot cancel dialog in state {:?}", dialog_state),
                });
            }
        }

        // Find the currently pending outbound INVITE transaction for this dialog.
        // Auth/session-timer retries create newer INVITE transactions under the
        // same dialog, and RFC 3261 CANCEL must target that latest transaction.
        let invite_tx_id = self
            .core
            .find_latest_invite_transaction_for_dialog(dialog_id)
            .await
            .ok_or_else(|| {
                error!("No INVITE transaction found for dialog {}", dialog_id);
                ApiError::Protocol {
                    message: "No INVITE transaction found to cancel".to_string(),
                }
            })?;

        // Cancel the INVITE transaction. Application extras ride
        // alongside the RFC 3261-mandated copies — appended after the
        // stack-managed slice per §5.2.
        let cancel_tx_id = self
            .core
            .cancel_invite_transaction_with_dialog_and_extras(&invite_tx_id, extra_headers)
            .await
            .map_err(|e| {
                error!(
                    "Failed to cancel INVITE transaction {} for dialog {}: {}",
                    invite_tx_id, dialog_id, e
                );
                ApiError::from(e)
            })?;

        info!(
            "Successfully sent CANCEL (tx: {}) for dialog {}",
            cancel_tx_id, dialog_id
        );
        Ok(cancel_tx_id)
    }

    // ========================================
    // DIALOG MANAGEMENT (ALL MODES)
    // ========================================

    /// Get information about a dialog
    pub async fn get_dialog_info(&self, dialog_id: &DialogId) -> ApiResult<Dialog> {
        self.core.get_dialog(dialog_id).map_err(|e| {
            warn!("Failed to get dialog info for {}: {}", dialog_id, e);
            ApiError::from(e)
        })
    }

    /// Get the current state of a dialog
    pub async fn get_dialog_state(&self, dialog_id: &DialogId) -> ApiResult<DialogState> {
        self.core.get_dialog_state(dialog_id).map_err(|e| {
            warn!("Failed to get dialog state for {}: {}", dialog_id, e);
            ApiError::from(e)
        })
    }

    /// Terminate a dialog
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> ApiResult<()> {
        info!("Terminating dialog {}", dialog_id);
        self.core.terminate_dialog(dialog_id).await.map_err(|e| {
            error!("Failed to terminate dialog {}: {}", dialog_id, e);
            ApiError::from(e)
        })
    }

    /// List all active dialogs
    pub async fn list_active_dialogs(&self) -> Vec<DialogId> {
        self.core.list_dialogs()
    }

    /// Get statistics for this manager
    pub async fn get_stats(&self) -> ManagerStats {
        let stats = self.stats.read().await;
        ManagerStats {
            active_dialogs: self.core.dialog_count(),
            total_dialogs: stats.total_dialogs,
            successful_calls: stats.successful_calls,
            failed_calls: stats.failed_calls,
            total_call_duration: stats.total_call_duration,
            outgoing_calls: stats.outgoing_calls,
            incoming_calls: stats.incoming_calls,
            auth_challenges: stats.auth_challenges,
            auto_responses: stats.auto_responses,
        }
    }

    /// Send ACK for 2xx response to INVITE
    ///
    /// Handles the automatic ACK sending required by RFC 3261 for 200 OK responses to INVITE.
    /// Available in all modes for proper SIP protocol compliance.
    ///
    /// # Arguments
    /// * `dialog_id` - Dialog ID for the call
    /// * `original_invite_tx_id` - Transaction ID of the original INVITE
    /// * `response` - The 200 OK response to acknowledge
    ///
    /// # Returns
    /// Success or error
    pub async fn send_ack_for_2xx_response(
        &self,
        dialog_id: &DialogId,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> ApiResult<()> {
        debug!(
            "Sending ACK for 2xx response for dialog {} via unified API",
            dialog_id
        );

        self.core
            .send_ack_for_2xx_response(dialog_id, original_invite_tx_id, response)
            .await
            .map_err(|e| {
                error!(
                    "Failed to send ACK for 2xx response for dialog {}: {}",
                    dialog_id, e
                );
                ApiError::from(e)
            })
    }
}
