use std::sync::Arc;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::sync::RwLock;
use anyhow::Result;
use tracing::{debug, warn};

use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri, Header, HeaderName};
use rvoip_transaction_core::TransactionManager;
use rvoip_session_core::{
    SessionManager, SessionId, SessionState, SessionConfig,
    EventBus
};

// Import nested types we need
use rvoip_session_core::session::SessionDirection;
use rvoip_session_core::media::AudioCodecType;

use crate::errors::Error;
use crate::routing::Router;
use crate::policy::{PolicyEngine, PolicyEnforcer, PolicyDecision};
use crate::registry::Registry;

/// Configuration for the call engine
#[derive(Debug, Clone)]
pub struct CallEngineConfig {
    /// Local signaling address
    pub local_signaling_addr: SocketAddr,
    
    /// Local media address
    pub local_media_addr: SocketAddr,
    
    /// Local domain
    pub local_domain: String,
    
    /// User agent string
    pub user_agent: String,
    
    /// Maximum sessions
    pub max_sessions: usize,
    
    /// Session cleanup interval
    pub cleanup_interval: Duration,
}

impl Default for CallEngineConfig {
    fn default() -> Self {
        Self {
            local_signaling_addr: "0.0.0.0:5060".parse().unwrap(),
            local_media_addr: "0.0.0.0:10000".parse().unwrap(),
            local_domain: "rvoip.local".to_string(),
            user_agent: "RVOIP/0.1.0".to_string(),
            max_sessions: 1000,
            cleanup_interval: Duration::from_secs(60),
        }
    }
}

/// Core engine for managing calls
pub struct CallEngine {
    /// Configuration
    config: CallEngineConfig,
    
    /// Session manager
    session_manager: Arc<SessionManager>,
    
    /// Transaction manager
    transaction_manager: Arc<TransactionManager>,
    
    /// Router
    router: RwLock<Router>,
    
    /// Policy engine
    policy: Arc<PolicyEngine>,
    
    /// Registry for user/endpoint registration
    registry: Arc<Registry>,
    
    /// Event bus
    event_bus: EventBus,
}

impl CallEngine {
    /// Create a new call engine
    pub fn new(
        config: CallEngineConfig,
        transaction_manager: Arc<TransactionManager>,
    ) -> Self {
        let event_bus = EventBus::new(1000); // Capacity for 1000 events
        let registry = Arc::new(Registry::new());
        let policy = Arc::new(PolicyEngine::new());
        
        let session_config = SessionConfig {
            local_signaling_addr: config.local_signaling_addr,
            local_media_addr: config.local_media_addr,
            display_name: None,
            user_agent: config.user_agent.clone(),
            max_duration: 0,
            supported_codecs: vec![
                AudioCodecType::PCMU,
                AudioCodecType::PCMA,
            ],
        };
        
        // Changed order of parameters to match SessionManager::new
        let session_manager = Arc::new(SessionManager::new(
            transaction_manager.clone(),
            session_config,
            event_bus.clone()
        ));
        
        let router = RwLock::new(Router::new(registry.clone()));
        
        Self {
            config,
            session_manager,
            transaction_manager,
            router,
            policy,
            registry,
            event_bus,
        }
    }
    
    /// Create a new call engine with a custom policy engine
    pub fn new_with_policy(
        config: CallEngineConfig,
        transaction_manager: Arc<TransactionManager>,
        custom_policy: Option<PolicyEngine>,
    ) -> Self {
        let event_bus = EventBus::new(1000); // Capacity for 1000 events
        let registry = Arc::new(Registry::new());
        
        // Use the provided policy engine or create a default one
        let policy = match custom_policy {
            Some(p) => Arc::new(p),
            None => Arc::new(PolicyEngine::new()),
        };
        
        let session_config = SessionConfig {
            local_signaling_addr: config.local_signaling_addr,
            local_media_addr: config.local_media_addr,
            display_name: None,
            user_agent: config.user_agent.clone(),
            max_duration: 0,
            supported_codecs: vec![
                AudioCodecType::PCMU,
                AudioCodecType::PCMA,
            ],
        };
        
        // Changed order of parameters to match SessionManager::new
        let session_manager = Arc::new(SessionManager::new(
            transaction_manager.clone(),
            session_config,
            event_bus.clone()
        ));
        
        let router = RwLock::new(Router::new(registry.clone()));
        
        Self {
            config,
            session_manager,
            transaction_manager,
            router,
            policy,
            registry,
            event_bus,
        }
    }
    
