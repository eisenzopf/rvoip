//! Web console HTTP + WebSocket server.
//!
//! Combines the API gateway, WebSocket hub, and static file serving
//! into a single Axum application.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::header::HeaderValue;
use axum::middleware::{self, Next};
use axum::response::Response;
use tower_http::cors::{CorsLayer, Any};
use tower_http::trace::TraceLayer;
use tokio::net::TcpListener;
use tracing::{info, warn};

use rvoip_call_engine::CallCenterEngine;
use rvoip_call_engine::monitoring::CallCenterEvents;
use rvoip_registrar_core::RegistrarService;
use users_core::auth::AuthenticationService;
use users_core::jwt::JwtConfig;

use crate::ws::WsHub;
use crate::ws::hub::WsEvent;

/// Tracks hourly call activity for the dashboard chart.
#[derive(Debug)]
pub struct ActivityTracker {
    pub calls: [u64; 24],
    pub queued: [u64; 24],
}

impl ActivityTracker {
    pub fn new() -> Self {
        Self {
            calls: [0; 24],
            queued: [0; 24],
        }
    }
}

/// Shared application state available to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<CallCenterEngine>,
    pub registrar: Option<Arc<RegistrarService>>,
    pub ws_hub: WsHub,
    pub activity_tracker: Arc<parking_lot::RwLock<ActivityTracker>>,
    /// Authentication service (optional — auth disabled when `None`).
    pub auth_service: Option<Arc<AuthenticationService>>,
    /// JWT decoding key for access-token validation (HS256 secret).
    pub decoding_key: Option<Arc<jsonwebtoken::DecodingKey>>,
    /// JWT configuration (issuer, audience, ttl, etc.).
    pub jwt_config: Option<JwtConfig>,
}

/// Configuration for the web console server.
#[derive(Debug, Clone)]
pub struct WebConsoleConfig {
    /// Address to bind the HTTP server to.
    pub listen_addr: SocketAddr,
    /// WebSocket broadcast channel capacity.
    pub ws_channel_capacity: usize,
    /// Enable CORS for development (allows all origins).
    pub dev_cors: bool,
    /// Path to TLS certificate PEM file (enables HTTPS when set together with `tls_key_path`).
    pub tls_cert_path: Option<String>,
    /// Path to TLS private key PEM file.
    pub tls_key_path: Option<String>,
}

