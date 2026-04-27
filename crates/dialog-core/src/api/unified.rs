//! Unified API for DialogManager
//!
//! This module provides a unified, high-level API that replaces the separate
//! DialogClient and DialogServer APIs with a single, comprehensive interface.
//! The behavior is determined by the DialogManagerConfig provided during construction.
//!
//! ## Overview
//!
//! The unified API eliminates the artificial client/server split while maintaining
//! all functionality from both previous APIs. The UnifiedDialogApi provides:
//!
//! - **All Client Operations**: `make_call`, outgoing dialog creation, authentication
//! - **All Server Operations**: `handle_invite`, auto-responses, incoming call handling
//! - **All Shared Operations**: Dialog management, response building, SIP method helpers
//! - **Session Coordination**: Integration with session-core for media management
//! - **Statistics & Monitoring**: Comprehensive metrics and dialog state tracking
//!
//! ## Architecture
//!
//! ```text
//! UnifiedDialogApi
//!        │
//!        ├── Configuration-based behavior
//!        │   ├── Client mode: make_call, create_dialog, auth
//!        │   ├── Server mode: handle_invite, auto-options, domain
//!        │   └── Hybrid mode: all operations available
//!        │
//!        ├── Shared operations (all modes)
//!        │   ├── Dialog management
//!        │   ├── Response building
//!        │   ├── SIP method helpers (BYE, REFER, etc.)
//!        │   └── Session coordination
//!        │
//!        └── Convenience handles
//!            ├── DialogHandle (dialog operations)
//!            └── CallHandle (call-specific operations)
//! ```
//!
//! ## Examples
//!
//! ### Client Mode Usage
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::unified::UnifiedDialogApi;
//! use rvoip_dialog_core::config::DialogManagerConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
//!     .with_from_uri("sip:alice@example.com")
//!     .with_auth("alice", "secret123")
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let api = UnifiedDialogApi::new(transaction_manager, config).await?;
//! api.start().await?;
//!
//! // Make outgoing calls
//! let call = api.make_call(
//!     "sip:alice@example.com",
//!     "sip:bob@example.com",
//!     Some("SDP offer".to_string())
//! ).await?;
//!
//! // Use call operations
//! call.hold(Some("SDP with hold".to_string())).await?;
//! call.transfer("sip:voicemail@example.com".to_string()).await?;
//! call.hangup().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Server Mode Usage
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::unified::UnifiedDialogApi;
//! use rvoip_dialog_core::config::DialogManagerConfig;
//! use rvoip_dialog_core::events::SessionCoordinationEvent;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DialogManagerConfig::server("0.0.0.0:5060".parse()?)
//!     .with_domain("sip.company.com")
//!     .with_auto_options()
//!     .build();
//!
//! # let transaction_manager = std::sync::Arc::new(unimplemented!());
//! let api = UnifiedDialogApi::new(transaction_manager, config).await?;
//!
//! // Session coordination events now flow via GlobalEventCoordinator;
//! // the channel below is illustrative only.
//! let (_session_tx, mut session_rx) =
//!     tokio::sync::mpsc::channel::<SessionCoordinationEvent>(100);
//! api.start().await?;
//!
//! // Handle incoming calls
//! tokio::spawn(async move {
//!     while let Some(event) = session_rx.recv().await {
//!         match event {
//!             SessionCoordinationEvent::IncomingCall { dialog_id, request, .. } => {
//!                 // Handle the incoming call
//!                 # let source_addr = "127.0.0.1:5060".parse().unwrap();
//!                 if let Ok(call) = api.handle_invite(request, source_addr).await {
//!                     call.answer(Some("SDP answer".to_string())).await.ok();
//!                 }
//!             },
//!             _ => {}
//!         }
//!     }
//! });
//! # Ok(())
//! # }
//! ```
//!
//! ### Hybrid Mode Usage
//!
//! ```rust,no_run
//! use rvoip_dialog_core::api::unified::UnifiedDialogApi;
//! use rvoip_dialog_core::config::DialogManagerConfig;
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
//! let api = UnifiedDialogApi::new(transaction_manager, config).await?;
//! api.start().await?;
//!
//! // Can both make outgoing calls AND handle incoming calls
//! let outgoing_call = api.make_call(
//!     "sip:pbx@company.com",
//!     "sip:external@provider.com",
//!     None
//! ).await?;
//!
//! // Also handles incoming calls via session coordination
//! # Ok(())
//! # }
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::transaction::{TransactionEvent, TransactionKey, TransactionManager};
use rvoip_sip_core::{Method, Request, Response, StatusCode};

use super::{
    common::{CallHandle, DialogHandle},
    ApiError, ApiResult, DialogStats,
};
use crate::config::DialogManagerConfig;
use crate::dialog::{Dialog, DialogId, DialogState};
use crate::events::{DialogEvent, SessionCoordinationEvent};
use crate::manager::unified::UnifiedDialogManager;

/// Unified Dialog API
///
/// Provides a comprehensive, high-level interface for SIP dialog management
/// that combines all functionality from the previous DialogClient and DialogServer
/// APIs into a single, configuration-driven interface.
///
/// ## Key Features
///
/// - **Mode-based behavior**: Client, Server, or Hybrid operation based on configuration
/// - **Complete SIP support**: All SIP methods and dialog operations
/// - **Session integration**: Built-in coordination with session-core
/// - **Convenience handles**: DialogHandle and CallHandle for easy operation
/// - **Comprehensive monitoring**: Statistics, events, and state tracking
/// - **Thread safety**: Safe to share across async tasks using Arc
///
/// ## Capabilities by Mode
///
/// ### Client Mode
/// - Make outgoing calls (`make_call`)
/// - Create outgoing dialogs (`create_dialog`)
/// - Handle authentication challenges
/// - Send in-dialog requests
/// - Build and send responses (when needed)
///
/// ### Server Mode
/// - Handle incoming calls (`handle_invite`)
/// - Auto-respond to OPTIONS/REGISTER (if configured)
/// - Build and send responses
/// - Send in-dialog requests
/// - Domain-based routing
///
/// ### Hybrid Mode
/// - All client capabilities
/// - All server capabilities
/// - Full bidirectional SIP support
/// - Complete PBX/gateway functionality
#[derive(Debug, Clone)]
pub struct UnifiedDialogApi {
    /// Underlying unified dialog manager
    manager: Arc<UnifiedDialogManager>,

    /// Configuration for this API instance
    config: DialogManagerConfig,
}

/// Options for constructing a non-dialog REGISTER request.
#[derive(Debug, Clone)]
pub struct RegisterRequestOptions {
    pub registrar_uri: String,
    pub aor_uri: String,
    pub contact_uri: String,
    pub expires: u32,
    pub authorization: Option<String>,
    pub call_id: Option<String>,
    pub cseq: Option<u32>,
    pub outbound_contact: Option<rvoip_sip_core::types::outbound::OutboundContactParams>,
}