    /// Initialize the call engine
    pub async fn initialize(&self) -> Result<(), Error> {
        // Start session cleanup task
        self.start_cleanup_task();
        
        // Subscribe to transaction events
        self.subscribe_to_transaction_events().await?;
        
        // Subscribe to session events
        self.subscribe_to_session_events().await?;
        
        Ok(())
    }
    
    /// Subscribe to transaction events
    async fn subscribe_to_transaction_events(&self) -> Result<(), Error> {
        // TODO: Implement transaction event subscription
        Ok(())
    }
    
    /// Subscribe to session events
    async fn subscribe_to_session_events(&self) -> Result<(), Error> {
        // TODO: Implement session event subscription
        Ok(())
    }
    
    /// Start the session cleanup task
    fn start_cleanup_task(&self) {
        let session_manager = self.session_manager.clone();
        let interval = self.config.cleanup_interval;
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            
            loop {
                interval.tick().await;
                session_manager.cleanup_terminated().await;
            }
        });
    }
    
    /// Handle an incoming SIP request
    pub async fn handle_request(&self, request: Request, source: SocketAddr) -> Result<Response, Error> {
        debug!("Handling {} request from {}", request.method, source);
        
        // Check policy
        match self.policy.decide_incoming_request(&request).await? {
            PolicyDecision::Allow => {
                // Process allowed request
            },
            PolicyDecision::Deny => {
                // Request denied by policy
                return Ok(Response::new(StatusCode::Forbidden));
            },
            PolicyDecision::Challenge => {
                // Authentication required
                return self.policy.challenge_request(&request).await;
            }
        }
        
        // Handle the request based on method
        match request.method {
            Method::Register => {
                // Registration handling
                self.registry.handle_register(&request, source)
            },
            Method::Invite => {
                // Call setup
                self.handle_invite(request, source).await
            },
            Method::Bye => {
                // Call termination
                self.handle_bye(request).await
            },
            Method::Cancel => {
                // Cancel pending INVITE
                self.handle_cancel(request).await
            },
            Method::Options => {
                // Server capabilities
                self.handle_options(request).await
            },
            _ => {
                // Forward to existing dialog if it exists
                self.handle_in_dialog_request(request).await
            }
        }
    }
    
    /// Handle an INVITE request
    async fn handle_invite(&self, request: Request, _source: SocketAddr) -> Result<Response, Error> {
        // Check for existing dialog
        let call_id = request.header(&HeaderName::CallId)
            .ok_or_else(|| Error::other("Missing Call-ID header"))?
            .value.as_text()
            .ok_or_else(|| Error::other("Invalid Call-ID header format"))?;
        
        // Get To and From tags
        let to_tag = request.header(&HeaderName::To)
            .and_then(|h| h.value.as_text())
            .and_then(|t| extract_tag(t));
        
        let from_tag = request.header(&HeaderName::From)
            .and_then(|h| h.value.as_text())
            .and_then(|t| extract_tag(t))
            .ok_or_else(|| Error::other("Missing or invalid From tag"))?;
        
        // Check if this is a re-INVITE (has To tag)
        if let Some(to_tag) = to_tag {
            // This is a re-INVITE for an existing dialog
            let dialog_id = format!("{};{};{}", call_id, to_tag, from_tag);
            
            // For now we don't have dialog lookup method
            warn!("No session found for re-INVITE with dialog ID: {}", dialog_id);
            return Ok(Response::new(StatusCode::NotFound));
        }
        
        // New INVITE - create a new session
        debug!("Creating new session for INVITE from {}", request.uri);
        
        // Create session and return simple OK response for now
        let _session = self.session_manager.create_incoming_session(request.clone())
            .await
            .map_err(|e| Error::other(format!("Failed to create session: {}", e)))?;
        
        // Return a simple 200 OK
        let response = Response::new(StatusCode::Ok);
        
        Ok(response)
    }
    
    /// Handle a BYE request
    async fn handle_bye(&self, request: Request) -> Result<Response, Error> {
        // Extract identifiers
        let call_id = request.header(&HeaderName::CallId)
            .ok_or_else(|| Error::other("Missing Call-ID header"))?
            .value.as_text()
            .ok_or_else(|| Error::other("Invalid Call-ID header format"))?;
        
        let to_tag = request.header(&HeaderName::To)
            .and_then(|h| h.value.as_text())
            .and_then(|t| extract_tag(t))
            .ok_or_else(|| Error::other("Missing or invalid To tag"))?;
        
        let from_tag = request.header(&HeaderName::From)
            .and_then(|h| h.value.as_text())
            .and_then(|t| extract_tag(t))
            .ok_or_else(|| Error::other("Missing or invalid From tag"))?;
        
        // For now just return OK
        warn!("No session found for BYE with dialog ID: {}.{}.{}", call_id, to_tag, from_tag);
        Ok(Response::new(StatusCode::Ok))
    }
    
    /// Handle a CANCEL request
    async fn handle_cancel(&self, request: Request) -> Result<Response, Error> {
        // CANCEL applies to a pending INVITE transaction
        let call_id = request.header(&HeaderName::CallId)
            .ok_or_else(|| Error::other("Missing Call-ID header"))?
            .value.as_text()
            .ok_or_else(|| Error::other("Invalid Call-ID header format"))?;
        
        // TODO: Lookup the transaction in the transaction manager
        
        // No matching transaction found
        warn!("No session found for CANCEL with Call-ID: {}", call_id);
        Ok(Response::new(StatusCode::Ok))
    }
    
    /// Handle an OPTIONS request
    async fn handle_options(&self, _request: Request) -> Result<Response, Error> {
        // Create a 200 OK response
        let mut response = Response::new(StatusCode::Ok);
        
        // Add Allow header
        response.headers.push(Header::text(
            HeaderName::Allow,
            "INVITE, ACK, CANCEL, OPTIONS, BYE, REFER, NOTIFY, MESSAGE, SUBSCRIBE, INFO"
        ));
        
        // Add Supported header
        response.headers.push(Header::text(
            HeaderName::Supported,
            "path, replaces"
        ));
        
        // Add Accept header for application/sdp
        response.headers.push(Header::text(
            HeaderName::ContentType,
            "application/sdp"
        ));
        
        // Add User-Agent
        response.headers.push(Header::text(
            HeaderName::UserAgent,
            &self.config.user_agent
        ));
        
        Ok(response)
    }
    
    /// Handle an in-dialog request
    async fn handle_in_dialog_request(&self, request: Request) -> Result<Response, Error> {
        // Find the session for this dialog
        let call_id = request.header(&HeaderName::CallId)
            .ok_or_else(|| Error::other("Missing Call-ID header"))?;
        
        let call_id_str = match call_id {
            TypedHeader::CallId(call_id) => call_id.to_string(),
            _ => return Err(Error::other("Invalid Call-ID header format"))
        };
        
        let to_tag = request.header(&HeaderName::To)
            .and_then(|h| match h {
                TypedHeader::To(to) => to.tag(),
                _ => None
            })
            .ok_or_else(|| Error::other("Missing or invalid To tag"))?;
        
        let from_tag = request.header(&HeaderName::From)
            .and_then(|h| match h {
                TypedHeader::From(from) => from.tag(),
                _ => None
            })
            .ok_or_else(|| Error::other("Missing or invalid From tag"))?;
        
        // No matching dialog found
        warn!("No session found for request {} with dialog ID: {}.{}.{}", 
                request.method, call_id_str, to_tag, from_tag);
        return Ok(Response::new(StatusCode::NotFound));
    }
    
    /// Create an outgoing call
    pub async fn create_call(&self, to_uri: Uri, from_uri: Uri) -> Result<SessionId, Error> {
        debug!("Creating outgoing call from {} to {}", from_uri, to_uri);
        
        // Check policy for outgoing call
        let mut request = Request::new(Method::Invite, to_uri.clone());
        request.headers.push(Header::text(HeaderName::From, from_uri.to_string()));
        
        match self.policy.decide_outgoing_request(&request).await? {
            PolicyDecision::Allow => {
                // Call allowed by policy
            },
            PolicyDecision::Deny => {
                // Call denied by policy
                return Err(Error::PolicyViolation("Outgoing call not allowed".into()));
            },
            PolicyDecision::Challenge => {
                // Outgoing calls shouldn't need challenge
                return Err(Error::other("Unexpected challenge for outgoing call"));
            }
        }
        
        // Route the call
        let routes = self.router.read().await
            .find_routes(&to_uri)
            .map_err(|e| Error::Routing(format!("No route available: {}", e)))?;
        
        if routes.is_empty() {
            return Err(Error::Routing(format!("No route available for {}", to_uri)));
        }
        
        // Create a new session
        let session = self.session_manager.create_outgoing_session()
            .await
            .map_err(|e| Error::other(format!("Failed to create session: {}", e)))?;
        
        // TODO: Start the outgoing call process
        // For now, we just return the session ID
        
        Ok(session.id.clone())
    }
    
    /// Terminate a call
    pub async fn terminate_call(&self, session_id: &SessionId) -> Result<(), Error> {
        if let Some(session) = self.session_manager.get_session(session_id) {
            // Check current state
            let state = session.state().await;
            if state == SessionState::Terminated || state == SessionState::Terminating {
                debug!("Session {} already terminated or terminating", session_id);
                return Ok(());
            }
            
            // Set state to terminating
            session.set_state(SessionState::Terminating).await
                .map_err(|e| Error::other(format!("Failed to update session state: {}", e)))?;
            
            // Get dialog
            if let Some(mut dialog) = session.dialog().await {
                // Create BYE request
                let mut bye_request = dialog.create_request(Method::Bye);
                
                // Add User-Agent header
                bye_request.headers.push(Header::text(
                    HeaderName::UserAgent,
                    &self.config.user_agent
                ));
                
                // Send BYE request
                // TODO: Send the BYE request through transaction manager
            } else {
                warn!("No dialog found for session {}", session_id);
            }
            
            // Set state to terminated
            session.set_state(SessionState::Terminated).await
                .map_err(|e| Error::other(format!("Failed to update session state: {}", e)))?;
            
            Ok(())
        } else {
            Err(Error::other(format!("Session not found: {}", session_id)))
        }
    }
    
    /// Get status of a call
    pub async fn get_call_status(&self, session_id: &SessionId) -> Result<SessionState, Error> {
        if let Some(session) = self.session_manager.get_session(session_id) {
            Ok(session.state().await)
        } else {
            Err(Error::other(format!("Session not found: {}", session_id)))
        }
    }
    
    /// Get active calls - For now returns an empty list since the actual method is not implemented
    pub async fn get_active_calls(&self) -> Vec<SessionId> {
        // TODO: Implement once SessionManager has this method
        Vec::new()
    }
    
    /// Get registry
    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }
    
    /// Get router
    pub async fn router(&self) -> tokio::sync::RwLockReadGuard<'_, Router> {
        self.router.read().await
    }
    
    /// Get session manager
    pub fn session_manager(&self) -> Arc<SessionManager> {
        self.session_manager.clone()
    }
    
    /// Get transaction manager
    pub fn transaction_manager(&self) -> Arc<TransactionManager> {
        self.transaction_manager.clone()
    }
    
    /// Get policy engine
    pub fn policy(&self) -> &Arc<PolicyEngine> {
        &self.policy
    }
    
    /// Set a new policy engine
    pub fn set_policy(&mut self, policy: Arc<PolicyEngine>) {
        self.policy = policy;
    }
    
    /// Get event bus
    pub fn event_bus(&self) -> EventBus {
        self.event_bus.clone()
    }
}

/// Helper function to extract tag parameter from a header value
fn extract_tag(header_value: &str) -> Option<String> {
    if let Some(tag_pos) = header_value.find(";tag=") {
        let tag_start = tag_pos + 5; // length of ";tag="
        let tag_end = header_value[tag_start..].find(';')
            .map(|pos| tag_start + pos)
            .unwrap_or(header_value.len());
        Some(header_value[tag_start..tag_end].to_string())
    } else {
        None
    }
} 