impl Default for WebConsoleConfig {
    fn default() -> Self {
        Self {
            listen_addr: SocketAddr::from(([0, 0, 0, 0], 3000)),
            ws_channel_capacity: 1024,
            dev_cors: cfg!(debug_assertions),
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}

/// The web console server.
pub struct WebConsoleServer {
    config: WebConsoleConfig,
    state: AppState,
    events: Option<Arc<CallCenterEvents>>,
}

impl WebConsoleServer {
    /// Create a new web console server backed by the given call-center engine.
    pub fn new(engine: Arc<CallCenterEngine>, config: WebConsoleConfig) -> Self {
        let ws_hub = WsHub::new(config.ws_channel_capacity);
        let activity_tracker = Arc::new(parking_lot::RwLock::new(ActivityTracker::new()));
        let state = AppState {
            engine,
            registrar: None,
            ws_hub,
            activity_tracker,
            auth_service: None,
            decoding_key: None,
            jwt_config: None,
        };
        Self { config, state, events: None }
    }

    /// Attach a registrar service for SIP registration data.
    pub fn with_registrar(mut self, registrar: Arc<RegistrarService>) -> Self {
        self.state.registrar = Some(registrar);
        self
    }

    /// Attach call-center events for real-time WebSocket push.
    pub fn with_events(mut self, events: Arc<CallCenterEvents>) -> Self {
        self.events = Some(events);
        self
    }

    /// Attach authentication service, JWT decoding key, and config.
    pub fn with_auth(
        mut self,
        auth_service: Arc<AuthenticationService>,
        decoding_key: Arc<jsonwebtoken::DecodingKey>,
        jwt_config: JwtConfig,
    ) -> Self {
        self.state.auth_service = Some(auth_service);
        self.state.decoding_key = Some(decoding_key);
        self.state.jwt_config = Some(jwt_config);
        self
    }

    /// Get a reference to the WebSocket hub (for pushing events from outside).
    pub fn ws_hub(&self) -> &WsHub {
        &self.state.ws_hub
    }

    /// Build the Axum router.
    pub fn router(&self) -> Router {
        let api_routes = Router::new()
            .nest("/api/v1", crate::api::router())
            .nest("/ws", crate::ws::router())
            // Audit middleware runs AFTER the handler (wraps response).
            // It must be layered before auth so it sees the AuthUser extension.
            .layer(middleware::from_fn_with_state(
                self.state.clone(),
                crate::audit::audit_middleware,
            ))
            .layer(middleware::from_fn_with_state(
                self.state.clone(),
                crate::auth::auth_middleware,
            ))
            .with_state(self.state.clone());

        let mut app = api_routes
            .fallback_service(crate::static_files::router());

        // Rate limiting (innermost → runs first on request path)
        app = app.layer(middleware::from_fn(crate::rate_limit::rate_limit_middleware));

        app = app.layer(TraceLayer::new_for_http());

        if self.config.dev_cors {
            app = app.layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            );
        }

        // Security headers (outermost → runs last on request, first on response)
        app = app.layer(middleware::from_fn(security_headers));

        app
    }

    /// Start the event forwarding pipeline (call-engine events → WebSocket).
    fn spawn_event_pipeline(&self) {
        let Some(events) = &self.events else { return };
        let ws_hub = self.state.ws_hub.clone();
        let activity_tracker = self.state.activity_tracker.clone();
        let events = events.clone();

        tokio::spawn(async move {
            // Subscribe to all events (using Call as the generic catch-all subscriber)
            let mut rx = match events.subscribe(
                rvoip_call_engine::monitoring::events::EventType::Call,
            ).await {
                Ok(rx) => rx,
                Err(e) => {
                    warn!("Failed to subscribe to call-engine events: {}", e);
                    return;
                }
            };

            info!("Event pipeline started — forwarding call-engine events to WebSocket");

            loop {
                match rx.recv().await {
                    Ok(event) => {
                        // Update activity tracker
                        let hour = event.timestamp.format("%H")
                            .to_string()
                            .parse::<usize>()
                            .unwrap_or(0);

                        {
                            let mut tracker = activity_tracker.write();
                            let event_type_str = format!("{:?}", event.event_type);
                            if event_type_str.contains("Call") {
                                tracker.calls[hour] = tracker.calls[hour].saturating_add(1);
                            }
                            if event_type_str.contains("Queue") || event_type_str.contains("Enqueue") {
                                tracker.queued[hour] = tracker.queued[hour].saturating_add(1);
                            }
                        }

                        // Forward to WebSocket clients
                        let ws_event = WsEvent {
                            event_type: format!("{:?}", event.event_type),
                            timestamp: event.timestamp.to_rfc3339(),
                            data: serde_json::to_value(&event.data).unwrap_or_default(),
                        };
                        ws_hub.broadcast(ws_event);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Event pipeline lagged by {} events", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!("Event pipeline channel closed");
                        break;
                    }
                }
            }
        });
    }

    /// Ensure at least one admin user exists.
    ///
    /// If the user store is empty a default super-admin account is created
    /// using the password from `RVOIP_ADMIN_PASSWORD` (or `"admin123"`).
    async fn init_default_admin(&self) -> anyhow::Result<()> {
        let Some(auth_service) = &self.state.auth_service else {
            return Ok(());
        };

        let users = auth_service
            .user_store()
            .list_users(users_core::UserFilter {
                limit: Some(1),
                ..Default::default()
            })
            .await
            .map_err(|e| anyhow::anyhow!("failed to list users: {}", e))?;

        if !users.is_empty() {
            return Ok(());
        }

        let password = std::env::var("RVOIP_ADMIN_PASSWORD")
            .unwrap_or_else(|_| "Rvoip@Console2026!".to_string());

        let req = users_core::CreateUserRequest {
            username: "admin".to_string(),
            password,
            email: None,
            display_name: Some("System Administrator".to_string()),
            roles: vec!["super_admin".to_string()],
        };

        auth_service
            .create_user(req)
            .await
            .map_err(|e| anyhow::anyhow!("failed to create default admin: {}", e))?;

        info!("Created default super_admin user 'admin'");
        Ok(())
    }

    /// Initialize database tables used by the web console.
    async fn init_tables(&self) -> anyhow::Result<()> {
        // Audit log table
        if let Err(e) = crate::audit::init_audit_table(&self.state).await {
            warn!("Could not initialize audit_log table: {}", e);
        }
        // Overflow policies table (+ seed defaults)
        if let Err(e) = crate::api::routing::init_overflow_policies_table(&self.state).await {
            warn!("Could not initialize overflow_policies table: {}", e);
        }
        // Departments table (+ seed defaults)
        if let Err(e) = crate::api::departments::init_departments_table(&self.state).await {
            warn!("Could not initialize departments table: {}", e);
        }
        // Extension ranges + extensions tables (+ seed defaults)
        if let Err(e) = crate::api::extensions::init_extensions_tables(&self.state).await {
            warn!("Could not initialize extensions tables: {}", e);
        }
        // Skill definitions + agent_skills tables (+ seed defaults)
        if let Err(e) = crate::api::skills::init_skills_tables(&self.state).await {
            warn!("Could not initialize skills tables: {}", e);
        }
        // Phone lists table (blacklist/whitelist/VIP) (+ seed defaults)
        if let Err(e) = crate::api::phone_lists::init_phone_lists_table(&self.state).await {
            warn!("Could not initialize phone_lists table: {}", e);
        }
        // IVR menus + options tables (+ seed defaults)
        if let Err(e) = crate::api::ivr::init_ivr_tables(&self.state).await {
            warn!("Could not initialize ivr tables: {}", e);
        }
        // SIP trunks + DID numbers tables (+ seed defaults)
        if let Err(e) = crate::api::trunks::init_trunks_tables(&self.state).await {
            warn!("Could not initialize trunks tables: {}", e);
        }
        // Shifts + schedule_entries tables (+ seed defaults)
        if let Err(e) = crate::api::schedules::init_schedules_tables(&self.state).await {
            warn!("Could not initialize schedules tables: {}", e);
        }
        // QC templates + scores tables (+ seed defaults)
        if let Err(e) = crate::api::quality::init_quality_tables(&self.state).await {
            warn!("Could not initialize quality tables: {}", e);
        }
        // Knowledge articles + talk scripts tables (+ seed defaults)
        if let Err(e) = crate::api::knowledge::init_knowledge_tables(&self.state).await {
            warn!("Could not initialize knowledge tables: {}", e);
        }
        Ok(())
    }

    /// Start serving and block until shutdown.
    pub async fn serve(self) -> anyhow::Result<()> {
        self.init_default_admin().await?;
        self.init_tables().await?;
        self.spawn_event_pipeline();
        let app = self.router();

        // TLS mode: use axum-server with rustls when both cert and key are configured.
        #[cfg(feature = "tls")]
        if let (Some(cert), Some(key)) = (&self.config.tls_cert_path, &self.config.tls_key_path) {
            let rustls_config =
                axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key).await?;
            info!(addr = %self.config.listen_addr, "Web console listening (TLS)");
            axum_server::bind_rustls(self.config.listen_addr, rustls_config)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                .await?;
            return Ok(());
        }

        let listener = TcpListener::bind(self.config.listen_addr).await?;
        info!(addr = %self.config.listen_addr, "Web console listening");
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Security headers middleware
// ---------------------------------------------------------------------------

/// Axum middleware that appends security headers to every response.
async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert(
        "X-XSS-Protection",
        HeaderValue::from_static("1; mode=block"),
    );
    headers.insert(
        "Referrer-Policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "Content-Security-Policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline' https://esm.sh; style-src 'self' 'unsafe-inline'; font-src 'self' data:; img-src 'self' data:; connect-src 'self' ws://127.0.0.1:8080 wss://127.0.0.1:8080 ws://localhost:8080 wss://localhost:8080 https://esm.sh",
        ),
    );
    response
}