/// Build a RFC 5626 outbound-aware Contact header from a raw URI string and
/// the supplied outbound parameters. The URI receives the `;ob` flag per
/// §5.4; the Contact receives `+sip.instance` + `reg-id` per §4.1/4.2.
///
/// Pure / sync so it's trivially unit-testable against the Contact's
/// rendered string form.
pub(crate) fn build_outbound_contact(
    contact_uri: &str,
    outbound_params: &rvoip_sip_core::types::outbound::OutboundContactParams,
) -> Result<rvoip_sip_core::types::contact::Contact, rvoip_sip_core::error::Error> {
    use rvoip_sip_core::types::{
        contact::{Contact, ContactParamInfo},
        outbound::{mark_uri_as_outbound, set_outbound_contact_params},
        uri::Uri,
        Address,
    };
    use std::str::FromStr;
    let uri = Uri::from_str(contact_uri)?;
    let mut address = Address::new(uri);
    mark_uri_as_outbound(&mut address);
    set_outbound_contact_params(&mut address, outbound_params);
    Ok(Contact::new_params(vec![ContactParamInfo { address }]))
}

impl UnifiedDialogApi {
    /// Create a new unified dialog API
    ///
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `config` - Configuration determining the behavior mode
    ///
    /// # Returns
    /// New UnifiedDialogApi instance
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::unified::UnifiedDialogApi;
    /// use rvoip_dialog_core::config::DialogManagerConfig;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
    ///     .with_from_uri("sip:alice@example.com")
    ///     .with_auth("alice", "secret123")
    ///     .build();
    ///
    /// # let transaction_manager = std::sync::Arc::new(unimplemented!());
    /// let api = UnifiedDialogApi::new(transaction_manager, config).await?;
    /// api.start().await?;
    ///
    /// // Make outgoing calls
    /// let call = api.make_call(
    ///     "sip:alice@example.com",
    ///     "sip:bob@example.com",
    ///     Some("SDP offer".to_string())
    /// ).await?;
    ///
    /// // Use call operations
    /// call.hold(Some("SDP with hold".to_string())).await?;
    /// call.transfer("sip:voicemail@example.com".to_string()).await?;
    /// call.hangup().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        config: DialogManagerConfig,
    ) -> ApiResult<Self> {
        info!(
            "Creating UnifiedDialogApi in {:?} mode",
            Self::mode_name(&config)
        );

        let manager = Arc::new(
            UnifiedDialogManager::new(transaction_manager, config.clone())
                .await
                .map_err(ApiError::from)?,
        );

        Ok(Self { manager, config })
    }

    /// Create a new unified dialog API with global event coordination
    pub async fn new_with_event_coordinator(
        transaction_manager: Arc<TransactionManager>,
        config: DialogManagerConfig,
        global_coordinator: Arc<rvoip_infra_common::events::coordinator::GlobalEventCoordinator>,
    ) -> ApiResult<Self> {
        info!(
            "Creating UnifiedDialogApi with global event coordination in {:?} mode",
            Self::mode_name(&config)
        );

        let manager = Arc::new(
            UnifiedDialogManager::new(transaction_manager, config.clone())
                .await
                .map_err(ApiError::from)?,
        );

        // Create and set up the event hub
        let event_hub = crate::events::DialogEventHub::new(
            global_coordinator,
            Arc::new(manager.as_ref().inner_manager().clone()),
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to create event hub: {}", e)))?;

        // Set the event hub on the dialog manager
        manager
            .as_ref()
            .inner_manager()
            .set_event_hub(event_hub)
            .await;

        Ok(Self { manager, config })
    }

    /// Create a new unified dialog API with global events AND event coordination
    pub async fn with_global_events_and_coordinator(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        config: DialogManagerConfig,
        global_coordinator: Arc<rvoip_infra_common::events::coordinator::GlobalEventCoordinator>,
    ) -> ApiResult<Self> {
        info!(
            "Creating UnifiedDialogApi with global events and event coordination in {:?} mode",
            Self::mode_name(&config)
        );

        // Create the manager with global events
        let manager = Arc::new(
            UnifiedDialogManager::with_global_events(
                transaction_manager,
                transaction_events,
                config.clone(),
            )
            .await
            .map_err(ApiError::from)?,
        );

        // Create and set up the event hub
        let event_hub = crate::events::DialogEventHub::new(
            global_coordinator,
            Arc::new(manager.as_ref().inner_manager().clone()),
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to create event hub: {}", e)))?;

        // Set the event hub on the dialog manager
        manager
            .as_ref()
            .inner_manager()
            .set_event_hub(event_hub)
            .await;

        Ok(Self { manager, config })
    }

    /// Create a new unified dialog API with global events (RECOMMENDED)
    ///
    /// # Arguments
    /// * `transaction_manager` - Pre-configured transaction manager
    /// * `transaction_events` - Global transaction event receiver
    /// * `config` - Configuration determining the behavior mode
    ///
    /// # Returns
    /// New UnifiedDialogApi instance with proper event consumption
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::unified::UnifiedDialogApi;
    /// use rvoip_dialog_core::config::DialogManagerConfig;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let transaction_manager = std::sync::Arc::new(unimplemented!());
    /// # let transaction_events = tokio::sync::mpsc::channel(100).1;
    /// let config = DialogManagerConfig::server("0.0.0.0:5060".parse()?)
    ///     .with_domain("sip.company.com")
    ///     .build();
    ///
    /// let api = UnifiedDialogApi::with_global_events(
    ///     transaction_manager,
    ///     transaction_events,
    ///     config
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        config: DialogManagerConfig,
    ) -> ApiResult<Self> {
        info!(
            "Creating UnifiedDialogApi with global events in {:?} mode",
            Self::mode_name(&config)
        );

        let manager = Arc::new(
            UnifiedDialogManager::with_global_events(
                transaction_manager,
                transaction_events,
                config.clone(),
            )
            .await
            .map_err(ApiError::from)?,
        );

        Ok(Self { manager, config })
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

    /// Get the underlying dialog manager
    ///
    /// Provides access to the underlying UnifiedDialogManager for advanced operations.
    pub fn dialog_manager(&self) -> &Arc<UnifiedDialogManager> {
        &self.manager
    }

    /// Get reference to the subscription manager if configured
    pub fn subscription_manager(&self) -> Option<&Arc<crate::subscription::SubscriptionManager>> {
        self.manager.subscription_manager()
    }

    // ========================================
    // LIFECYCLE MANAGEMENT
    // ========================================

    /// Start the dialog API
    ///
    /// Initializes the API for processing SIP messages and events.
    pub async fn start(&self) -> ApiResult<()> {
        info!("Starting UnifiedDialogApi");
        self.manager.start().await.map_err(ApiError::from)
    }

    /// Stop the dialog API
    ///
    /// Gracefully shuts down the API and all active dialogs.
    pub async fn stop(&self) -> ApiResult<()> {
        info!("Stopping UnifiedDialogApi");
        self.manager.stop().await.map_err(ApiError::from)
    }

    // ========================================
    // SESSION COORDINATION
    // ========================================
    //
    // REMOVED: set_session_coordinator() / set_dialog_event_sender() /
    // subscribe_to_dialog_events() — use GlobalEventCoordinator instead.
    // Wire the coordinator via `with_global_events(...)` at construction time
    // and receive events through the coordinator's broadcast channels.

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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(api: rvoip_dialog_core::api::unified::UnifiedDialogApi) -> Result<(), Box<dyn std::error::Error>> {
    /// let call = api.make_call(
    ///     "sip:alice@example.com",
    ///     "sip:bob@example.com",
    ///     Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\n...".to_string())
    /// ).await?;
    ///
    /// println!("Call created: {}", call.call_id());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn make_call(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
    ) -> ApiResult<CallHandle> {
        self.manager.make_call(from_uri, to_uri, sdp_offer).await
    }

    /// Make an outgoing call with a specific Call-ID
    ///
    /// Like `make_call` but allows specifying the Call-ID to use for the SIP dialog.
    /// This is useful when the call originator needs to control the Call-ID.
    ///
    /// # Arguments
    /// * `from_uri` - The calling party's SIP URI
    /// * `to_uri` - The called party's SIP URI
    /// * `sdp_offer` - Optional SDP offer for media negotiation
    /// * `call_id` - Optional Call-ID to use (will be generated if None)
    ///
    /// # Returns
    /// A `CallHandle` for controlling the established call
    pub async fn make_call_with_id(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
        call_id: Option<String>,
    ) -> ApiResult<CallHandle> {
        self.manager
            .make_call_with_id(from_uri, to_uri, sdp_offer, call_id)
            .await
    }

    /// Send an INVITE and pre-register the given session↔dialog mapping
    /// before the INVITE goes on the wire. Use this from session-core layers
    /// to close the race where a fast-RTT failure response (e.g. 420 on
    /// localhost) arrives before the mapping has been populated and gets
    /// dropped by the event-hub converter.
    pub async fn make_call_for_session(
        &self,
        session_id: &str,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
        call_id: Option<String>,
    ) -> ApiResult<CallHandle> {
        self.manager
            .make_call_for_session(session_id, from_uri, to_uri, sdp_offer, call_id)
            .await
    }

    /// Send an INVITE with caller-supplied extra headers riding on the very
    /// first wire transmission. Used for headers the dialog layer can't infer
    /// from the dialog state alone — most commonly:
    ///
    /// - `TypedHeader::PAssertedIdentity(...)` (RFC 3325) for trunk-asserted identity
    /// - `TypedHeader::PPreferredIdentity(...)` (RFC 3325) for caller preference
    ///
    /// Headers are appended verbatim — no validation against method/dialog state.
    /// session-core constructs the typed PAI from `Config::pai_uri` and
    /// reaches this entry point via `DialogAdapter::make_call_with_pai`.
    pub async fn make_call_with_extra_headers(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> ApiResult<CallHandle> {
        self.manager
            .make_call_with_extra_headers(from_uri, to_uri, sdp_offer, extra_headers)
            .await
    }

    /// `make_call_for_session` + extra headers. The session↔dialog mapping
    /// is pre-registered before the INVITE goes on the wire (closes the
    /// fast-RTT race for very fast localhost responses), and the supplied
    /// extras (typically PAI) ride on the first transmission.
    pub async fn make_call_with_extra_headers_for_session(
        &self,
        session_id: &str,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
        call_id: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> ApiResult<CallHandle> {
        self.manager
            .make_call_with_extra_headers_for_session(
                session_id,
                from_uri,
                to_uri,
                sdp_offer,
                call_id,
                extra_headers,
            )
            .await
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(api: rvoip_dialog_core::api::unified::UnifiedDialogApi) -> Result<(), Box<dyn std::error::Error>> {
    /// let dialog = api.create_dialog("sip:alice@example.com", "sip:bob@example.com").await?;
    ///
    /// // Send custom requests within the dialog
    /// dialog.send_info("Custom application data".to_string()).await?;
    /// dialog.send_notify("presence".to_string(), Some("online".to_string())).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_dialog(&self, from_uri: &str, to_uri: &str) -> ApiResult<DialogHandle> {
        self.manager.create_dialog(from_uri, to_uri).await
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
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example(api: rvoip_dialog_core::api::unified::UnifiedDialogApi, request: rvoip_sip_core::Request) -> Result<(), Box<dyn std::error::Error>> {
    /// let source = "192.168.1.100:5060".parse().unwrap();
    /// let call = api.handle_invite(request, source).await?;
    ///
    /// // Accept the call
    /// call.answer(Some("v=0\r\no=server 789 012 IN IP4 192.168.1.10\r\n...".to_string())).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn handle_invite(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> ApiResult<CallHandle> {
        self.manager.handle_invite(request, source).await
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
        self.manager
            .send_request_in_dialog(dialog_id, method, body)
            .await
    }

    /// Send an INFO request (RFC 6086) with a caller-chosen `Content-Type`.
    ///
    /// The generic `send_request_in_dialog` path stamps INFO bodies with
    /// `application/info`. This method lets the caller specify the type —
    /// e.g. `application/dtmf-relay` for DTMF-over-INFO,
    /// `application/sipfrag` for fax flow control.
    pub async fn send_info_with_content_type(
        &self,
        dialog_id: &DialogId,
        content_type: String,
        body: bytes::Bytes,
    ) -> ApiResult<TransactionKey> {
        self.manager
            .send_info_with_content_type(dialog_id, content_type, body)
            .await
    }

    /// RFC 3261 §22.2 — resend an INVITE with digest auth after a 401/407
    /// challenge. Session-core-v3 uses this to transparently retry call setup
    /// when the UAS or proxy challenged the original INVITE.
    pub async fn send_invite_with_auth(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> ApiResult<TransactionKey> {
        self.manager
            .send_invite_with_auth(dialog_id, sdp, auth_header_name, auth_header_value)
            .await
    }

    /// RFC 4028 §6 — resend an INVITE with a per-call `Session-Expires` /
    /// `Min-SE` override after a 422 Session Interval Too Small. The timer
    /// headers on the retry bypass [`DialogManagerConfig`]'s global values
    /// and use the supplied overrides instead — typically with `session_secs`
    /// and `min_se` both set to the UAS's required Min-SE floor.
    pub async fn send_invite_with_session_timer_override(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>,
        session_secs: u32,
        min_se: u32,
    ) -> ApiResult<TransactionKey> {
        self.manager
            .send_invite_with_session_timer_override(dialog_id, sdp, session_secs, min_se)
            .await
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
        self.manager.send_response(transaction_id, response).await
    }

    /// Build a response for a transaction
    ///
    /// Constructs a properly formatted SIP response.
    ///
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `status_code` - HTTP-style status code
    /// * `body` - Optional response body
    ///
    /// # Returns
    /// Constructed response ready to send
    pub async fn build_response(
        &self,
        transaction_id: &TransactionKey,
        status_code: StatusCode,
        body: Option<String>,
    ) -> ApiResult<Response> {
        self.manager
            .build_response(transaction_id, status_code, body)
            .await
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
        reason: Option<String>,
    ) -> ApiResult<()> {
        self.manager
            .send_status_response(transaction_id, status_code, reason)
            .await
    }

    /// Send a 3xx redirect response for a session with one or more Contact
    /// URIs (RFC 3261 §8.1.3.4 / §21.3).
    ///
    /// `status_code` should be in 300..=399 (300 Multiple Choices, 301 Moved
    /// Permanently, 302 Moved Temporarily, 305 Use Proxy, 380 Alternative
    /// Service). Each entry in `contacts` is parsed as a SIP URI and added as
    /// a separate `ContactParamInfo` in a single `Contact:` header so the UAC
    /// can choose among alternatives per RFC 3261 §8.1.3.4.
    pub async fn send_redirect_response_for_session(
        &self,
        session_id: &str,
        status_code: u16,
        contacts: Vec<String>,
    ) -> ApiResult<()> {
        use rvoip_sip_core::types::{
            address::Address,
            contact::{Contact, ContactParamInfo},
            uri::Uri,
            TypedHeader,
        };
        use std::str::FromStr;

        if !(300..=399).contains(&status_code) {
            return Err(ApiError::Internal {
                message: format!(
                    "send_redirect_response_for_session: status {} is not 3xx",
                    status_code
                ),
            });
        }
        if contacts.is_empty() {
            return Err(ApiError::Internal {
                message: "send_redirect_response_for_session: no Contact URIs supplied".to_string(),
            });
        }

        // Look up the dialog + transaction the same way send_response_for_session does.
        let dialog_id = self
            .manager
            .core()
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| ApiError::Dialog {
                message: format!("No dialog found for session {}", session_id),
            })?
            .clone();
        let transaction_id = self
            .manager
            .core()
            .transaction_to_dialog
            .iter()
            .find(|entry| entry.value() == &dialog_id)
            .map(|entry| entry.key().clone())
            .ok_or_else(|| ApiError::Dialog {
                message: format!("No transaction found for dialog {}", dialog_id),
            })?;

        let status = StatusCode::from_u16(status_code).unwrap_or(StatusCode::MovedTemporarily);
        let mut response = self.build_response(&transaction_id, status, None).await?;

        // Build Contact: uri1, uri2, ... as a single header with multiple params.
        let mut params: Vec<ContactParamInfo> = Vec::with_capacity(contacts.len());
        for raw in &contacts {
            let uri = Uri::from_str(raw).map_err(|e| ApiError::Internal {
                message: format!("Invalid Contact URI {:?}: {}", raw, e),
            })?;
            params.push(ContactParamInfo {
                address: Address::new(uri),
            });
        }
        response
            .headers
            .push(TypedHeader::Contact(Contact::new_params(params)));

        info!(
            "Sending {} redirect response for session {} via transaction {} with {} contact(s)",
            status_code,
            session_id,
            transaction_id,
            contacts.len()
        );

        self.send_response(&transaction_id, response).await
    }

    /// Send a response for a session (session-core convenience method)
    ///
    /// Allows session-core to send responses without knowing transaction details.
    /// Dialog-core will look up the appropriate transaction for the session.
    ///
    /// # Arguments
    /// * `session_id` - Session ID to respond for
    /// * `status_code` - Status code to send
    /// * `body` - Optional response body (e.g., SDP)
    pub async fn send_response_for_session(
        &self,
        session_id: &str,
        status_code: u16,
        body: Option<String>,
    ) -> ApiResult<()> {
        debug!(
            "send_response_for_session called for session {} with status {}",
            session_id, status_code
        );

        // Look up the dialog ID for this session
        let dialog_id = self
            .manager
            .core()
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| {
                error!("No dialog found for session {}", session_id);
                ApiError::Dialog {
                    message: format!("No dialog found for session {}", session_id),
                }
            })?
            .clone();

        debug!("Found dialog {} for session {}", dialog_id, session_id);

        // Find the *pending* server transaction for this dialog.
        //
        // `transaction_to_dialog` is many-to-one: an established dialog has
        // one INVITE server-tx (retained for retransmission) plus any later
        // mid-dialog UAS-tx such as UPDATE/re-INVITE. Responding to the
        // first match is wrong the moment there's more than one — it can
        // pick the already-completed INVITE-tx and build a stale 200 OK
        // that confuses the UAC (seen as a pre-existing bug under
        // session-timer refresh once the refresh path started awaiting a
        // real response).
        //
        // The right pick is the server-side transaction still in a state
        // that expects a response (Trying/Proceeding for NonInviteServer,
        // Proceeding for InviteServer). Filter on server-kind + open state;
        // prefer a non-INVITE tx when available so an in-dialog UPDATE/
        // re-INVITE response doesn't get misrouted to the original INVITE.
        let tx_mgr = self.manager.core().transaction_manager();
        let candidates: Vec<crate::transaction::TransactionKey> = self
            .manager
            .core()
            .transaction_to_dialog
            .iter()
            .filter(|entry| entry.value() == &dialog_id && entry.key().is_server())
            .map(|entry| entry.key().clone())
            .collect();

        let mut pending_non_invite: Option<crate::transaction::TransactionKey> = None;
        let mut pending_invite: Option<crate::transaction::TransactionKey> = None;
        let mut any_server: Option<crate::transaction::TransactionKey> = None;
        for key in candidates.into_iter() {
            let state = tx_mgr.transaction_state(&key).await.ok();
            let awaiting_response = matches!(
                state,
                Some(crate::transaction::TransactionState::Initial)
                    | Some(crate::transaction::TransactionState::Trying)
                    | Some(crate::transaction::TransactionState::Proceeding)
            );
            if awaiting_response {
                if *key.method() == rvoip_sip_core::Method::Invite {
                    pending_invite.get_or_insert(key.clone());
                } else {
                    pending_non_invite.get_or_insert(key.clone());
                }
            }
            any_server.get_or_insert(key);
        }

        let transaction_id = pending_non_invite
            .or(pending_invite)
            .or(any_server)
            .ok_or_else(|| {
                error!(
                    "No server transaction found for dialog {} (session {})",
                    dialog_id, session_id
                );
                for entry in self.manager.core().transaction_to_dialog.iter() {
                    debug!("Transaction {} -> Dialog {}", entry.key(), entry.value());
                }
                ApiError::Dialog {
                    message: format!("No transaction found for dialog {}", dialog_id),
                }
            })?;

        debug!(
            "Found transaction {} for dialog {}",
            transaction_id, dialog_id
        );

        // Build the response
        // For 200 OK responses to INVITE, we need special handling to ensure To tag is added
        let response = if status_code == 200 {
            // Get original request to check if it's an INVITE
            let original_request = self
                .manager
                .core()
                .transaction_manager()
                .original_request(&transaction_id)
                .await
                .map_err(|e| ApiError::Internal {
                    message: format!("Failed to get original request: {}", e),
                })?
                .ok_or_else(|| ApiError::Internal {
                    message: "No original request found for transaction".to_string(),
                })?;

            if original_request.method() == rvoip_sip_core::Method::Invite {
                // Use special response builder for 200 OK to INVITE that adds To tag
                use crate::transaction::utils::response_builders;
                let local_addr = self.manager.core().local_address;
                let mut response =
                    if let Some(contact_uri) = self.manager.core().local_contact_uri() {
                        response_builders::create_ok_response_with_contact_uri(
                            &original_request,
                            &contact_uri,
                        )
                        .map_err(|e| ApiError::Internal {
                            message: format!(
                                "Invalid configured local Contact URI {}: {}",
                                contact_uri, e
                            ),
                        })?
                    } else {
                        response_builders::create_ok_response_with_dialog_info(
                            &original_request,
                            "server",
                            &local_addr.ip().to_string(),
                            Some(local_addr.port()),
                        )
                    };

                // Add SDP if provided
                if let Some(sdp_body) = body {
                    response = response.with_body(sdp_body.as_bytes().to_vec());
                    // Add Content-Type header for SDP
                    use rvoip_sip_core::parser::headers::content_type::ContentTypeValue;
                    use rvoip_sip_core::{types::content_type::ContentType, TypedHeader};
                    response
                        .headers
                        .push(TypedHeader::ContentType(ContentType::new(
                            ContentTypeValue {
                                m_type: "application".to_string(),
                                m_subtype: "sdp".to_string(),
                                parameters: std::collections::HashMap::new(),
                            },
                        )));
                }

                response
            } else {
                // Not an INVITE, use regular response building
                self.build_response(
                    &transaction_id,
                    StatusCode::from_u16(status_code).unwrap_or(StatusCode::Ok),
                    body,
                )
                .await?
            }
        } else {
            // Not a 200 OK, use regular response building
            self.build_response(
                &transaction_id,
                StatusCode::from_u16(status_code).unwrap_or(StatusCode::Ok),
                body,
            )
            .await?
        };

        info!(
            "Sending {} response for session {} via transaction {}",
            status_code, session_id, transaction_id
        );

        // Call pre-send lifecycle hook for dialog state management
        // This handles UAS dialog confirmation when sending 200 OK to INVITE
        if let Ok(Some(original_request)) = self
            .manager
            .core()
            .transaction_manager()
            .original_request(&transaction_id)
            .await
        {
            use crate::manager::ResponseLifecycle;
            if let Err(e) = self
                .manager
                .core()
                .pre_send_response(&dialog_id, &response, &transaction_id, &original_request)
                .await
            {
                error!(
                    "Failed to execute pre_send_response hook for dialog {}: {}",
                    dialog_id, e
                );
                // Continue with sending - the error is logged but shouldn't block the response
            }
        }

        self.send_response(&transaction_id, response).await
    }

    // ========================================
    // SIP METHOD HELPERS (ALL MODES)
    // ========================================

    /// Send BYE request to terminate a dialog
    pub async fn send_bye(&self, dialog_id: &DialogId) -> ApiResult<TransactionKey> {
        self.manager.send_bye(dialog_id).await
    }

    /// Send REGISTER request for SIP registration
    ///
    /// Note: REGISTER is a non-dialog request, so it doesn't use a dialog_id.
    /// This method sends a REGISTER request directly via the transaction manager.
    ///
    /// # Arguments
    /// * `registrar_uri` - URI of the registrar (e.g., "sip:registrar.example.com")
    /// * `from_uri` - From URI (e.g., "sip:user@example.com")
    /// * `contact_uri` - Contact URI (e.g., "sip:user@192.168.1.100:5060")
    /// * `expires` - Registration expiry in seconds
    /// * `authorization` - Optional Authorization header value for digest auth
    /// Send REGISTER request for SIP registration
    ///
    /// Note: REGISTER is a non-dialog request, so it doesn't use a dialog_id.
    /// This method sends a REGISTER request directly via the transaction manager
    /// and waits for the response (including 401 auth challenges).
    ///
    /// # Arguments
    /// * `registrar_uri` - URI of the registrar (e.g., "sip:registrar.example.com")
    /// * `from_uri` - From URI (e.g., "sip:user@example.com")
    /// * `contact_uri` - Contact URI (e.g., "sip:user@192.168.1.100:5060")
    /// * `expires` - Registration expiry in seconds
    /// * `authorization` - Optional Authorization header value for digest auth
    ///
    /// # Returns
    /// The SIP response (200 OK, 401 Unauthorized, etc.)
    /// Snapshot of the most-recently-discovered public address, learned
    /// from RFC 3581 `received=`/`rport=` echoed back on inbound
    /// responses. Returns `None` until at least one qualifying
    /// response arrives. Useful for rewriting outbound `Contact:`
    /// headers in RE-registration / re-INVITE flows so a registrar's
    /// binding stays reachable through NAT (RFC 5626 §5).
    pub async fn discovered_public_addr(&self) -> Option<SocketAddr> {
        self.manager.core().discovered_public_addr().await
    }

    /// Snapshot of the registrar-provided Service-Route (RFC 3608) for
    /// the given AoR, learned from a previous REGISTER 2xx. Callers
    /// that originate out-of-dialog requests within the registration
    /// binding SHOULD pre-load these URIs as Route headers, in order.
    ///
    /// Returns `None` if no REGISTER 2xx has been observed for this AoR
    /// yet. Returns `Some(empty vec)` if a 2xx was observed but the
    /// registrar set no Service-Route — callers should not pre-load a
    /// Route in that case.
    pub async fn service_route_for_aor(
        &self,
        aor: &str,
    ) -> Option<Vec<rvoip_sip_core::types::uri::Uri>> {
        self.manager.core().service_route_for_aor(aor).await
    }

    /// Snapshot of the registrar-assigned GRUU URIs (RFC 5627 §5.3)
    /// for the given AoR, learned from the echoed Contact on a previous
    /// REGISTER 2xx. UAs that want to advertise a stable identity to
    /// peers should populate Contact with `pub_gruu` (or, for privacy,
    /// `temp_gruu`) on outbound out-of-dialog requests.
    ///
    /// Returns `None` if no REGISTER 2xx with GRUU has been observed
    /// for this AoR. The two fields of the returned struct are
    /// independent — a registrar may assign only `pub_gruu` or only
    /// `temp_gruu`.
    pub async fn gruu_for_aor(
        &self,
        aor: &str,
    ) -> Option<rvoip_sip_core::types::outbound::GruuContactParams> {
        self.manager.core().gruu_for_aor(aor).await
    }

    pub async fn send_register(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
        authorization: Option<String>,
    ) -> ApiResult<Response> {
        self.send_register_with_options(RegisterRequestOptions {
            registrar_uri: registrar_uri.to_string(),
            aor_uri: from_uri.to_string(),
            contact_uri: contact_uri.to_string(),
            expires,
            authorization,
            call_id: None,
            cseq: None,
            outbound_contact: None,
        })
        .await
    }

    /// Send REGISTER with RFC 5626 SIP Outbound Contact parameters.
    ///
    /// Attaches `+sip.instance="<urn>"` and `reg-id=N` Contact-header
    /// parameters and adds the `;ob` URI flag to the Contact URI. Use this
    /// variant when the registrar / carrier expects RFC 5626 Outbound
    /// semantics (most modern carrier infrastructure does).
    ///
    /// Semantically equivalent to [`Self::send_register`] for request
    /// routing; the only difference is the Contact header's shape.
    pub async fn send_register_with_outbound_contact(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        outbound_params: &rvoip_sip_core::types::outbound::OutboundContactParams,
        expires: u32,
        authorization: Option<String>,
    ) -> ApiResult<Response> {
        self.send_register_with_options(RegisterRequestOptions {
            registrar_uri: registrar_uri.to_string(),
            aor_uri: from_uri.to_string(),
            contact_uri: contact_uri.to_string(),
            expires,
            authorization,
            call_id: None,
            cseq: None,
            outbound_contact: Some(outbound_params.clone()),
        })
        .await
    }

    pub async fn send_register_with_options(
        &self,
        options: RegisterRequestOptions,
    ) -> ApiResult<Response> {
        use crate::transaction::client::builders::RegisterBuilder;
        use rvoip_sip_core::types::header::{HeaderName, HeaderValue};
        use rvoip_sip_core::types::TypedHeader;

        let cseq = options.cseq.unwrap_or(1);

        debug!(
            "Building REGISTER request to {} (expires={}, cseq={}, auth={}, outbound={})",
            options.registrar_uri,
            options.expires,
            cseq,
            options.authorization.is_some(),
            options.outbound_contact.is_some()
        );

        let mut builder = RegisterBuilder::new()
            .registrar(&options.registrar_uri)
            .aor(&options.aor_uri)
            .user_info(&options.aor_uri, "")
            .contact(&options.contact_uri)
            .local_address(self.manager.core().local_address())
            .expires(options.expires)
            .cseq(cseq);

        if let Some(call_id) = &options.call_id {
            builder = builder.call_id(call_id);
        }

        if let Some(params) = &options.outbound_contact {
            let contact = build_outbound_contact(&options.contact_uri, params).map_err(|e| {
                ApiError::protocol(format!(
                    "Invalid outbound Contact URI {}: {}",
                    options.contact_uri, e
                ))
            })?;
            builder = builder.contact_header(contact);
        }

        if let Some(auth) = options.authorization {
            builder = builder.header(TypedHeader::Other(
                HeaderName::Authorization,
                HeaderValue::Raw(auth.into_bytes()),
            ));
            debug!("Added Authorization header to REGISTER");
        }

        let request = builder
            .build()
            .map_err(|e| ApiError::protocol(format!("Failed to build REGISTER request: {}", e)))?;

        let dest_uri = options
            .registrar_uri
            .parse::<rvoip_sip_core::Uri>()
            .map_err(|e| ApiError::protocol(format!("Invalid registrar URI: {}", e)))?;

        let destination = crate::dialog::dialog_utils::resolve_uri_to_socketaddr(&dest_uri)
            .await
            .ok_or_else(|| {
                ApiError::protocol(format!(
                    "Failed to resolve registrar URI: {}",
                    options.registrar_uri
                ))
            })?;

        debug!("Sending REGISTER to {}", destination);

        let response = self
            .send_non_dialog_request(request, destination, std::time::Duration::from_secs(32))
            .await?;

        debug!("Received REGISTER response: {}", response.status_code());
        Ok(response)
    }

    /// Send an out-of-dialog SUBSCRIBE request.
    pub async fn send_subscribe_out_of_dialog(
        &self,
        target_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        event_package: &str,
        expires: u32,
    ) -> ApiResult<Response> {
        let local_addr = self.manager.core().local_address();
        let request = crate::transaction::dialog::subscribe_out_of_dialog(
            target_uri,
            from_uri,
            contact_uri,
            event_package,
            expires,
            1,
            local_addr,
        )
        .map_err(|e| ApiError::protocol(format!("Failed to build SUBSCRIBE request: {}", e)))?;

        let dest_uri = target_uri
            .parse::<rvoip_sip_core::Uri>()
            .map_err(|e| ApiError::protocol(format!("Invalid SUBSCRIBE target URI: {}", e)))?;
        let destination = crate::dialog::dialog_utils::resolve_uri_to_socketaddr(&dest_uri)
            .await
            .ok_or_else(|| {
                ApiError::protocol(format!(
                    "Failed to resolve SUBSCRIBE target URI: {}",
                    target_uri
                ))
            })?;

        self.send_non_dialog_request(request, destination, std::time::Duration::from_secs(30))
            .await
    }

    /// Send an out-of-dialog MESSAGE request.
    pub async fn send_message_out_of_dialog(
        &self,
        target_uri: &str,
        from_uri: &str,
        body: String,
    ) -> ApiResult<Response> {
        let local_addr = self.manager.core().local_address();
        let request = crate::transaction::dialog::message_out_of_dialog(
            target_uri, from_uri, body, 1, local_addr,
        )
        .map_err(|e| ApiError::protocol(format!("Failed to build MESSAGE request: {}", e)))?;

        let dest_uri = target_uri
            .parse::<rvoip_sip_core::Uri>()
            .map_err(|e| ApiError::protocol(format!("Invalid MESSAGE target URI: {}", e)))?;
        let destination = crate::dialog::dialog_utils::resolve_uri_to_socketaddr(&dest_uri)
            .await
            .ok_or_else(|| {
                ApiError::protocol(format!(
                    "Failed to resolve MESSAGE target URI: {}",
                    target_uri
                ))
            })?;

        self.send_non_dialog_request(request, destination, std::time::Duration::from_secs(10))
            .await
    }

    /// Send REFER request for call transfer
    pub async fn send_refer(
        &self,
        dialog_id: &DialogId,
        target_uri: String,
        refer_body: Option<String>,
    ) -> ApiResult<TransactionKey> {
        self.manager
            .send_refer(dialog_id, target_uri, refer_body)
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
        self.manager
            .send_notify(dialog_id, event, body, subscription_state)
            .await
    }

    /// Send NOTIFY for REFER implicit subscription (RFC 3515)
    pub async fn send_refer_notify(
        &self,
        dialog_id: &DialogId,
        status_code: u16,
        reason: &str,
    ) -> ApiResult<TransactionKey> {
        self.manager
            .send_refer_notify(dialog_id, status_code, reason)
            .await
    }

    /// Send UPDATE request for media modifications
    pub async fn send_update(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>,
    ) -> ApiResult<TransactionKey> {
        self.manager.send_update(dialog_id, sdp).await
    }

    /// Send a re-INVITE on an established dialog. Used by RFC 4028 session
    /// timer refreshers as a fallback when UPDATE is not supported (501
    /// Not Implemented), and by hold/resume flows.
    pub async fn send_reinvite(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>,
    ) -> ApiResult<TransactionKey> {
        self.manager
            .inner_manager()
            .send_request(
                dialog_id,
                rvoip_sip_core::Method::Invite,
                sdp.map(bytes::Bytes::from),
            )
            .await
            .map_err(ApiError::from)
    }

    /// Send PRACK for a reliable provisional response (RFC 3262).
    pub async fn send_prack(&self, dialog_id: &DialogId, rseq: u32) -> ApiResult<TransactionKey> {
        self.manager.send_prack(dialog_id, rseq).await
    }

    /// Send INFO request for application-specific information
    pub async fn send_info(
        &self,
        dialog_id: &DialogId,
        info_body: String,
    ) -> ApiResult<TransactionKey> {
        self.manager.send_info(dialog_id, info_body).await
    }

    /// Send CANCEL request to cancel a pending INVITE
    ///
    /// This method cancels a pending INVITE transaction that hasn't received a final response.
    /// Only works for dialogs in the Early state (before 200 OK is received).
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
    /// - Dialog is not in Early state
    /// - No pending INVITE transaction found
    pub async fn send_cancel(&self, dialog_id: &DialogId) -> ApiResult<TransactionKey> {
        self.manager.send_cancel(dialog_id).await
    }

    // ========================================
    // DIALOG MANAGEMENT (ALL MODES)
    // ========================================

    /// Get information about a dialog
    pub async fn get_dialog_info(&self, dialog_id: &DialogId) -> ApiResult<Dialog> {
        self.manager.get_dialog_info(dialog_id).await
    }

    /// Get the current state of a dialog
    pub async fn get_dialog_state(&self, dialog_id: &DialogId) -> ApiResult<DialogState> {
        self.manager.get_dialog_state(dialog_id).await
    }

    /// Terminate a dialog
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> ApiResult<()> {
        self.manager.terminate_dialog(dialog_id).await
    }

    /// List all active dialogs
    pub async fn list_active_dialogs(&self) -> Vec<DialogId> {
        self.manager.list_active_dialogs().await
    }

    /// Get a dialog handle for convenient operations
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog ID to create a handle for
    ///
    /// # Returns
    /// DialogHandle for the specified dialog
    pub async fn get_dialog_handle(&self, dialog_id: &DialogId) -> ApiResult<DialogHandle> {
        // Verify dialog exists first
        self.get_dialog_info(dialog_id).await?;

        // Create handle using the core dialog manager
        Ok(DialogHandle::new(
            dialog_id.clone(),
            Arc::new(self.manager.core().clone()),
        ))
    }

    /// Get a call handle for convenient call operations
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog ID representing the call
    ///
    /// # Returns
    /// CallHandle for the specified call
    pub async fn get_call_handle(&self, dialog_id: &DialogId) -> ApiResult<CallHandle> {
        // Verify dialog exists first
        self.get_dialog_info(dialog_id).await?;

        // Create call handle using the core dialog manager
        Ok(CallHandle::new(
            dialog_id.clone(),
            Arc::new(self.manager.core().clone()),
        ))
    }

    // ========================================
    // MONITORING & STATISTICS
    // ========================================

    /// Get comprehensive statistics for this API instance
    ///
    /// Returns detailed statistics including dialog counts, call metrics,
    /// and mode-specific information.
    pub async fn get_stats(&self) -> DialogStats {
        let manager_stats = self.manager.get_stats().await;

        DialogStats {
            active_dialogs: manager_stats.active_dialogs,
            total_dialogs: manager_stats.total_dialogs,
            successful_calls: manager_stats.successful_calls,
            failed_calls: manager_stats.failed_calls,
            avg_call_duration: if manager_stats.successful_calls > 0 {
                manager_stats.total_call_duration / manager_stats.successful_calls as f64
            } else {
                0.0
            },
        }
    }

    /// Get active dialogs with handles for easy management
    ///
    /// Returns a list of DialogHandle instances for all active dialogs.
    pub async fn active_dialogs(&self) -> Vec<DialogHandle> {
        let dialog_ids = self.list_active_dialogs().await;
        let mut handles = Vec::new();

        for dialog_id in dialog_ids {
            if let Ok(handle) = self.get_dialog_handle(&dialog_id).await {
                handles.push(handle);
            }
        }

        handles
    }

    /// Send ACK for 2xx response to INVITE
    ///
    /// Handles the automatic ACK sending required by RFC 3261 for 200 OK responses to INVITE.
    /// This method ensures proper completion of the 3-way handshake (INVITE → 200 OK → ACK).
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
        self.manager
            .send_ack_for_2xx_response(dialog_id, original_invite_tx_id, response)
            .await
    }

    // ========================================
    // CONVENIENCE METHODS
    // ========================================

    /// Check if this API supports outgoing calls
    pub fn supports_outgoing_calls(&self) -> bool {
        self.config.supports_outgoing_calls()
    }

    /// Check if this API supports incoming calls
    pub fn supports_incoming_calls(&self) -> bool {
        self.config.supports_incoming_calls()
    }

    /// Get the from URI for outgoing requests (if configured)
    pub fn from_uri(&self) -> Option<&str> {
        self.config.from_uri()
    }

    /// Get the domain for server operations (if configured)
    pub fn domain(&self) -> Option<&str> {
        self.config.domain()
    }

    /// Check if automatic authentication is enabled
    pub fn auto_auth_enabled(&self) -> bool {
        self.config.auto_auth_enabled()
    }

    /// Check if automatic OPTIONS response is enabled
    pub fn auto_options_enabled(&self) -> bool {
        self.config.auto_options_enabled()
    }

    /// Check if automatic REGISTER response is enabled
    pub fn auto_register_enabled(&self) -> bool {
        self.config.auto_register_enabled()
    }

    /// Create a new unified dialog API with automatic transport setup (SIMPLE)
    ///
    /// This is the recommended constructor for most use cases. It automatically
    /// creates and configures the transport and transaction managers internally,
    /// providing a clean high-level API.
    ///
    /// # Arguments
    /// * `config` - Configuration determining the behavior mode and bind address
    ///
    /// # Returns
    /// New UnifiedDialogApi instance with automatic transport setup
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use rvoip_dialog_core::api::unified::UnifiedDialogApi;
    /// use rvoip_dialog_core::config::DialogManagerConfig;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
    ///     .with_from_uri("sip:alice@example.com")
    ///     .build();
    ///
    /// let api = UnifiedDialogApi::create(config).await?;
    /// api.start().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create(config: DialogManagerConfig) -> ApiResult<Self> {
        use crate::transaction::{
            transport::{TransportManager, TransportManagerConfig},
            TransactionManager,
        };

        info!(
            "Creating UnifiedDialogApi with automatic transport setup in {:?} mode",
            Self::mode_name(&config)
        );

        // Create transport manager automatically with sensible defaults
        let bind_addr = config.local_address();
        let transport_config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec![bind_addr],
            ..Default::default()
        };

        let (mut transport, transport_rx) =
            TransportManager::new(transport_config)
                .await
                .map_err(|e| ApiError::Internal {
                    message: format!("Failed to create transport manager: {}", e),
                })?;

        transport
            .initialize()
            .await
            .map_err(|e| ApiError::Internal {
                message: format!("Failed to initialize transport: {}", e),
            })?;

        // Create transaction manager with global events automatically
        // Use larger channel capacity for high-concurrency scenarios (e.g., 500+ concurrent calls)
        let (transaction_manager, global_rx) = TransactionManager::with_transport_manager(
            transport,
            transport_rx,
            Some(10000), // Increased from 100 to handle high concurrent call volumes
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: format!("Failed to create transaction manager: {}", e),
        })?;

        // Create the unified dialog API with all components
        Self::with_global_events(Arc::new(transaction_manager), global_rx, config).await
    }

    // ========================================
    // NON-DIALOG OPERATIONS
    // ========================================

    /// Send a non-dialog SIP request (for REGISTER, OPTIONS, etc.)
    ///
    /// This method allows sending SIP requests that don't establish or require
    /// a dialog context. Useful for:
    /// - REGISTER requests for endpoint registration
    /// - OPTIONS requests for capability discovery
    /// - MESSAGE requests for instant messaging
    /// - SUBSCRIBE requests for event subscriptions
    ///
    /// # Arguments
    /// * `request` - Complete SIP request to send
    /// * `destination` - Target address to send the request to
    /// * `timeout` - Maximum time to wait for a response
    ///
    /// # Returns
    /// The SIP response received
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::expires::ExpiresExt;
    /// use std::time::Duration;
    ///
    /// # async fn example(api: rvoip_dialog_core::api::unified::UnifiedDialogApi) -> Result<(), Box<dyn std::error::Error>> {
    /// // Build a REGISTER request
    /// let request = SimpleRequestBuilder::register("sip:registrar.example.com")?
    ///     .from("", "sip:alice@example.com", Some("tag123"))
    ///     .to("", "sip:alice@example.com", None)
    ///     .call_id("reg-12345")
    ///     .cseq(1)
    ///     .via("192.168.1.100:5060", "UDP", Some("branch123"))
    ///     .contact("sip:alice@192.168.1.100:5060", None)
    ///     .expires(3600)
    ///     .build();
    ///
    /// let destination = "192.168.1.1:5060".parse()?;
    /// let response = api.send_non_dialog_request(
    ///     request,
    ///     destination,
    ///     Duration::from_secs(32)
    /// ).await?;
    ///
    /// println!("Registration response: {}", response.status_code());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_non_dialog_request(
        &self,
        request: Request,
        destination: SocketAddr,
        timeout: std::time::Duration,
    ) -> ApiResult<Response> {
        debug!(
            "Sending non-dialog {} request to {}",
            request.method(),
            destination
        );

        // Create a non-dialog transaction directly with the transaction manager
        let transaction_id = match request.method() {
            Method::Invite => {
                return Err(ApiError::protocol(
                    "INVITE requests must use dialog context. Use make_call() instead.",
                ));
            }
            _ => {
                // Create non-INVITE client transaction
                self.manager
                    .core()
                    .transaction_manager()
                    .create_non_invite_client_transaction(request, destination)
                    .await
                    .map_err(|e| {
                        ApiError::internal(format!("Failed to create transaction: {}", e))
                    })?
            }
        };

        // Send the request
        self.manager
            .core()
            .transaction_manager()
            .send_request(&transaction_id)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to send request: {}", e)))?;

        // Wait for final response
        let response = self
            .manager
            .core()
            .transaction_manager()
            .wait_for_final_response(&transaction_id, timeout)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to wait for response: {}", e)))?
            .ok_or_else(|| ApiError::network(format!("Request timed out after {:?}", timeout)))?;

        debug!(
            "Received response {} for non-dialog request",
            response.status_code()
        );
        Ok(response)
    }
}

