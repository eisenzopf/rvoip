//! Event handling for the client-core library
//! 
//! This module contains the event handler that bridges session-core events
//! to client-core events, providing a clean abstraction for applications.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use dashmap::DashMap;
use chrono::Utc;

// Import session-core types
use rvoip_session_core::{
    api::{
        types::{SessionId, CallSession, CallState, IncomingCall, CallDecision},
        handlers::CallHandler,
    },
};

// Import client-core types
use crate::{
    call::{CallId, CallInfo, CallDirection},
    events::{ClientEventHandler, IncomingCallInfo, CallStatusInfo},
};

// All types are re-exported from the main events module

/// Internal call handler that bridges session-core events to client-core events
/// 
/// This handler receives events from the session-core layer and translates them
/// into client-core events that applications can consume. It manages mappings
/// between session IDs and call IDs, tracks call state, and forwards events
/// to registered event handlers.
/// 
/// # Architecture
/// 
/// The handler maintains several mappings:
/// - Session ID ↔ Call ID mapping for event translation
/// - Call information storage with extended metadata
/// - Incoming call storage for deferred acceptance/rejection
/// - Event broadcasting through multiple channels
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::client::events::ClientCallHandler;
/// use rvoip_session_core::CallHandler;
/// use std::sync::Arc;
/// use dashmap::DashMap;
/// 
/// let handler = ClientCallHandler::new(
///     Arc::new(DashMap::new()), // call_mapping
///     Arc::new(DashMap::new()), // session_mapping  
///     Arc::new(DashMap::new()), // call_info
///     Arc::new(DashMap::new()), // incoming_calls
/// );
/// ```
pub struct ClientCallHandler {
    /// Client event handler for forwarding processed events to applications
    /// 
    /// This optional handler receives high-level client events after they have been
    /// processed and enriched by this bridge. Applications can register handlers
    /// to receive notifications about incoming calls, state changes, etc.
    pub client_event_handler: Arc<RwLock<Option<Arc<dyn ClientEventHandler>>>>,
    
    /// Mapping from session-core SessionId to client-core CallId
    /// 
    /// This bidirectional mapping allows the handler to translate between
    /// session-core's internal session identifiers and the client-facing call IDs
    /// that applications use to reference calls.
    pub call_mapping: Arc<DashMap<SessionId, CallId>>,
    
    /// Reverse mapping from client-core CallId to session-core SessionId
    /// 
    /// Provides efficient lookup in the opposite direction from call_mapping,
    /// allowing quick translation from client call IDs to session IDs when
    /// making session-core API calls.
    pub session_mapping: Arc<DashMap<CallId, SessionId>>,
    
    /// Enhanced call information storage with extended metadata
    /// 
    /// Stores comprehensive call information including state, timing data,
    /// participant details, and custom metadata. This information persists
    /// throughout the call lifecycle and can be used for history and reporting.
    pub call_info: Arc<DashMap<CallId, CallInfo>>,
    
    /// Storage for incoming calls awaiting acceptance or rejection
    /// 
    /// When incoming calls arrive, they are stored here until the application
    /// decides whether to accept or reject them. This allows for deferred
    /// call handling and provides access to full call details for decision making.
    pub incoming_calls: Arc<DashMap<CallId, IncomingCall>>,
    
    /// Optional broadcast channel for real-time event streaming
    /// 
    /// If configured, events are broadcast through this channel in addition to
    /// being sent to the registered event handler. This allows multiple consumers
    /// to receive events independently.
    pub event_tx: Option<tokio::sync::broadcast::Sender<crate::events::ClientEvent>>,
    
    /// Channel for notifying when calls become established
    /// 
    /// This channel is used to notify ClientManager when a call transitions to 
    /// the Connected state, allowing it to set up audio frame subscription.
    pub(crate) call_established_tx: Option<tokio::sync::mpsc::UnboundedSender<CallId>>,
    
    /// Channel for sending events back to session-core
    /// 
    /// This channel is used to send cleanup confirmations and other events
    /// back to the session coordinator. We use RwLock to allow setting it after creation.
    pub session_event_tx: Arc<RwLock<Option<tokio::sync::mpsc::Sender<rvoip_session_core::manager::events::SessionEvent>>>>,
}

impl std::fmt::Debug for ClientCallHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientCallHandler")
            .field("client_event_handler", &"<event handler>")
            .field("call_mapping", &self.call_mapping)
            .field("session_mapping", &self.session_mapping)
            .field("call_info", &self.call_info)
            .field("incoming_calls", &self.incoming_calls)
            .finish()
    }
}

impl ClientCallHandler {
    /// Create a new ClientCallHandler with required mappings and storage
    /// 
    /// This constructor initializes the handler with the necessary data structures
    /// for managing call state and event translation between session-core and client-core.
    /// 
    /// # Arguments
    /// 
    /// * `call_mapping` - Bidirectional mapping between session IDs and call IDs
    /// * `session_mapping` - Reverse mapping for efficient lookups  
    /// * `call_info` - Storage for comprehensive call information and metadata
    /// * `incoming_calls` - Storage for pending incoming calls
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::client::events::ClientCallHandler;
    /// use std::sync::Arc;
    /// use dashmap::DashMap;
    /// 
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// ```
    pub fn new(
        call_mapping: Arc<DashMap<SessionId, CallId>>,
        session_mapping: Arc<DashMap<CallId, SessionId>>,
        call_info: Arc<DashMap<CallId, CallInfo>>,
        incoming_calls: Arc<DashMap<CallId, IncomingCall>>,
    ) -> Self {
        Self {
            client_event_handler: Arc::new(RwLock::new(None)),
            call_mapping,
            session_mapping,
            call_info,
            incoming_calls,
            event_tx: None,
            call_established_tx: None,
            session_event_tx: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Configure the handler with an event broadcast channel
    /// 
    /// This method adds broadcast capability to the handler, allowing events
    /// to be sent to multiple consumers through a tokio broadcast channel.
    /// Events will be sent to both the registered event handler and the broadcast channel.
    /// 
    /// # Arguments
    /// 
    /// * `event_tx` - Broadcast sender for streaming events to multiple consumers
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::client::events::ClientCallHandler;
    /// use std::sync::Arc;
    /// use dashmap::DashMap;
    /// 
    /// let (tx, _rx) = tokio::sync::broadcast::channel(100);
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// ).with_event_tx(tx);
    /// ```
    pub fn with_event_tx(mut self, event_tx: tokio::sync::broadcast::Sender<crate::events::ClientEvent>) -> Self {
        self.event_tx = Some(event_tx);
        self
    }
    
    /// Configure the handler with a call established notification channel
    /// 
    /// This internal method is used by ClientManager to provide a channel
    /// for receiving notifications when calls transition to the Connected state.
    pub(crate) fn with_call_established_tx(mut self, tx: tokio::sync::mpsc::UnboundedSender<CallId>) -> Self {
        self.call_established_tx = Some(tx);
        self
    }
    
    /// Set the session event channel
    /// 
    /// This internal method is used to provide a channel for sending events
    /// back to the session coordinator, particularly for cleanup confirmations.
    pub(crate) async fn set_session_event_tx(&self, tx: tokio::sync::mpsc::Sender<rvoip_session_core::manager::events::SessionEvent>) {
        *self.session_event_tx.write().await = Some(tx);
    }
    
    /// Register an event handler to receive processed client events
    /// 
    /// This method sets the application-level event handler that will receive
    /// high-level client events after they have been processed and enriched by this bridge.
    /// The handler will be called for incoming calls, state changes, media events, etc.
    /// 
    /// # Arguments
    /// 
    /// * `handler` - The event handler implementation to register
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use rvoip_client_core::events::ClientEventHandler;
    /// # use std::sync::Arc;
    /// # use dashmap::DashMap;
    /// # struct MyEventHandler;
    /// # #[async_trait::async_trait]
    /// # impl ClientEventHandler for MyEventHandler {
    /// #     async fn on_incoming_call(&self, _info: rvoip_client_core::events::IncomingCallInfo) -> rvoip_client_core::events::CallAction {
    /// #         rvoip_client_core::events::CallAction::Accept
    /// #     }
    /// #     async fn on_call_state_changed(&self, _info: rvoip_client_core::events::CallStatusInfo) {}
    /// #     async fn on_media_event(&self, _info: rvoip_client_core::events::MediaEventInfo) {}
    /// #     async fn on_registration_status_changed(&self, _info: rvoip_client_core::events::RegistrationStatusInfo) {}
    /// # }
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// let event_handler = Arc::new(MyEventHandler);
    /// handler.set_event_handler(event_handler).await;
    /// # }
    /// ```
    pub async fn set_event_handler(&self, handler: Arc<dyn ClientEventHandler>) {
        *self.client_event_handler.write().await = Some(handler);
    }
    
    /// Store an IncomingCall object for later use
    /// 
    /// This method stores an incoming call in the handler's storage, allowing it to be
    /// retrieved later when the application decides to accept or reject the call.
    /// This enables deferred call handling where the application can examine call
    /// details before making a decision.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The client-core call ID for this incoming call
    /// * `incoming_call` - The session-core IncomingCall object with full details
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use rvoip_client_core::call::CallId;
    /// # use rvoip_session_core::api::types::IncomingCall;
    /// # use std::sync::Arc;
    /// # use dashmap::DashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// let call_id = CallId::new_v4();
    /// # let incoming_call = IncomingCall {
    /// #     id: rvoip_session_core::api::types::SessionId("test".to_string()),
    /// #     from: "sip:caller@example.com".to_string(),
    /// #     to: "sip:callee@example.com".to_string(),
    /// #     sdp: None,
    /// #     headers: std::collections::HashMap::new(),
    /// #     received_at: std::time::Instant::now(),
    /// # };
    /// handler.store_incoming_call(call_id, incoming_call).await;
    /// # }
    /// ```
    pub async fn store_incoming_call(&self, call_id: CallId, incoming_call: IncomingCall) {
        self.incoming_calls.insert(call_id, incoming_call);
    }
    
    /// Retrieve a stored IncomingCall object
    /// 
    /// This method retrieves a previously stored incoming call by its call ID.
    /// This is useful when the application needs to access the full incoming call
    /// details (SDP, headers, etc.) when making accept/reject decisions or during
    /// call processing.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The client-core call ID to retrieve the incoming call for
    /// 
    /// # Returns
    /// 
    /// `Some(IncomingCall)` if a stored incoming call exists for the given call ID,
    /// `None` if no incoming call is found or if the call has already been processed.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use rvoip_client_core::call::CallId;
    /// # use std::sync::Arc;
    /// # use dashmap::DashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// let call_id = CallId::new_v4();
    /// 
    /// // Retrieve incoming call details
    /// if let Some(incoming_call) = handler.get_incoming_call(&call_id).await {
    ///     println!("Found incoming call from: {}", incoming_call.from);
    ///     println!("SDP offer present: {}", incoming_call.sdp.is_some());
    /// } else {
    ///     println!("No incoming call found for ID: {}", call_id);
    /// }
    /// # }
    /// ```
    pub async fn get_incoming_call(&self, call_id: &CallId) -> Option<IncomingCall> {
        self.incoming_calls.get(call_id).map(|entry| entry.value().clone())
    }
    
    /// Extract display name from SIP URI or headers
    /// 
    /// This method attempts to extract a human-readable display name from a SIP URI
    /// or associated headers. It implements multiple extraction strategies to handle
    /// various SIP message formats and header configurations.
    /// 
    /// # Arguments
    /// 
    /// * `uri` - The SIP URI to extract display name from (e.g., "Alice Smith" <sip:alice@example.com>)
    /// * `headers` - SIP message headers that may contain display name information
    /// 
    /// # Returns
    /// 
    /// `Some(String)` containing the extracted display name if found, `None` if no
    /// display name could be extracted from the URI or headers.
    /// 
    /// # Extraction Strategy
    /// 
    /// 1. Check for quoted display name in URI: `"Display Name" <sip:user@domain>`
    /// 2. Check for unquoted display name before angle brackets: `Display Name <sip:user@domain>`
    /// 3. Check From header for display name using the same strategies
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use std::sync::Arc;
    /// # use std::collections::HashMap;
    /// # use dashmap::DashMap;
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// let mut headers = HashMap::new();
    /// headers.insert("From".to_string(), "\"Alice Smith\" <sip:alice@example.com>".to_string());
    /// 
    /// // Extract from quoted URI
    /// let display_name = handler.extract_display_name(
    ///     "\"Alice Smith\" <sip:alice@example.com>", 
    ///     &headers
    /// );
    /// assert_eq!(display_name, Some("Alice Smith".to_string()));
    /// 
    /// // Extract from unquoted URI
    /// let display_name = handler.extract_display_name(
    ///     "Bob Jones <sip:bob@example.com>", 
    ///     &HashMap::new()
    /// );
    /// assert_eq!(display_name, Some("Bob Jones".to_string()));
    /// 
    /// // No display name available
    /// let display_name = handler.extract_display_name(
    ///     "sip:carol@example.com", 
    ///     &HashMap::new()
    /// );
    /// assert_eq!(display_name, None);
    /// ```
    pub fn extract_display_name(&self, uri: &str, headers: &HashMap<String, String>) -> Option<String> {
        // First try to extract from URI (e.g., "Display Name" <sip:user@domain>)
        if let Some(start) = uri.find('"') {
            if let Some(end) = uri[start + 1..].find('"') {
                let display_name = &uri[start + 1..start + 1 + end];
                if !display_name.is_empty() {
                    return Some(display_name.to_string());
                }
            }
        }
        
        // Try display name before < in URI
        if let Some(angle_pos) = uri.find('<') {
            let potential_name = uri[..angle_pos].trim();
            if !potential_name.is_empty() && !potential_name.starts_with("sip:") {
                return Some(potential_name.to_string());
            }
        }
        
        // Try From header display name
        if let Some(from_header) = headers.get("From") {
            return self.extract_display_name_from_header(from_header);
        }
        
        None
    }
    
    /// Extract display name from a SIP header string
    /// 
    /// This method extracts a human-readable display name from a SIP header value,
    /// typically the From or To header. It handles both quoted and unquoted display
    /// name formats commonly found in SIP messages.
    /// 
    /// # Arguments
    /// 
    /// * `header` - The SIP header value to parse for display name information
    /// 
    /// # Returns
    /// 
    /// `Some(String)` containing the extracted display name if found, `None` if no
    /// display name could be extracted from the header.
    /// 
    /// # Supported Formats
    /// 
    /// - Quoted display name: `"Alice Smith" <sip:alice@example.com>`
    /// - Unquoted display name: `Alice Smith <sip:alice@example.com>`
    /// - Plain SIP URI: `sip:alice@example.com` (returns None)
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use std::sync::Arc;
    /// # use dashmap::DashMap;
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// // Extract from quoted header
    /// let name = handler.extract_display_name_from_header(
    ///     "\"Alice Smith\" <sip:alice@example.com>"
    /// );
    /// assert_eq!(name, Some("Alice Smith".to_string()));
    /// 
    /// // Extract from unquoted header
    /// let name = handler.extract_display_name_from_header(
    ///     "Bob Jones <sip:bob@example.com>"
    /// );
    /// assert_eq!(name, Some("Bob Jones".to_string()));
    /// 
    /// // No display name in plain URI
    /// let name = handler.extract_display_name_from_header(
    ///     "sip:carol@example.com"
    /// );
    /// assert_eq!(name, None);
    /// ```
    pub fn extract_display_name_from_header(&self, header: &str) -> Option<String> {
        if let Some(start) = header.find('"') {
            if let Some(end) = header[start + 1..].find('"') {
                let display_name = &header[start + 1..start + 1 + end];
                if !display_name.is_empty() {
                    return Some(display_name.to_string());
                }
            }
        }
        
        if let Some(angle_pos) = header.find('<') {
            let potential_name = header[..angle_pos].trim();
            if !potential_name.is_empty() && !potential_name.starts_with("sip:") {
                return Some(potential_name.to_string());
            }
        }
        
        None
    }
    
    /// Extract call subject from SIP message headers
    /// 
    /// This method extracts the subject/purpose of a call from SIP message headers.
    /// The subject provides contextual information about the call and is typically
    /// used for displaying call purpose in user interfaces or for call routing decisions.
    /// 
    /// # Arguments
    /// 
    /// * `headers` - HashMap containing SIP message headers to search for subject information
    /// 
    /// # Returns
    /// 
    /// `Some(String)` containing the subject text if found and non-empty,
    /// `None` if no subject header exists or if the subject is empty.
    /// 
    /// # Header Priority
    /// 
    /// The method searches for subject information in the following order:
    /// 1. "Subject" header (standard case)
    /// 2. "subject" header (lowercase variant)
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use std::sync::Arc;
    /// # use std::collections::HashMap;
    /// # use dashmap::DashMap;
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// let mut headers = HashMap::new();
    /// headers.insert("Subject".to_string(), "Conference Call".to_string());
    /// 
    /// // Extract subject from headers
    /// let subject = handler.extract_subject(&headers);
    /// assert_eq!(subject, Some("Conference Call".to_string()));
    /// 
    /// // Empty subject returns None
    /// let mut empty_headers = HashMap::new();
    /// empty_headers.insert("Subject".to_string(), "".to_string());
    /// let subject = handler.extract_subject(&empty_headers);
    /// assert_eq!(subject, None);
    /// 
    /// // No subject header returns None
    /// let subject = handler.extract_subject(&HashMap::new());
    /// assert_eq!(subject, None);
    /// ```
    pub fn extract_subject(&self, headers: &HashMap<String, String>) -> Option<String> {
        headers.get("Subject")
            .or_else(|| headers.get("subject"))
            .cloned()
            .filter(|s| !s.is_empty())
    }
    
    /// Extract SIP Call-ID from message headers
    /// 
    /// This method extracts the unique Call-ID identifier from SIP message headers.
    /// The Call-ID is a mandatory header in SIP messages that uniquely identifies
    /// a call dialog and remains constant throughout the entire call session.
    /// 
    /// # Arguments
    /// 
    /// * `headers` - HashMap containing SIP message headers to search for Call-ID
    /// 
    /// # Returns
    /// 
    /// `Some(String)` containing the Call-ID value if found,
    /// `None` if no Call-ID header exists in the message.
    /// 
    /// # Header Priority
    /// 
    /// The method searches for Call-ID information in the following order:
    /// 1. "Call-ID" header (standard case)
    /// 2. "call-id" header (lowercase variant)
    /// 
    /// # SIP Specification
    /// 
    /// According to RFC 3261, the Call-ID header is mandatory in all SIP requests
    /// and responses. It consists of a locally unique identifier followed by an
    /// "@" sign and a globally unique identifier (usually the host domain).
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use std::sync::Arc;
    /// # use std::collections::HashMap;
    /// # use dashmap::DashMap;
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// let mut headers = HashMap::new();
    /// headers.insert("Call-ID".to_string(), "1234567890@example.com".to_string());
    /// 
    /// // Extract Call-ID from headers
    /// let call_id = handler.extract_call_id(&headers);
    /// assert_eq!(call_id, Some("1234567890@example.com".to_string()));
    /// 
    /// // Case-insensitive header lookup
    /// let mut headers_lower = HashMap::new();
    /// headers_lower.insert("call-id".to_string(), "abcdef@sip.example.org".to_string());
    /// let call_id = handler.extract_call_id(&headers_lower);
    /// assert_eq!(call_id, Some("abcdef@sip.example.org".to_string()));
    /// 
    /// // No Call-ID header returns None
    /// let call_id = handler.extract_call_id(&HashMap::new());
    /// assert_eq!(call_id, None);
    /// ```
    pub fn extract_call_id(&self, headers: &HashMap<String, String>) -> Option<String> {
        headers.get("Call-ID")
            .or_else(|| headers.get("call-id"))
            .cloned()
    }
    
    /// Update stored call information with enhanced session data
    /// 
    /// This method synchronizes call information stored in the client-core layer
    /// with the current state from the session-core layer. It handles state transitions,
    /// timestamp updates, and event emission when significant changes occur.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The client-core call ID to update information for
    /// * `session` - The session-core CallSession object containing current state and metadata
    /// 
    /// # Behavior
    /// 
    /// The method performs the following operations:
    /// 1. Maps session-core state to client-core state representation
    /// 2. Updates timestamps for significant state transitions (connected, ended)
    /// 3. Emits state change events to registered handlers when state changes
    /// 4. Preserves historical information and call metadata
    /// 
    /// # State Transitions
    /// 
    /// Special handling is applied for specific state transitions:
    /// - **Connected**: Sets `connected_at` timestamp if not already set
    /// - **Terminated/Failed/Cancelled**: Sets `ended_at` timestamp if not already set
    /// - **Other states**: Updates state without timestamp modifications
    /// 
    /// # Event Emission
    /// 
    /// When a state change is detected, the method automatically emits a `CallStatusInfo`
    /// event to the registered client event handler, providing:
    /// - Current and previous states
    /// - Transition timestamp
    /// - Call identification information
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use rvoip_client_core::call::CallId;
    /// # use rvoip_session_core::api::types::{CallSession, CallState, SessionId};
    /// # use std::sync::Arc;
    /// # use dashmap::DashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// let call_id = CallId::new_v4();
    /// # let session = CallSession {
    /// #     id: SessionId("session123".to_string()),
    /// #     from: "sip:alice@example.com".to_string(),
    /// #     to: "sip:bob@example.com".to_string(),
    /// #     state: CallState::Active,
    /// #     started_at: Some(std::time::Instant::now()),
    /// # };
    /// 
    /// // Update call info when session state changes
    /// handler.update_call_info_from_session(call_id, &session).await;
    /// 
    /// // The method will:
    /// // 1. Map CallState::Active to client-core Connected state
    /// // 2. Set connected_at timestamp if transitioning to Connected
    /// // 3. Emit state change event to registered handlers
    /// # }
    /// ```
    /// 
    /// # Thread Safety
    /// 
    /// This method is async and thread-safe. It uses atomic operations on the
    /// underlying DashMap storage and properly handles concurrent access to call information.
    pub async fn update_call_info_from_session(&self, call_id: CallId, session: &CallSession) {
        if let Some(mut call_info_ref) = self.call_info.get_mut(&call_id) {
            // Update state if it changed
            let new_client_state = self.map_session_state_to_client_state(&session.state);
            let old_state = call_info_ref.state.clone();
            
            if new_client_state != old_state {
                // Update timestamps based on state transition
                match new_client_state {
                    crate::call::CallState::Connected => {
                        if call_info_ref.connected_at.is_none() {
                            call_info_ref.connected_at = Some(Utc::now());
                        }
                    }
                    crate::call::CallState::Terminated | 
                    crate::call::CallState::Failed | 
                    crate::call::CallState::Cancelled => {
                        if call_info_ref.ended_at.is_none() {
                            call_info_ref.ended_at = Some(Utc::now());
                        }
                    }
                    _ => {}
                }
                
                call_info_ref.state = new_client_state.clone();
                
                // Emit state change event
                if let Some(handler) = self.client_event_handler.read().await.as_ref() {
                    let status_info = CallStatusInfo {
                        call_id,
                        new_state: new_client_state,
                        previous_state: Some(old_state),
                        reason: None,
                        timestamp: Utc::now(),
                    };
                    handler.on_call_state_changed(status_info).await;
                }
            }
        }
    }
    
    /// Map session-core CallState to client-core CallState with enhanced logic
    /// 
    /// This method translates between the internal session-core call state representation
    /// and the client-facing call state representation. It provides a clean abstraction
    /// layer and applies enhanced logic for complex state mappings.
    /// 
    /// # Arguments
    /// 
    /// * `session_state` - The session-core CallState to map to client-core representation
    /// 
    /// # Returns
    /// 
    /// The corresponding client-core CallState that represents the same logical state
    /// but with client-appropriate semantics and naming.
    /// 
    /// # State Mapping Logic
    /// 
    /// The mapping applies the following transformations:
    /// 
    /// | Session-Core State | Client-Core State | Notes |
    /// |-------------------|------------------|--------|
    /// | `Initiating` | `Initiating` | Direct mapping |
    /// | `Ringing` | `Ringing` | Direct mapping |
    /// | `Active` | `Connected` | Semantic clarity for client |
    /// | `OnHold` | `Connected` | Still connected, just on hold |
    /// | `Transferring` | `Proceeding` | Transfer in progress |
    /// | `Terminating` | `Terminating` | Direct mapping |
    /// | `Terminated` | `Terminated` | Direct mapping |
    /// | `Cancelled` | `Cancelled` | Direct mapping |
    /// | `Failed(reason)` | `Failed` | Logs reason, maps to simple Failed |
    /// 
    /// # Enhanced Logic
    /// 
    /// - **OnHold Handling**: Calls on hold are still considered "Connected" from the client
    ///   perspective, as the media session is established and can be resumed.
    /// - **Transfer Handling**: Calls being transferred are mapped to "Proceeding" to indicate
    ///   ongoing call setup activities.
    /// - **Failure Handling**: Failed states with reasons are logged for debugging but
    ///   simplified to a single Failed state for client consumption.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use rvoip_session_core::api::types::CallState;
    /// # use std::sync::Arc;
    /// # use dashmap::DashMap;
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// // Map active session to connected client state
    /// let session_state = CallState::Active;
    /// let client_state = handler.map_session_state_to_client_state(&session_state);
    /// assert_eq!(client_state, rvoip_client_core::call::CallState::Connected);
    /// 
    /// // Map on-hold session to still connected client state
    /// let session_state = CallState::OnHold;
    /// let client_state = handler.map_session_state_to_client_state(&session_state);
    /// assert_eq!(client_state, rvoip_client_core::call::CallState::Connected);
    /// 
    /// // Map failed session to failed client state (reason is logged)
    /// let session_state = CallState::Failed("Network timeout".to_string());
    /// let client_state = handler.map_session_state_to_client_state(&session_state);
    /// assert_eq!(client_state, rvoip_client_core::call::CallState::Failed);
    /// ```
    /// 
    /// # Logging
    /// 
    /// The method logs debug information for failed states to assist with troubleshooting
    /// while providing a clean interface to client applications.
    pub fn map_session_state_to_client_state(&self, session_state: &CallState) -> crate::call::CallState {
        match session_state {
            CallState::Initiating => crate::call::CallState::Initiating,
            CallState::Ringing => crate::call::CallState::Ringing,
            CallState::Active => crate::call::CallState::Connected,
            CallState::OnHold => crate::call::CallState::Connected, // Still connected, just on hold
            CallState::Transferring => crate::call::CallState::Proceeding,
            CallState::Terminating => crate::call::CallState::Terminating,
            CallState::Terminated => crate::call::CallState::Terminated,
            CallState::Cancelled => crate::call::CallState::Cancelled,
            CallState::Failed(reason) => {
                tracing::debug!("Call failed with reason: {}", reason);
                crate::call::CallState::Failed
            }
        }
    }
}

/// Implementation of session-core CallHandler trait for ClientCallHandler
/// 
/// This trait implementation bridges session-core events to client-core events,
/// providing the core event translation and handling logic. The implementation
/// receives low-level session events and transforms them into high-level client
/// events that applications can easily consume.
/// 
/// # Event Flow
/// 
/// 1. **Session-core events** arrive through this trait implementation
/// 2. **Event translation** maps session concepts to client concepts
/// 3. **State management** updates call information and mappings
/// 4. **Client events** are emitted to registered handlers and broadcast channels
/// 5. **Cleanup** removes temporary state when calls complete
/// 
/// # Thread Safety
/// 
/// All methods in this implementation are async and thread-safe, using
/// atomic operations and concurrent data structures for state management.
#[async_trait::async_trait]
impl CallHandler for ClientCallHandler {
    /// Handle incoming call from session-core layer
    /// 
    /// This method is called by session-core when a new incoming call arrives.
    /// It performs comprehensive call processing including ID mapping, metadata extraction,
    /// event emission, and decision routing to the application layer.
    /// 
    /// # Arguments
    /// 
    /// * `call` - The IncomingCall object from session-core containing all call details
    /// 
    /// # Returns
    /// 
    /// `CallDecision` indicating how session-core should handle the call:
    /// - `Accept(sdp)` - Accept the call with optional SDP answer
    /// - `Reject(reason)` - Reject the call with a reason string  
    /// - `Defer` - Defer the decision (used for ignore action)
    /// 
    /// # Processing Flow
    /// 
    /// 1. **ID Mapping**: Creates new client call ID and establishes bidirectional mapping
    /// 2. **Metadata Extraction**: Extracts display names, subject, and SIP headers
    /// 3. **Call Info Creation**: Creates comprehensive CallInfo with all available data
    /// 4. **Event Broadcasting**: Emits incoming call event to broadcast channel if configured
    /// 5. **Handler Consultation**: Forwards to application event handler for decision
    /// 6. **Decision Translation**: Maps client decision back to session-core format
    /// 
    /// # SDP Handling
    /// 
    /// - If the incoming call contains an SDP offer and the application accepts,
    ///   the method allows session-core to generate the SDP answer automatically
    /// - If no SDP offer is present, the call is accepted without SDP negotiation
    /// - Complex SDP scenarios are handled transparently by session-core
    /// 
    /// # State Management
    /// 
    /// - Call starts in `IncomingPending` state
    /// - Full call information is stored for history and reporting
    /// - Incoming call object is stored for later access during accept/reject operations
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use rvoip_session_core::api::types::{IncomingCall, CallDecision, SessionId};
    /// # use rvoip_session_core::CallHandler;
    /// # use std::sync::Arc;
    /// # use std::collections::HashMap;
    /// # use dashmap::DashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// # let incoming_call = IncomingCall {
    /// #     id: SessionId("session123".to_string()),
    /// #     from: "\"Alice Smith\" <sip:alice@example.com>".to_string(),
    /// #     to: "sip:bob@example.com".to_string(),
    /// #     sdp: Some("v=0...".to_string()),
    /// #     headers: HashMap::new(),
    /// #     received_at: std::time::Instant::now(),
    /// # };
    /// 
    /// // This method is called automatically by session-core
    /// let decision = handler.on_incoming_call(incoming_call).await;
    /// 
    /// // The method will:
    /// // 1. Extract caller display name "Alice Smith"
    /// // 2. Create call info with all metadata
    /// // 3. Emit incoming call event to application
    /// // 4. Return application's accept/reject decision
    /// # }
    /// ```
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Map session to call
        let call_id = CallId::new_v4();
        self.call_mapping.insert(call.id.clone(), call_id);
        self.session_mapping.insert(call_id, call.id.clone());
        
        // Store the IncomingCall for later use in answer/reject
        self.incoming_calls.insert(call_id, call.clone());
        
        // Enhanced call info extraction
        let caller_display_name = self.extract_display_name(&call.from, &call.headers);
        let subject = self.extract_subject(&call.headers);
        let sip_call_id = self.extract_call_id(&call.headers)
            .unwrap_or_else(|| call.id.0.clone());
        
        // Create comprehensive call info
        let call_info = CallInfo {
            call_id,
            state: crate::call::CallState::IncomingPending,
            direction: CallDirection::Incoming,
            local_uri: call.to.clone(),
            remote_uri: call.from.clone(),
            remote_display_name: caller_display_name.clone(),
            subject: subject.clone(),
            created_at: Utc::now(),
            connected_at: None,
            ended_at: None,
            remote_addr: None, // TODO: Extract from session if available
            media_session_id: None,
            sip_call_id,
            metadata: call.headers.clone(),
        };
        
        // Store call info
        self.call_info.insert(call_id, call_info.clone());
        
        // Create incoming call info for event
        let incoming_call_info = IncomingCallInfo {
            call_id,
            caller_uri: call.from.clone(),
            callee_uri: call.to.clone(),
            caller_display_name,
            subject,
            created_at: Utc::now(),
        };
        
        // Broadcast event
        if let Some(event_tx) = &self.event_tx {
            let _ = event_tx.send(crate::events::ClientEvent::IncomingCall { 
                info: incoming_call_info.clone(),
                priority: crate::events::EventPriority::High,
            });
        }
        
        // Forward to client event handler
        if let Some(handler) = self.client_event_handler.read().await.as_ref() {
            let action = handler.on_incoming_call(incoming_call_info).await;
            match action {
                crate::events::CallAction::Accept => {
                    // When Accept is returned, we need to generate SDP answer and accept the call
                    tracing::info!("Handler returned Accept for call {}, generating SDP answer", call_id);
                    
                    // Generate SDP answer if the incoming call has an offer
                    let sdp_answer = if let Some(_offer) = &call.sdp {
                        // Use session-core's media control to generate answer
                        // Note: We need access to the coordinator here, which we don't have directly
                        // So we'll let session-core handle SDP generation by passing None
                        // and marking that we need SDP generation
                        tracing::info!("Incoming call has SDP offer, will generate answer in session-core");
                        None // Let session-core generate the answer
                    } else {
                        tracing::info!("No SDP offer in incoming call, accepting without SDP");
                        None
                    };
                    
                    // Return Accept with the SDP (or None to let session-core generate it)
                    CallDecision::Accept(sdp_answer)
                }
                crate::events::CallAction::Reject => CallDecision::Reject("Call rejected by user".to_string()),
                crate::events::CallAction::Ignore => CallDecision::Defer,
            }
        } else {
            CallDecision::Reject("No event handler configured".to_string())
        }
    }
    
    /// Handle call termination from session-core layer
    /// 
    /// This method is called by session-core when a call ends, regardless of the cause
    /// (user hangup, network failure, timeout, etc.). It performs cleanup operations,
    /// updates call statistics, and emits final state change events.
    /// 
    /// # Arguments
    /// 
    /// * `session` - The CallSession object containing final call state and metadata
    /// * `reason` - Human-readable string describing why the call ended
    /// 
    /// # Processing Flow
    /// 
    /// 1. **Call Lookup**: Maps session ID to client call ID
    /// 2. **Statistics Update**: Handles connected call counter management
    /// 3. **State Finalization**: Updates call info with final state and timestamp
    /// 4. **Metadata Preservation**: Stores termination reason and final state
    /// 5. **Event Emission**: Broadcasts call ended event to handlers
    /// 6. **Cleanup**: Removes active mappings while preserving call history
    /// 
    /// # Critical Bug Fix
    /// 
    /// This method includes a critical fix for integer overflow in call statistics.
    /// Previously, calls ending through session-core (network timeouts, remote hangup)
    /// weren't decrementing the connected_calls counter, leading to overflow issues.
    /// 
    /// The fix:
    /// - Tracks whether the call was in Connected state before termination
    /// - Adds metadata flag for the manager to decrement counters appropriately
    /// - Prevents statistics corruption from external call termination
    /// 
    /// # State Management
    /// 
    /// - Final call state is mapped from session-core to client-core representation
    /// - `ended_at` timestamp is set to current time
    /// - Termination reason is stored in call metadata for history
    /// - Call information is preserved for reporting and analytics
    /// 
    /// # Cleanup Operations
    /// 
    /// - Removes active session-to-call and call-to-session mappings
    /// - Preserves call_info for historical access and reporting
    /// - Cleans up any temporary state related to the call
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use rvoip_session_core::api::types::{CallSession, CallState, SessionId};
    /// # use rvoip_session_core::CallHandler;
    /// # use std::sync::Arc;
    /// # use dashmap::DashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// # let session = CallSession {
    /// #     id: SessionId("session123".to_string()),
    /// #     from: "sip:alice@example.com".to_string(),
    /// #     to: "sip:bob@example.com".to_string(),
    /// #     state: CallState::Terminated,
    /// #     started_at: Some(std::time::Instant::now()),
    /// # };
    /// 
    /// // This method is called automatically by session-core
    /// handler.on_call_ended(session, "User hangup").await;
    /// 
    /// // The method will:
    /// // 1. Update call info to Terminated state
    /// // 2. Set ended_at timestamp
    /// // 3. Store "User hangup" in metadata
    /// // 4. Emit final state change event
    /// // 5. Clean up active mappings
    /// # }
    /// ```
    async fn on_call_ended(&self, session: CallSession, reason: &str) {
        // Map session to client call and emit event
        if let Some(call_id) = self.call_mapping.get(&session.id).map(|entry| *entry.value()) {
            // Check if the call was previously connected to determine if we need to decrement counter
            let was_connected = if let Some(call_info) = self.call_info.get(&call_id) {
                call_info.state == crate::call::CallState::Connected
            } else {
                false
            };
            
            // Update call info with final state
            if let Some(mut call_info_ref) = self.call_info.get_mut(&call_id) {
                call_info_ref.state = self.map_session_state_to_client_state(&session.state);
                call_info_ref.ended_at = Some(Utc::now());
                
                // Add termination reason to metadata
                call_info_ref.metadata.insert("termination_reason".to_string(), reason.to_string());
            }
            
            // Update stats - decrement connected_calls if the call was connected
            // This fixes the critical integer overflow bug where calls ending through
            // session-core (network timeouts, remote hangup, etc.) weren't decrementing the counter
            if was_connected {
                // We need access to the stats, but we don't have it directly here.
                // We'll emit a special event that the manager can handle to update stats.
                tracing::debug!("Call {} was connected and ended, should decrement connected_calls counter", call_id);
                
                // Since we can't access stats directly, we'll add metadata to let
                // the manager know to decrement the counter when processing this event
                if let Some(mut call_info_ref) = self.call_info.get_mut(&call_id) {
                    call_info_ref.metadata.insert("was_connected_when_ended".to_string(), "true".to_string());
                }
            }
            
            let status_info = CallStatusInfo {
                call_id,
                new_state: self.map_session_state_to_client_state(&session.state),
                previous_state: None, // TODO: Track previous state
                reason: Some(reason.to_string()),
                timestamp: Utc::now(),
            };
            
            // Broadcast event
            if let Some(event_tx) = &self.event_tx {
                let _ = event_tx.send(crate::events::ClientEvent::CallStateChanged { 
                    info: status_info.clone(),
                    priority: crate::events::EventPriority::Normal,
                });
            }
            
            // Forward to client event handler
            if let Some(handler) = self.client_event_handler.read().await.as_ref() {
                handler.on_call_state_changed(status_info).await;
            }
            
            // Phase 2: Clean up mappings but keep call_info for history
            // This is now safe because we're in Phase 2 (final cleanup)
            self.call_mapping.remove(&session.id);
            self.session_mapping.remove(&call_id);
            
            // Send cleanup confirmation back to session-core
            // This confirms that client-core has completed its cleanup for this session
            if let Some(session_event_tx) = self.session_event_tx.read().await.as_ref() {
                use rvoip_session_core::manager::events::SessionEvent;
                let _ = session_event_tx.send(SessionEvent::CleanupConfirmation {
                    session_id: session.id.clone(),
                    layer: "Client".to_string(),
                }).await;
                tracing::debug!("Sent cleanup confirmation for session {} from client-core", session.id);
            }
        }
    }
    
    /// Handle successful call establishment from session-core layer
    /// 
    /// This method is called by session-core when a call is successfully established
    /// and media can flow between participants. It represents the transition from
    /// call setup to active communication phase.
    /// 
    /// # Arguments
    /// 
    /// * `session` - The CallSession object containing established call state
    /// * `local_sdp` - Optional SDP offer/answer generated locally
    /// * `remote_sdp` - Optional SDP offer/answer received from remote party
    /// 
    /// # Processing Flow
    /// 
    /// 1. **Call Lookup**: Maps session ID to client call ID
    /// 2. **State Update**: Transitions call to Connected state
    /// 3. **Timestamp Recording**: Sets connected_at timestamp for analytics
    /// 4. **SDP Storage**: Preserves SDP information in call metadata
    /// 5. **Event Emission**: Broadcasts call established event with high priority
    /// 6. **Logging**: Records successful establishment for debugging
    /// 
    /// # SDP Information Management
    /// 
    /// Both local and remote SDP information is stored in call metadata:
    /// - `local_sdp`: The SDP offer or answer generated by the local endpoint
    /// - `remote_sdp`: The SDP offer or answer received from the remote endpoint
    /// 
    /// This information is crucial for:
    /// - Media session management and troubleshooting
    /// - Codec negotiation analysis
    /// - Network path and capability verification
    /// - Call quality investigation
    /// 
    /// # State Transitions
    /// 
    /// - Updates call state to `Connected`
    /// - Sets `connected_at` timestamp if not already set
    /// - Preserves call establishment timing for billing and analytics
    /// 
    /// # Event Priority
    /// 
    /// Call establishment events are emitted with `High` priority because:
    /// - They represent successful completion of call setup
    /// - Applications often need immediate notification for UI updates
    /// - Statistics and billing systems require prompt notification
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// # use rvoip_client_core::client::events::ClientCallHandler;
    /// # use rvoip_session_core::api::types::{CallSession, CallState, SessionId};
    /// # use rvoip_session_core::CallHandler;
    /// # use std::sync::Arc;
    /// # use dashmap::DashMap;
    /// # #[tokio::main]
    /// # async fn main() {
    /// let handler = ClientCallHandler::new(
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    ///     Arc::new(DashMap::new()),
    /// );
    /// 
    /// # let session = CallSession {
    /// #     id: SessionId("session123".to_string()),
    /// #     from: "sip:alice@example.com".to_string(),
    /// #     to: "sip:bob@example.com".to_string(),
    /// #     state: CallState::Active,
    /// #     started_at: Some(std::time::Instant::now()),
    /// # };
    /// 
    /// let local_sdp = Some("v=0\r\no=alice 123456 654321 IN IP4 192.168.1.100\r\n...".to_string());
    /// let remote_sdp = Some("v=0\r\no=bob 789012 210987 IN IP4 192.168.1.200\r\n...".to_string());
    /// 
    /// // This method is called automatically by session-core
    /// handler.on_call_established(session, local_sdp, remote_sdp).await;
    /// 
    /// // The method will:
    /// // 1. Update call state to Connected
    /// // 2. Set connected_at timestamp
    /// // 3. Store both local and remote SDP in metadata
    /// // 4. Emit high-priority call established event
    /// // 5. Log successful establishment
    /// # }
    /// ```
    async fn on_call_established(&self, session: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        // Map session to client call
        if let Some(call_id) = self.call_mapping.get(&session.id).map(|entry| *entry.value()) {
            // Update call info with establishment
            if let Some(mut call_info_ref) = self.call_info.get_mut(&call_id) {
                call_info_ref.state = crate::call::CallState::Connected;
                if call_info_ref.connected_at.is_none() {
                    call_info_ref.connected_at = Some(Utc::now());
                }
                
                // Store SDP information
                if let Some(local_sdp) = &local_sdp {
                    call_info_ref.metadata.insert("local_sdp".to_string(), local_sdp.clone());
                }
                if let Some(remote_sdp) = &remote_sdp {
                    call_info_ref.metadata.insert("remote_sdp".to_string(), remote_sdp.clone());
                    
                    // Process the SDP answer to configure RTP endpoints
                    // Note: We don't have direct access to ClientManager here, but that's OK
                    // because session-core will handle the SDP processing when it receives
                    // the CallAnswered event. The remote SDP is already stored in metadata.
                }
            }
            
            let status_info = CallStatusInfo {
                call_id,
                new_state: crate::call::CallState::Connected,
                previous_state: Some(crate::call::CallState::Proceeding),
                reason: Some("Call established".to_string()),
                timestamp: Utc::now(),
            };
            
            // Broadcast event
            if let Some(event_tx) = &self.event_tx {
                let _ = event_tx.send(crate::events::ClientEvent::CallStateChanged { 
                    info: status_info.clone(),
                    priority: crate::events::EventPriority::High,
                });
            }
            
            // Forward to client event handler
            if let Some(handler) = self.client_event_handler.read().await.as_ref() {
                handler.on_call_state_changed(status_info).await;
            }
            
            // Notify about call establishment for audio setup
            if let Some(tx) = &self.call_established_tx {
                let _ = tx.send(call_id);
            }
            
            tracing::info!("Call {} established with SDP exchange", call_id);
        }
    }
}