#[cfg(test)]
mod outbound_contact_tests {
    use super::build_outbound_contact;
    use rvoip_sip_core::types::outbound::OutboundContactParams;

    #[test]
    fn builds_contact_with_instance_regid_and_ob_flag() {
        let params = OutboundContactParams {
            instance_urn: "urn:uuid:00000000-0000-1000-8000-AABBCCDDEEFF".into(),
            reg_id: 1,
        };
        let contact = build_outbound_contact("sip:alice@192.168.1.10:5060", &params).unwrap();
        let s = contact.to_string();
        assert!(s.contains(";ob"), "Contact missing ;ob URI flag: {}", s);
        assert!(
            s.contains("+sip.instance=\"<urn:uuid:00000000-0000-1000-8000-AABBCCDDEEFF>\""),
            "Contact missing +sip.instance: {}",
            s
        );
        assert!(s.contains("reg-id=1"), "Contact missing reg-id: {}", s);
    }

    #[test]
    fn ob_flag_goes_on_uri_params_section() {
        // RFC 5626 §5.4: `;ob` is a URI parameter, inside the `<>`.
        // Contact-header params (`+sip.instance`, `reg-id`) go after the
        // URI's `>`. Validate the ordering by finding `;ob` before `>`.
        let params = OutboundContactParams {
            instance_urn: "urn:uuid:x".into(),
            reg_id: 1,
        };
        let s = build_outbound_contact("sip:alice@host:5060", &params)
            .unwrap()
            .to_string();
        let ob_pos = s.find(";ob").expect("missing ;ob");
        let angle_pos = s.find('>').expect("Contact missing closing angle bracket");
        assert!(
            ob_pos < angle_pos,
            "`;ob` must sit inside the URI angle brackets, got: {}",
            s
        );
    }

    #[test]
    fn invalid_uri_returns_error() {
        let params = OutboundContactParams {
            instance_urn: "urn:uuid:x".into(),
            reg_id: 1,
        };
        assert!(build_outbound_contact("not a uri", &params).is_err());
    }

    #[test]
    fn reg_id_value_propagates() {
        let params = OutboundContactParams {
            instance_urn: "urn:uuid:x".into(),
            reg_id: 7,
        };
        let s = build_outbound_contact("sip:alice@host", &params)
            .unwrap()
            .to_string();
        assert!(s.contains("reg-id=7"), "reg-id value not propagated: {}", s);
    }
}
