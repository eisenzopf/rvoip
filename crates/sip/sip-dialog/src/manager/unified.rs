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
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::transaction::{TransactionEvent, TransactionKey, TransactionManager};
use rvoip_sip_core::{Method, Request, Response, StatusCode, Uri};

use crate::api::{
    common::{CallHandle, DialogHandle},
    ApiError, ApiResult,
};
use crate::config::DialogManagerConfig;
use crate::diagnostics::safe_log::method_class;
use crate::dialog::{Dialog, DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::DialogEvent;
use crate::subscription::SubscriptionManager;

// Import the existing core DialogManager functionality
use super::core::DialogManager;

const INITIAL_INVITE_INSTALLING: u8 = 0;
const INITIAL_INVITE_INSTALLED: u8 = 1;
const INITIAL_INVITE_DISPATCHING: u8 = 2;
const INITIAL_INVITE_SENT: u8 = 3;
const INITIAL_INVITE_WIRE_UNKNOWN: u8 = 4;
const INITIAL_INVITE_RELEASING: u8 = 5;

/// Exact ownership proof for a locally installed outbound initial INVITE.
///
/// The opaque token prevents stale cleanup work from releasing a later
/// installation that happens to use the same application session identifier.
/// Callers may retain this small handle in their own resource registry.
#[derive(Clone, PartialEq, Eq)]
pub struct InitialInviteOwner {
    dialog_id: DialogId,
    call_id: String,
    session_id: Option<String>,
    token: Uuid,
}

impl InitialInviteOwner {
    pub fn dialog_id(&self) -> &DialogId {
        &self.dialog_id
    }

    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

impl std::fmt::Debug for InitialInviteOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InitialInviteOwner")
            .field("dialog_id", &self.dialog_id)
            .field("call_id_len", &self.call_id.len())
            .field("session_id_present", &self.session_id.is_some())
            .finish_non_exhaustive()
    }
}

/// Side-effect-free outbound INVITE plan.
///
/// Planning parses and validates caller input, allocates the exact Dialog-ID
/// and Call-ID, and snapshots the registration Service-Route. It does not
/// publish dialog state, install mappings, create transactions, or send wire
/// bytes.
pub struct PlannedInitialInvite {
    owner: InitialInviteOwner,
    dialog: Dialog,
    options: crate::api::unified::InviteRequestOptions,
}

impl PlannedInitialInvite {
    pub fn owner(&self) -> &InitialInviteOwner {
        &self.owner
    }

    pub fn dialog_id(&self) -> &DialogId {
        self.owner.dialog_id()
    }

    pub fn call_id(&self) -> &str {
        self.owner.call_id()
    }
}

impl std::fmt::Debug for PlannedInitialInvite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlannedInitialInvite")
            .field("owner", &self.owner)
            .field("route_count", &self.dialog.route_set.len())
            .finish_non_exhaustive()
    }
}

/// A plan whose exact dialog and optional session mapping are installed
/// locally, but whose INVITE has not been dispatched.
pub struct InstalledInitialInvite {
    owner: InitialInviteOwner,
    options: crate::api::unified::InviteRequestOptions,
    lease: InitialInviteInstallLease,
}

impl InstalledInitialInvite {
    pub fn owner(&self) -> &InitialInviteOwner {
        &self.owner
    }
}

struct InitialInviteInstallLease {
    manager: UnifiedDialogManager,
    owner: InitialInviteOwner,
    armed: bool,
}

impl InitialInviteInstallLease {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for InitialInviteInstallLease {
    fn drop(&mut self) {
        if self.armed {
            self.manager
                .compensate_dropped_initial_invite_install(&self.owner);
        }
    }
}

impl std::fmt::Debug for InstalledInitialInvite {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InstalledInitialInvite")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

/// What is known about wire emission when an initial-INVITE dispatch finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialInviteWireOutcome {
    /// Dispatch failed before any transaction attempt could reach transport.
    ZeroWire,
    /// The transaction layer accepted one INVITE for transmission.
    Sent,
    /// An attempt reached transport, but failure timing makes emission
    /// impossible to prove either way.
    Unknown,
}

/// Dispatch failure that preserves the exact owner needed for subsequent
/// CANCEL/BYE teardown when wire emission is unknown.
pub struct InitialInviteDispatchError {
    owner: InitialInviteOwner,
    wire_outcome: InitialInviteWireOutcome,
    error: ApiError,
}

impl InitialInviteDispatchError {
    pub fn owner(&self) -> &InitialInviteOwner {
        &self.owner
    }

    pub fn wire_outcome(&self) -> InitialInviteWireOutcome {
        self.wire_outcome
    }

    pub fn error(&self) -> &ApiError {
        &self.error
    }

    pub fn into_api_error(self) -> ApiError {
        self.error
    }
}

impl std::fmt::Debug for InitialInviteDispatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InitialInviteDispatchError")
            .field("owner", &self.owner)
            .field("wire_outcome", &self.wire_outcome)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for InitialInviteDispatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "initial INVITE dispatch failed ({:?})",
            self.wire_outcome
        )
    }
}

impl std::error::Error for InitialInviteDispatchError {}

/// Terminal result from a retained initial-INVITE dispatch task.
pub struct InitialInviteDispatchCompletion {
    owner: InitialInviteOwner,
    wire_outcome: InitialInviteWireOutcome,
    transaction_id: Option<TransactionKey>,
    error: Option<ApiError>,
}

impl InitialInviteDispatchCompletion {
    pub fn owner(&self) -> &InitialInviteOwner {
        &self.owner
    }

    pub fn wire_outcome(&self) -> InitialInviteWireOutcome {
        self.wire_outcome
    }

    pub fn transaction_id(&self) -> Option<&TransactionKey> {
        self.transaction_id.as_ref()
    }

    pub fn error(&self) -> Option<&ApiError> {
        self.error.as_ref()
    }

    pub fn into_result(
        self,
    ) -> Result<(InitialInviteOwner, TransactionKey), InitialInviteDispatchError> {
        match (self.transaction_id, self.error) {
            (Some(transaction_id), None) => Ok((self.owner, transaction_id)),
            (_, Some(error)) => Err(InitialInviteDispatchError {
                owner: self.owner,
                wire_outcome: self.wire_outcome,
                error,
            }),
            _ => Err(InitialInviteDispatchError {
                owner: self.owner,
                wire_outcome: self.wire_outcome,
                error: ApiError::internal(
                    "Initial INVITE dispatch completed without a transaction",
                ),
            }),
        }
    }
}

impl std::fmt::Debug for InitialInviteDispatchCompletion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InitialInviteDispatchCompletion")
            .field("owner", &self.owner)
            .field("wire_outcome", &self.wire_outcome)
            .field("transaction_present", &self.transaction_id.is_some())
            .field("error_present", &self.error.is_some())
            .finish()
    }
}

enum InitialInviteDispatchTask {
    Running(tokio::sync::oneshot::Receiver<InitialInviteDispatchCompletion>),
    Ready(Option<InitialInviteDispatchCompletion>),
}

/// Result handle for a manager-owned retained dispatch. Dropping it does not
/// cancel or detach ownership; the manager registry joins the task on stop.
pub struct InitialInviteDispatch {
    manager: UnifiedDialogManager,
    owner: InitialInviteOwner,
    task: InitialInviteDispatchTask,
}

impl InitialInviteDispatch {
    pub fn owner(&self) -> &InitialInviteOwner {
        &self.owner
    }

    pub async fn wait(mut self) -> InitialInviteDispatchCompletion {
        let task = std::mem::replace(&mut self.task, InitialInviteDispatchTask::Ready(None));
        match task {
            InitialInviteDispatchTask::Ready(Some(completion)) => completion,
            InitialInviteDispatchTask::Ready(None) => InitialInviteDispatchCompletion {
                owner: self.owner,
                wire_outcome: InitialInviteWireOutcome::Unknown,
                transaction_id: None,
                error: Some(ApiError::internal(
                    "Initial INVITE dispatch completion was already consumed",
                )),
            },
            InitialInviteDispatchTask::Running(completion) => match completion.await {
                Ok(completion) => completion,
                Err(_task_closed) => {
                    self.manager.mark_initial_invite_wire_unknown(&self.owner);
                    self.manager
                        .supervise_wire_unknown_cleanup(self.owner.clone());
                    InitialInviteDispatchCompletion {
                        owner: self.owner,
                        wire_outcome: InitialInviteWireOutcome::Unknown,
                        transaction_id: None,
                        error: Some(ApiError::internal("Initial INVITE dispatch task failed")),
                    }
                }
            },
        }
    }
}

impl std::fmt::Debug for InitialInviteDispatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InitialInviteDispatch")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

struct InitialInviteInstallRecord {
    token: Uuid,
    phase: Arc<AtomicU8>,
    /// Admission is owned by the exact installation record. Removing the
    /// exact record drops the permit; stale cleanup cannot release a newer
    /// installation's capacity.
    _capacity_permit: tokio::sync::OwnedSemaphorePermit,
}

#[derive(Clone)]
struct InitialInviteCleanupTask {
    token: Uuid,
    abort: tokio::task::AbortHandle,
    completion: tokio::sync::watch::Receiver<bool>,
}

struct InitialInviteCleanupCompletion {
    sender: Option<tokio::sync::watch::Sender<bool>>,
}

impl Drop for InitialInviteCleanupCompletion {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(true);
        }
    }
}

#[derive(Clone)]
struct InitialInviteDispatchTaskRecord {
    token: Uuid,
    abort: tokio::task::AbortHandle,
    completion: tokio::sync::watch::Receiver<bool>,
}

struct InitialInviteDispatchExecution {
    manager: UnifiedDialogManager,
    owner: InitialInviteOwner,
    task_token: Uuid,
    completion: Option<tokio::sync::watch::Sender<bool>>,
    normal_completion: bool,
}

impl Drop for InitialInviteDispatchExecution {
    fn drop(&mut self) {
        if !self.normal_completion {
            self.manager.mark_initial_invite_wire_unknown(&self.owner);
            self.manager
                .supervise_wire_unknown_cleanup(self.owner.clone());
        }
        self.manager
            .initial_invite_dispatch_tasks
            .remove_if(&self.owner.dialog_id, |_, current| {
                current.token == self.task_token
            });
        if let Some(completion) = self.completion.take() {
            let _ = completion.send(true);
        }
    }
}

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
/// using `Arc<UnifiedDialogManager>`.
#[derive(Clone)]
pub struct UnifiedDialogManager {
    /// Core dialog manager (contains all the actual implementation)
    core: DialogManager,

    /// Configuration determining behavior mode
    config: DialogManagerConfig,

    /// Statistics for this manager instance
    stats: Arc<tokio::sync::RwLock<ManagerStats>>,

    /// Opaque exact-owner records for staged outbound initial INVITEs.
    /// This is deliberately only a resource-ownership index; dialog-core's
    /// existing lifecycle remains the sole signaling authority.
    initial_invite_installs: Arc<dashmap::DashMap<DialogId, Arc<InitialInviteInstallRecord>>>,

    /// The public staged-install boundary is independently bounded before it
    /// can publish any dialog/session state. The configured logical INVITE
    /// capacity is reused as the semaphore size.
    initial_invite_install_slots: Arc<tokio::sync::Semaphore>,

    /// Manager-owned retained dispatch tasks. Callers only observe a result
    /// receiver, so dropping a public handle cannot detach work from shutdown.
    initial_invite_dispatch_tasks: Arc<dashmap::DashMap<DialogId, InitialInviteDispatchTaskRecord>>,
    initial_invite_dispatch_gate: Arc<std::sync::Mutex<bool>>,

    /// One owned protocol-cleanup driver per legacy wrapper failure. The map
    /// bounds task count by installed dialogs and gives shutdown an exact
    /// abort set.
    initial_invite_cleanup_tasks: Arc<dashmap::DashMap<DialogId, InitialInviteCleanupTask>>,
    initial_invite_cleanup_gate: Arc<std::sync::Mutex<bool>>,

    #[cfg(test)]
    initial_invite_dispatch_test_hook: Arc<AtomicU8>,
    #[cfg(test)]
    initial_invite_cleanup_test_hook: Arc<AtomicU8>,
}

impl std::fmt::Debug for UnifiedDialogManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UnifiedDialogManager")
            .field("core", &self.core)
            .field("mode", &Self::mode_name(&self.config))
            .field(
                "outgoing_calls_supported",
                &self.config.supports_outgoing_calls(),
            )
            .field(
                "incoming_calls_supported",
                &self.config.supports_incoming_calls(),
            )
            .finish_non_exhaustive()
    }
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
        config
            .validate()
            .map_err(|_error| DialogError::internal_error("Invalid configuration", None))?;

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
        let initial_invite_install_capacity = core.invite_failover_active_plan_capacity;

        Ok(Self {
            core,
            config,
            stats: Arc::new(tokio::sync::RwLock::new(ManagerStats::default())),
            initial_invite_installs: Arc::new(dashmap::DashMap::new()),
            initial_invite_install_slots: Arc::new(tokio::sync::Semaphore::new(
                initial_invite_install_capacity,
            )),
            initial_invite_dispatch_tasks: Arc::new(dashmap::DashMap::new()),
            initial_invite_dispatch_gate: Arc::new(std::sync::Mutex::new(true)),
            initial_invite_cleanup_tasks: Arc::new(dashmap::DashMap::new()),
            initial_invite_cleanup_gate: Arc::new(std::sync::Mutex::new(true)),
            #[cfg(test)]
            initial_invite_dispatch_test_hook: Arc::new(AtomicU8::new(0)),
            #[cfg(test)]
            initial_invite_cleanup_test_hook: Arc::new(AtomicU8::new(0)),
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
        if let Err(_error) = config.validate() {
            error!("Failed to create UnifiedDialogManager: invalid configuration");
            return Err(DialogError::internal_error("Invalid configuration", None));
        }

        let local_address = config.local_address();
        info!(
            "Creating UnifiedDialogManager with global events in {:?} mode at {}",
            Self::mode_name(&config),
            local_address
        );

        // Create core dialog manager with global events
        let mut core = DialogManager::with_global_events_and_index_capacity_and_config(
            transaction_manager,
            transaction_events,
            local_address,
            config.dialog_config().max_dialogs.unwrap_or(10_000),
            Some(config.clone()),
        )
        .await?;

        // **NEW**: Inject the unified configuration into the core manager
        core.set_config(config.clone());
        let initial_invite_install_capacity = core.invite_failover_active_plan_capacity;

        Ok(Self {
            core,
            config,
            stats: Arc::new(tokio::sync::RwLock::new(ManagerStats::default())),
            initial_invite_installs: Arc::new(dashmap::DashMap::new()),
            initial_invite_install_slots: Arc::new(tokio::sync::Semaphore::new(
                initial_invite_install_capacity,
            )),
            initial_invite_dispatch_tasks: Arc::new(dashmap::DashMap::new()),
            initial_invite_dispatch_gate: Arc::new(std::sync::Mutex::new(true)),
            initial_invite_cleanup_tasks: Arc::new(dashmap::DashMap::new()),
            initial_invite_cleanup_gate: Arc::new(std::sync::Mutex::new(true)),
            #[cfg(test)]
            initial_invite_dispatch_test_hook: Arc::new(AtomicU8::new(0)),
            #[cfg(test)]
            initial_invite_cleanup_test_hook: Arc::new(AtomicU8::new(0)),
        })
    }

    /// Create a unified dialog manager on the pointer-sized authoritative
    /// transaction-event path.
    pub async fn with_shared_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<Arc<TransactionEvent>>,
        config: DialogManagerConfig,
    ) -> DialogResult<Self> {
        if config.validate().is_err() {
            error!("Failed to create UnifiedDialogManager: invalid configuration");
            return Err(DialogError::internal_error("Invalid configuration", None));
        }

        let local_address = config.local_address();
        info!(
            "Creating UnifiedDialogManager with shared global events in {:?} mode at {}",
            Self::mode_name(&config),
            local_address
        );

        let mut core = DialogManager::with_shared_global_events_and_index_capacity_and_config(
            transaction_manager,
            transaction_events,
            local_address,
            config.dialog_config().max_dialogs.unwrap_or(10_000),
            Some(config.clone()),
        )
        .await?;

        core.set_config(config.clone());
        let initial_invite_install_capacity = core.invite_failover_active_plan_capacity;

        Ok(Self {
            core,
            config,
            stats: Arc::new(tokio::sync::RwLock::new(ManagerStats::default())),
            initial_invite_installs: Arc::new(dashmap::DashMap::new()),
            initial_invite_install_slots: Arc::new(tokio::sync::Semaphore::new(
                initial_invite_install_capacity,
            )),
            initial_invite_dispatch_tasks: Arc::new(dashmap::DashMap::new()),
            initial_invite_dispatch_gate: Arc::new(std::sync::Mutex::new(true)),
            initial_invite_cleanup_tasks: Arc::new(dashmap::DashMap::new()),
            initial_invite_cleanup_gate: Arc::new(std::sync::Mutex::new(true)),
            #[cfg(test)]
            initial_invite_dispatch_test_hook: Arc::new(AtomicU8::new(0)),
            #[cfg(test)]
            initial_invite_cleanup_test_hook: Arc::new(AtomicU8::new(0)),
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
                    "Client mode active - from_uri_present: {}, auto_auth: {}",
                    client.from_uri.is_some(),
                    client.auto_auth
                );
            }
            DialogManagerConfig::Server(server) => {
                info!(
                    "Server mode active - domain_present: {}, auto_options: {}, auto_register: {}",
                    server.domain.is_some(),
                    server.auto_options_response,
                    server.auto_register_response
                );
            }
            DialogManagerConfig::Hybrid(hybrid) => {
                info!(
                    "Hybrid mode active - from_uri_present: {}, domain_present: {}, auto_auth: {}, auto_options: {}",
                    hybrid.from_uri.is_some(), hybrid.domain.is_some(), hybrid.auto_auth, hybrid.auto_options_response
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

        {
            let mut accepting = self
                .initial_invite_dispatch_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *accepting = false;
        }
        self.abort_and_join_initial_invite_dispatch_tasks().await?;
        {
            let mut accepting = self
                .initial_invite_cleanup_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *accepting = false;
        }
        let incomplete_cleanup = self
            .drain_abort_and_join_initial_invite_cleanup_tasks()
            .await?;
        let retained_wire_unknown = self
            .initial_invite_installs
            .iter()
            .filter(|record| record.phase.load(Ordering::Acquire) == INITIAL_INVITE_WIRE_UNKNOWN)
            .count();
        let incomplete_cleanup = incomplete_cleanup.max(retained_wire_unknown);
        if incomplete_cleanup > 0 {
            return Err(DialogError::InternalError {
                message: format!(
                    "initial INVITE protocol drain incomplete for {} wire-unknown owner(s); local ownership preserved",
                    incomplete_cleanup
                ),
                context: None,
            });
        }

        // Stop the core dialog manager
        self.core.stop().await?;
        self.initial_invite_installs.clear();

        info!("UnifiedDialogManager stopped successfully");
        Ok(())
    }

    async fn abort_and_join_initial_invite_dispatch_tasks(&self) -> DialogResult<()> {
        let tasks: Vec<(DialogId, InitialInviteDispatchTaskRecord)> = self
            .initial_invite_dispatch_tasks
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        for (_, task) in &tasks {
            task.abort.abort();
        }

        let mut stragglers = 0usize;
        for (dialog_id, task) in tasks {
            let mut completion = task.completion.clone();
            let completed = *completion.borrow()
                || tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    loop {
                        if completion.changed().await.is_err() || *completion.borrow() {
                            break;
                        }
                    }
                })
                .await
                .is_ok();
            if completed {
                self.initial_invite_dispatch_tasks
                    .remove_if(&dialog_id, |_, current| current.token == task.token);
            } else {
                stragglers = stragglers.saturating_add(1);
                warn!(
                    dialog_id = %dialog_id,
                    "Initial INVITE dispatch task did not stop within the join deadline"
                );
            }
        }

        if stragglers == 0 {
            Ok(())
        } else {
            Err(DialogError::InternalError {
                message: format!(
                    "{} initial INVITE dispatch task(s) did not stop before core shutdown",
                    stragglers
                ),
                context: None,
            })
        }
    }

    async fn drain_abort_and_join_initial_invite_cleanup_tasks(&self) -> DialogResult<usize> {
        let tasks: Vec<(DialogId, InitialInviteCleanupTask)> = self
            .initial_invite_cleanup_tasks
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();

        // Give already-owned cleanup drivers one bounded window to issue their
        // single CANCEL/BYE and observe a terminal response. Aborting first
        // could orphan a peer after a real wire-unknown INVITE.
        let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            let all_complete = tasks.iter().all(|(_, task)| {
                *task.completion.borrow() || task.completion.has_changed().is_err()
            });
            if all_complete || tokio::time::Instant::now() >= drain_deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        for (dialog_id, task) in &tasks {
            if *task.completion.borrow() || task.completion.has_changed().is_err() {
                self.initial_invite_cleanup_tasks
                    .remove_if(dialog_id, |_, current| current.token == task.token);
            }
        }

        let incomplete: Vec<_> = tasks
            .iter()
            .filter(|(_, task)| !*task.completion.borrow() && task.completion.has_changed().is_ok())
            .map(|(dialog_id, task)| (dialog_id.clone(), task.clone()))
            .collect();
        for (_, task) in &incomplete {
            task.abort.abort();
        }

        let mut stragglers = 0usize;
        for (dialog_id, task) in incomplete.iter().cloned() {
            let mut completion = task.completion.clone();
            let completed = *completion.borrow()
                || tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    loop {
                        if completion.changed().await.is_err() || *completion.borrow() {
                            break;
                        }
                    }
                })
                .await
                .is_ok();
            if completed {
                self.initial_invite_cleanup_tasks
                    .remove_if(&dialog_id, |_, current| current.token == task.token);
            } else {
                stragglers = stragglers.saturating_add(1);
                warn!(
                    dialog_id = %dialog_id,
                    "Initial INVITE cleanup task did not stop within the join deadline"
                );
            }
        }

        if stragglers == 0 {
            Ok(incomplete.len())
        } else {
            Err(DialogError::InternalError {
                message: format!(
                    "{} initial INVITE cleanup task(s) did not stop before core shutdown",
                    stragglers
                ),
                context: None,
            })
        }
    }

    // REMOVED: Channel-based methods - use GlobalEventCoordinator instead
    // - set_session_coordinator()
    // - set_dialog_event_sender()
    // - subscribe_to_dialog_events()

    // ========================================
    // CLIENT-MODE OPERATIONS
    // ========================================

    /// Validate and allocate an exact outbound initial-INVITE plan without
    /// installing state, publishing events, or touching the network.
    pub async fn plan_initial_invite(
        &self,
        pre_register_session_id: Option<String>,
        mut options: crate::api::unified::InviteRequestOptions,
    ) -> ApiResult<PlannedInitialInvite> {
        use rvoip_sip_core::types::header::HeaderName;

        if !self.config.supports_outgoing_calls() {
            return Err(ApiError::Configuration {
                message: "Outgoing calls not supported in Server mode".to_string(),
            });
        }
        crate::api::unified::validate_initial_invite_options(&options)?;

        if let Some(authorization) = options.precomputed_authorization.take() {
            options.extra_headers.insert(
                0,
                rvoip_sip_core::validation::validated_authorization_header(
                    HeaderName::Authorization,
                    authorization,
                )
                .map_err(|_| {
                    ApiError::protocol("INVITE Authorization failed wire-safety validation")
                })?,
            );
        }

        let local_uri: Uri = options
            .from_uri
            .parse()
            .map_err(|_| ApiError::Configuration {
                message: "Invalid caller URI".to_string(),
            })?;
        let remote_uri: Uri = options
            .to_uri
            .parse()
            .map_err(|_| ApiError::Configuration {
                message: "Invalid target URI".to_string(),
            })?;
        let call_id = options
            .call_id
            .clone()
            .unwrap_or_else(|| format!("call-{}", Uuid::new_v4()));
        options.call_id = Some(call_id.clone());

        let mut dialog = Dialog::new_early(
            call_id.clone(),
            local_uri.clone(),
            remote_uri,
            None,
            None,
            true,
        );
        if let Some(service_route) = self
            .core
            .service_route_for_aor(&local_uri.to_string())
            .await
        {
            dialog.route_set = service_route;
        }

        let owner = InitialInviteOwner {
            dialog_id: dialog.id.clone(),
            call_id,
            session_id: pre_register_session_id,
            token: Uuid::new_v4(),
        };
        Ok(PlannedInitialInvite {
            owner,
            dialog,
            options,
        })
    }

    /// Install the plan's exact dialog and optional session mapping locally.
    ///
    /// This operation is synchronous by design: once it returns, a response
    /// racing the subsequent dispatch can already resolve both mappings.
    pub fn install_initial_invite(
        &self,
        plan: PlannedInitialInvite,
    ) -> ApiResult<InstalledInitialInvite> {
        self.install_initial_invite_with_sink(plan, |_| Ok(()))
    }

    /// Install with a synchronous, non-cloneable lifecycle handoff.
    ///
    /// The `FnOnce` sink is the exact ownership linearization point for an
    /// upper lifecycle/resource registry. It receives the non-cloneable
    /// installed value by reference after exact admission is reserved, but
    /// before dialog/session mappings or network state are published. If the
    /// sink rejects the handoff, the admission record and permit are removed
    /// without publishing mappings. Dropping the returned value before
    /// dispatch also compensates the never-sent installation synchronously.
    pub fn install_initial_invite_with_sink<F>(
        &self,
        plan: PlannedInitialInvite,
        sink: F,
    ) -> ApiResult<InstalledInitialInvite>
    where
        F: FnOnce(&InstalledInitialInvite) -> ApiResult<()>,
    {
        use dashmap::mapref::entry::Entry;

        let accepting = self
            .initial_invite_dispatch_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !*accepting || !self.core.is_accepting_work() {
            return Err(ApiError::Dialog {
                message: "Initial INVITE admission is closed".to_string(),
            });
        }

        let PlannedInitialInvite {
            owner,
            dialog,
            options,
        } = plan;
        let capacity_permit = self
            .initial_invite_install_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::Dialog {
                message: "Initial INVITE admission capacity exhausted".to_string(),
            })?;

        let record = Arc::new(InitialInviteInstallRecord {
            token: owner.token,
            phase: Arc::new(AtomicU8::new(INITIAL_INVITE_INSTALLING)),
            _capacity_permit: capacity_permit,
        });
        match self.initial_invite_installs.entry(owner.dialog_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(record.clone());
            }
            Entry::Occupied(_) => {
                return Err(ApiError::Dialog {
                    message: "Initial INVITE dialog owner already installed".to_string(),
                });
            }
        }

        let installed = InstalledInitialInvite {
            lease: InitialInviteInstallLease {
                manager: self.clone(),
                owner: owner.clone(),
                armed: true,
            },
            owner,
            options,
        };

        if self.core.dialogs.contains_key(&installed.owner.dialog_id) {
            return Err(ApiError::Dialog {
                message: "Initial INVITE dialog already exists".to_string(),
            });
        }
        if installed
            .owner
            .session_id
            .as_ref()
            .is_some_and(|session_id| self.core.session_to_dialog.contains_key(session_id))
        {
            return Err(ApiError::Dialog {
                message: "Session already owns a dialog".to_string(),
            });
        }

        // No dialog/session mapping exists before this callback. A lifecycle
        // registry can therefore record the exact owner before cancellation
        // could strand lower-layer resources.
        sink(&installed)?;

        match self.core.dialogs.entry(installed.owner.dialog_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(dialog);
            }
            Entry::Occupied(_) => {
                return Err(ApiError::Dialog {
                    message: "Initial INVITE dialog already exists".to_string(),
                });
            }
        }

        if let Some(session_id) = installed.owner.session_id.as_ref() {
            match self.core.session_to_dialog.entry(session_id.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(installed.owner.dialog_id.clone());
                }
                Entry::Occupied(_) => {
                    return Err(ApiError::Dialog {
                        message: "Session already owns a dialog".to_string(),
                    });
                }
            }
            self.core
                .dialog_to_session
                .insert(installed.owner.dialog_id.clone(), session_id.clone());
        }

        record
            .phase
            .store(INITIAL_INVITE_INSTALLED, Ordering::Release);
        Ok(installed)
    }

    /// Start a retained dispatch of an installed initial INVITE.
    ///
    /// The installed value is consumed, preventing duplicate dispatch. The
    /// task is retained in the manager registry, remains owned if this result
    /// handle is dropped, and is joined during manager shutdown.
    pub fn dispatch_initial_invite(
        &self,
        installed: InstalledInitialInvite,
    ) -> InitialInviteDispatch {
        let owner = installed.owner.clone();
        let runtime = tokio::runtime::Handle::try_current();
        let task = match runtime {
            Err(_) => InitialInviteDispatchTask::Ready(Some(InitialInviteDispatchCompletion {
                owner: owner.clone(),
                wire_outcome: InitialInviteWireOutcome::ZeroWire,
                transaction_id: None,
                error: Some(ApiError::internal(
                    "Initial INVITE dispatch requires a Tokio runtime",
                )),
            })),
            Ok(runtime) => {
                let accepting = self
                    .initial_invite_dispatch_gate
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if !*accepting
                    || self
                        .initial_invite_dispatch_tasks
                        .contains_key(owner.dialog_id())
                {
                    InitialInviteDispatchTask::Ready(Some(InitialInviteDispatchCompletion {
                        owner: owner.clone(),
                        wire_outcome: InitialInviteWireOutcome::ZeroWire,
                        transaction_id: None,
                        error: Some(ApiError::Dialog {
                            message: "Initial INVITE dispatch admission is closed".to_string(),
                        }),
                    }))
                } else {
                    let task_token = Uuid::new_v4();
                    let (start_tx, start_rx) = tokio::sync::oneshot::channel();
                    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
                    let (completion_tx, completion_rx) = tokio::sync::watch::channel(false);
                    let manager = self.clone();
                    let task_owner = owner.clone();
                    let task = runtime.spawn(async move {
                        let mut execution = InitialInviteDispatchExecution {
                            manager: manager.clone(),
                            owner: task_owner,
                            task_token,
                            completion: Some(completion_tx),
                            normal_completion: false,
                        };
                        if start_rx.await.is_err() {
                            return;
                        }
                        let completion = manager.dispatch_initial_invite_inner(installed).await;
                        if completion.wire_outcome() == InitialInviteWireOutcome::Unknown {
                            manager.supervise_wire_unknown_cleanup(completion.owner().clone());
                        }
                        execution.normal_completion = true;
                        drop(execution);
                        let _ = result_tx.send(completion);
                    });
                    self.initial_invite_dispatch_tasks.insert(
                        owner.dialog_id().clone(),
                        InitialInviteDispatchTaskRecord {
                            token: task_token,
                            abort: task.abort_handle(),
                            completion: completion_rx,
                        },
                    );
                    drop(task);
                    let _ = start_tx.send(());
                    InitialInviteDispatchTask::Running(result_rx)
                }
            }
        };
        InitialInviteDispatch {
            manager: self.clone(),
            owner,
            task,
        }
    }

    async fn dispatch_initial_invite_inner(
        &self,
        installed: InstalledInitialInvite,
    ) -> InitialInviteDispatchCompletion {
        let InstalledInitialInvite {
            owner,
            options,
            mut lease,
        } = installed;
        let record = self
            .initial_invite_installs
            .get(&owner.dialog_id)
            .map(|record| record.value().clone());
        let admitted = record.as_ref().is_some_and(|record| {
            record.token == owner.token
                && record
                    .phase
                    .compare_exchange(
                        INITIAL_INVITE_INSTALLED,
                        INITIAL_INVITE_DISPATCHING,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
        });
        if !admitted {
            return InitialInviteDispatchCompletion {
                owner,
                wire_outcome: InitialInviteWireOutcome::ZeroWire,
                transaction_id: None,
                error: Some(ApiError::Dialog {
                    message: "Initial INVITE installation is not dispatchable".to_string(),
                }),
            };
        }
        lease.disarm();

        #[cfg(test)]
        match self
            .initial_invite_dispatch_test_hook
            .swap(0, Ordering::AcqRel)
        {
            1 => panic!("injected initial INVITE dispatch panic after admission"),
            2 => std::future::pending::<()>().await,
            _ => {}
        }

        {
            let mut stats = self.stats.write().await;
            stats.outgoing_calls = stats.outgoing_calls.saturating_add(1);
            stats.active_dialogs = stats.active_dialogs.saturating_add(1);
        }
        self.core
            .emit_dialog_event(DialogEvent::Created {
                dialog_id: owner.dialog_id.clone(),
            })
            .await;

        let body = options.sdp.map(bytes::Bytes::from);
        let send_result = self
            .core
            .send_initial_invite_with_wire_receipt(
                &owner.dialog_id,
                body,
                options.extra_headers,
                options.from_display,
                options.contact_uri,
                options.outbound_proxy_uri,
                options.supported_100rel,
            )
            .await;

        match send_result {
            Ok(transaction_id) => {
                if let Some(record) = record {
                    let _ = record.phase.compare_exchange(
                        INITIAL_INVITE_DISPATCHING,
                        INITIAL_INVITE_SENT,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
                InitialInviteDispatchCompletion {
                    owner,
                    wire_outcome: InitialInviteWireOutcome::Sent,
                    transaction_id: Some(transaction_id),
                    error: None,
                }
            }
            Err(failure) => {
                if failure.wire_was_attempted() {
                    self.mark_initial_invite_wire_unknown(&owner);
                    InitialInviteDispatchCompletion {
                        owner,
                        wire_outcome: InitialInviteWireOutcome::Unknown,
                        transaction_id: None,
                        error: Some(ApiError::from(failure.into_dialog_error())),
                    }
                } else {
                    let error = ApiError::from(failure.into_dialog_error());
                    self.rollback_zero_wire_dispatch(&owner).await;
                    InitialInviteDispatchCompletion {
                        owner,
                        wire_outcome: InitialInviteWireOutcome::ZeroWire,
                        transaction_id: None,
                        error: Some(error),
                    }
                }
            }
        }
    }

    fn mark_initial_invite_wire_unknown(&self, owner: &InitialInviteOwner) {
        if let Some(record) = self.initial_invite_installs.get(&owner.dialog_id) {
            if record.token == owner.token {
                let _ = record.phase.compare_exchange(
                    INITIAL_INVITE_DISPATCHING,
                    INITIAL_INVITE_WIRE_UNKNOWN,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }
        }
    }

    fn compensate_dropped_initial_invite_install(&self, owner: &InitialInviteOwner) -> bool {
        let Some(record) = self
            .initial_invite_installs
            .get(&owner.dialog_id)
            .map(|entry| entry.value().clone())
        else {
            return false;
        };
        if record.token != owner.token {
            return false;
        }
        loop {
            let phase = record.phase.load(Ordering::Acquire);
            if phase != INITIAL_INVITE_INSTALLING && phase != INITIAL_INVITE_INSTALLED {
                return false;
            }
            if record
                .phase
                .compare_exchange(
                    phase,
                    INITIAL_INVITE_RELEASING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                break;
            }
        }
        if self
            .initial_invite_installs
            .remove_if(&owner.dialog_id, |_, current| current.token == owner.token)
            .is_none()
        {
            return false;
        }
        if let Some(session_id) = owner.session_id.as_ref() {
            self.core
                .session_to_dialog
                .remove_if(session_id, |_, mapped| mapped == &owner.dialog_id);
        }
        self.core.cleanup_dialog_storage(&owner.dialog_id);
        true
    }

    async fn release_initial_invite_owner_from_phase(
        &self,
        owner: &InitialInviteOwner,
        expected_phase: u8,
    ) -> bool {
        let Some(record) = self
            .initial_invite_installs
            .get(&owner.dialog_id)
            .map(|entry| entry.value().clone())
        else {
            return false;
        };
        if record.token != owner.token {
            return false;
        }
        if record
            .phase
            .compare_exchange(
                expected_phase,
                INITIAL_INVITE_RELEASING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }
        if self
            .initial_invite_installs
            .remove_if(&owner.dialog_id, |_, current| current.token == owner.token)
            .is_none()
        {
            return false;
        }
        self.core
            .cleanup_dialog_storage_and_transactions(&owner.dialog_id)
            .await;
        true
    }

    /// Roll back a locally installed, never-dispatched INVITE by exact owner.
    ///
    /// Dispatching, Sent, and Unknown owners are refused because they require
    /// protocol teardown (CANCEL or BYE) before local release.
    pub async fn compensate_initial_invite(&self, owner: &InitialInviteOwner) -> bool {
        self.release_initial_invite_owner_from_phase(owner, INITIAL_INVITE_INSTALLED)
            .await
    }

    /// Return whether this exact initial-INVITE owner is still retained.
    ///
    /// This is intentionally owner-qualified: a delayed upper-layer release
    /// must never infer ownership from a reusable application session ID or
    /// even from a Dialog-ID alone.
    pub fn initial_invite_owner_is_retained(&self, owner: &InitialInviteOwner) -> bool {
        self.initial_invite_installs
            .get(owner.dialog_id())
            .is_some_and(|record| record.token == owner.token)
    }

    /// Hand a sent initial INVITE to the manager's protocol-teardown
    /// supervisor.
    ///
    /// The supervisor applies the dialog-state-specific CANCEL/BYE policy and
    /// its at-most-once ambiguous-send rules. Repeated calls for the same exact
    /// owner are harmless; stale owners are refused.
    pub fn supervise_initial_invite_teardown(&self, owner: &InitialInviteOwner) -> bool {
        loop {
            let Some(record) = self
                .initial_invite_installs
                .get(owner.dialog_id())
                .map(|entry| entry.value().clone())
            else {
                return false;
            };
            if record.token != owner.token {
                return false;
            }

            match record.phase.load(Ordering::Acquire) {
                INITIAL_INVITE_SENT => {
                    if record
                        .phase
                        .compare_exchange(
                            INITIAL_INVITE_SENT,
                            INITIAL_INVITE_WIRE_UNKNOWN,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        continue;
                    }
                    self.supervise_wire_unknown_cleanup(owner.clone());
                    return true;
                }
                INITIAL_INVITE_WIRE_UNKNOWN => {
                    self.supervise_wire_unknown_cleanup(owner.clone());
                    return true;
                }
                _ => return false,
            }
        }
    }

    /// Retire an exact sent/uncertain initial-INVITE owner after the upper
    /// layer has already dispatched the legal protocol teardown or observed a
    /// terminal dialog.
    ///
    /// This does not synthesize signaling. Callers that have not already sent
    /// CANCEL/BYE must use [`Self::supervise_initial_invite_teardown`].
    pub async fn finish_initial_invite_teardown(&self, owner: &InitialInviteOwner) -> bool {
        let released = self
            .release_initial_invite_owner_from_phase(owner, INITIAL_INVITE_SENT)
            .await
            || self
                .release_initial_invite_owner_from_phase(owner, INITIAL_INVITE_WIRE_UNKNOWN)
                .await;
        if released {
            let mut stats = self.stats.write().await;
            stats.active_dialogs = stats.active_dialogs.saturating_sub(1);
        }
        released
    }

    async fn rollback_zero_wire_dispatch(&self, owner: &InitialInviteOwner) {
        if self
            .release_initial_invite_owner_from_phase(owner, INITIAL_INVITE_DISPATCHING)
            .await
        {
            let mut stats = self.stats.write().await;
            stats.active_dialogs = stats.active_dialogs.saturating_sub(1);
            stats.failed_calls = stats.failed_calls.saturating_add(1);
        }
    }

    fn finish_legacy_sent_owner_handoff(&self, owner: &InitialInviteOwner) {
        let Some(record) = self
            .initial_invite_installs
            .get(owner.dialog_id())
            .map(|entry| entry.value().clone())
        else {
            return;
        };
        if record.token != owner.token
            || record
                .phase
                .compare_exchange(
                    INITIAL_INVITE_SENT,
                    INITIAL_INVITE_RELEASING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
        {
            return;
        }
        self.initial_invite_installs
            .remove_if(owner.dialog_id(), |_, current| current.token == owner.token);
    }

    fn supervise_wire_unknown_cleanup(&self, owner: InitialInviteOwner) {
        use dashmap::mapref::entry::Entry;

        let accepting = self
            .initial_invite_cleanup_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !*accepting {
            return;
        }
        let task_token = Uuid::new_v4();
        let manager = self.clone();
        let task_dialog_id = owner.dialog_id.clone();
        let map_dialog_id = task_dialog_id.clone();
        let (start_tx, start_rx) = tokio::sync::oneshot::channel();
        let (completion_tx, completion_rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move {
            let _completion = InitialInviteCleanupCompletion {
                sender: Some(completion_tx),
            };
            if start_rx.await.is_err() {
                return;
            }
            manager.run_wire_unknown_cleanup(owner).await;
            manager
                .initial_invite_cleanup_tasks
                .remove_if(&task_dialog_id, |_, task| task.token == task_token);
        });
        let abort = task.abort_handle();
        drop(task);

        match self.initial_invite_cleanup_tasks.entry(map_dialog_id) {
            Entry::Vacant(entry) => {
                entry.insert(InitialInviteCleanupTask {
                    token: task_token,
                    abort,
                    completion: completion_rx,
                });
                let _ = start_tx.send(());
            }
            Entry::Occupied(_) => {
                abort.abort();
            }
        }
    }

    async fn run_wire_unknown_cleanup(&self, owner: InitialInviteOwner) {
        #[cfg(test)]
        if self
            .initial_invite_cleanup_test_hook
            .compare_exchange(2, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let mut retry_delay = std::time::Duration::from_millis(10);
        let mut cancel_attempted = false;
        let mut bye_attempted = false;
        loop {
            let still_owned = self
                .initial_invite_installs
                .get(owner.dialog_id())
                .is_some_and(|record| {
                    record.token == owner.token
                        && record.phase.load(Ordering::Acquire) == INITIAL_INVITE_WIRE_UNKNOWN
                });
            if !still_owned {
                return;
            }
            if self
                .try_protocol_teardown_for_wire_unknown(
                    &owner,
                    &mut cancel_attempted,
                    &mut bye_attempted,
                )
                .await
            {
                self.core
                    .complete_wire_unknown_invite_for_dialog(owner.dialog_id())
                    .await;
                if self
                    .release_initial_invite_owner_from_phase(&owner, INITIAL_INVITE_WIRE_UNKNOWN)
                    .await
                {
                    let mut stats = self.stats.write().await;
                    stats.active_dialogs = stats.active_dialogs.saturating_sub(1);
                    stats.failed_calls = stats.failed_calls.saturating_add(1);
                }
                return;
            }
            #[cfg(test)]
            if self
                .initial_invite_cleanup_test_hook
                .swap(0, Ordering::AcqRel)
                == 1
            {
                std::future::pending::<()>().await;
            }
            tokio::time::sleep(retry_delay).await;
            retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(1));
        }
    }

    async fn try_protocol_teardown_for_wire_unknown(
        &self,
        owner: &InitialInviteOwner,
        cancel_attempted: &mut bool,
        bye_attempted: &mut bool,
    ) -> bool {
        let state = match self.core.get_dialog_state(owner.dialog_id()) {
            Ok(state) => state,
            Err(_) => {
                return self
                    .core
                    .wire_unknown_invite_has_terminal_failure(owner.dialog_id())
                    .await;
            }
        };
        match state {
            DialogState::Initial | DialogState::Early => {
                if self
                    .core
                    .wire_unknown_invite_has_terminal_failure(owner.dialog_id())
                    .await
                {
                    return true;
                }
                // `send_cancel` does not expose a wire receipt on error. Once
                // invoked, an Err is therefore ambiguous and must not create a
                // second CANCEL. Keep the exact owner/capacity charged while
                // watching for a terminal or confirmed dialog transition.
                if *cancel_attempted {
                    return false;
                }
                *cancel_attempted = true;
                let _ = self.send_cancel(owner.dialog_id()).await;
                // A 200 response to CANCEL only settles the CANCEL transaction;
                // it does not settle the original INVITE. The next supervisor
                // pass verifies the retained INVITE's terminal response.
                false
            }
            DialogState::Confirmed | DialogState::Recovering => {
                // Apply the same at-most-once rule to BYE: a lower-layer Err
                // can occur after bytes crossed the transport boundary.
                if *bye_attempted {
                    return false;
                }
                *bye_attempted = true;
                let transaction_id = match self
                    .core
                    .send_request(owner.dialog_id(), Method::Bye, None)
                    .await
                {
                    Ok(transaction_id) => transaction_id,
                    Err(_) => return false,
                };
                let timeout = self
                    .core
                    .transaction_manager()
                    .timer_settings()
                    .transaction_timeout;
                match self
                    .core
                    .transaction_manager()
                    .wait_for_final_response(&transaction_id, timeout)
                    .await
                {
                    Ok(Some(response)) => {
                        response.status().is_success()
                            || response.status() == StatusCode::CallOrTransactionDoesNotExist
                    }
                    Ok(None) | Err(_) => false,
                }
            }
            DialogState::Terminated => {
                self.core
                    .wire_unknown_invite_has_terminal_failure(owner.dialog_id())
                    .await
            }
        }
    }

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
            None,
            None,
            None,
            false,
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
        self.make_call_inner(
            from_uri,
            to_uri,
            sdp_offer,
            call_id,
            None,
            Vec::new(),
            None,
            None,
            None,
            false,
        )
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

        let from: Uri = from_uri.parse().map_err(|_error| ApiError::Configuration {
            message: "Invalid caller URI".to_string(),
        })?;
        let target: Uri = target_uri
            .parse()
            .map_err(|_error| ApiError::Configuration {
                message: "Invalid target URI".to_string(),
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
            .map_err(|_error| ApiError::protocol("Failed to build SUBSCRIBE"))?;
            let destination = crate::dialog::dialog_utils::resolve_uri_to_socketaddr(
                &crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request),
            )
            .await
            .ok_or_else(|| ApiError::protocol("Failed to resolve SUBSCRIBE target URI"))?;
            (destination, request)
        };

        let request_key = crate::manager::core::outbound_request_key(&request);
        let next_hop =
            crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request);
        let selected_transport = self
            .core
            .transaction_manager()
            .get_best_transport_for_uri(&next_hop);
        let transaction_id = self
            .core
            .transaction_manager()
            .create_non_invite_client_transaction(request, destination)
            .await
            .map_err(|_error| ApiError::internal("Failed to create SUBSCRIBE transaction"))?;
        self.core
            .link_transaction_to_dialog_indexed(&transaction_id, &dialog_id);
        self.core
            .transaction_manager()
            .send_request(&transaction_id)
            .await
            .map_err(|_error| ApiError::internal("Failed to send SUBSCRIBE"))?;
        self.core.record_outbound_transport_context(
            &transaction_id,
            request_key,
            selected_transport,
            destination,
        );

        let response = self
            .core
            .transaction_manager()
            .wait_for_final_response(&transaction_id, std::time::Duration::from_secs(30))
            .await
            .map_err(|_error| ApiError::internal("Failed to wait for SUBSCRIBE response"))?
            .ok_or_else(|| ApiError::network("SUBSCRIBE timed out".to_string()))?;

        if !(200..=299).contains(&response.status_code()) {
            return Err(ApiError::protocol(format!(
                "SUBSCRIBE failed with status {}",
                response.status_code()
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
            .map_err(|_error| ApiError::protocol("Failed to build SUBSCRIBE refresh"))?;
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
                request.headers.push(
                    rvoip_sip_core::validation::validated_authorization_header(
                        HeaderName::Authorization,
                        auth,
                    )
                    .map_err(|_| {
                        ApiError::protocol("SUBSCRIBE Authorization failed wire-safety validation")
                    })?,
                );
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
            .ok_or_else(|| ApiError::protocol("Failed to resolve SUBSCRIBE refresh URI"))?;
            (destination, request)
        };

        let request_key = crate::manager::core::outbound_request_key(&request);
        let next_hop =
            crate::transaction::transport::multiplexed::next_hop_uri_for_request(&request);
        let selected_transport = self
            .core
            .transaction_manager()
            .get_best_transport_for_uri(&next_hop);
        let transaction_id = self
            .core
            .transaction_manager()
            .create_non_invite_client_transaction(request, destination)
            .await
            .map_err(|_error| {
                ApiError::internal("Failed to create SUBSCRIBE refresh transaction")
            })?;
        self.core
            .link_transaction_to_dialog_indexed(&transaction_id, dialog_id);
        self.core
            .transaction_manager()
            .send_request(&transaction_id)
            .await
            .map_err(|_error| ApiError::internal("Failed to send SUBSCRIBE refresh"))?;
        self.core.record_outbound_transport_context(
            &transaction_id,
            request_key,
            selected_transport,
            destination,
        );

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
        self.make_call_inner(
            from_uri,
            to_uri,
            sdp_offer,
            None,
            None,
            extra_headers,
            None,
            None,
            None,
            false,
        )
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
            None,
            None,
            None,
            false,
        )
        .await
    }

    /// SIP_API_DESIGN_2 Phase B — structured initial-INVITE send. Folds the
    /// pre-computed `Authorization` into the appended headers and threads the
    /// `From` display name + `Contact` override structurally into the builder.
    pub async fn send_invite_with_options(
        &self,
        pre_register_session_id: Option<String>,
        opts: crate::api::unified::InviteRequestOptions,
    ) -> ApiResult<CallHandle> {
        use rvoip_sip_core::types::header::HeaderName;

        crate::api::unified::validate_initial_invite_options(&opts)?;

        let mut extra_headers = opts.extra_headers;
        if let Some(auth) = opts.precomputed_authorization {
            // Pre-emptive auth on the initial INVITE rides as an appended
            // Authorization header — same shape as the digest stamp the
            // auth-retry path emits.
            extra_headers.insert(
                0,
                rvoip_sip_core::validation::validated_authorization_header(
                    HeaderName::Authorization,
                    auth,
                )
                .map_err(|_| {
                    ApiError::protocol("INVITE Authorization failed wire-safety validation")
                })?,
            );
        }

        self.make_call_inner(
            &opts.from_uri,
            &opts.to_uri,
            opts.sdp,
            opts.call_id,
            pre_register_session_id,
            extra_headers,
            opts.from_display,
            opts.contact_uri,
            opts.outbound_proxy_uri,
            opts.supported_100rel,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn make_call_inner(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
        call_id: Option<String>,
        pre_register_session_id: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
        from_display: Option<String>,
        contact_override: Option<String>,
        outbound_proxy_uri: Option<Uri>,
        supported_100rel: bool,
    ) -> ApiResult<CallHandle> {
        info!("Making outgoing call with caller and target URIs present");
        let plan = self
            .plan_initial_invite(
                pre_register_session_id,
                crate::api::unified::InviteRequestOptions {
                    from_uri: from_uri.to_string(),
                    to_uri: to_uri.to_string(),
                    sdp: sdp_offer,
                    call_id,
                    from_display,
                    contact_uri: contact_override,
                    precomputed_authorization: None,
                    outbound_proxy_uri,
                    supported_100rel,
                    extra_headers,
                },
            )
            .await?;
        let installed = self.install_initial_invite(plan)?;
        let completion = self.dispatch_initial_invite(installed).wait().await;
        let (owner, _transaction_key) = match completion.into_result() {
            Ok(success) => success,
            Err(error) => {
                if error.wire_outcome() == InitialInviteWireOutcome::Unknown {
                    self.supervise_wire_unknown_cleanup(error.owner().clone());
                }
                return Err(error.into_api_error());
            }
        };
        let dialog_id = owner.dialog_id().clone();
        self.finish_legacy_sent_owner_handoff(&owner);
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

        debug!("Creating outgoing dialog with local and remote URIs present");

        // Parse URIs
        let from_uri: Uri = from_uri.parse().map_err(|_error| {
            error!("Failed to parse caller URI for dialog creation");
            ApiError::Configuration {
                message: "Invalid caller URI".to_string(),
            }
        })?;
        let to_uri: Uri = to_uri.parse().map_err(|_error| {
            error!("Failed to parse target URI for dialog creation");
            ApiError::Configuration {
                message: "Invalid target URI".to_string(),
            }
        })?;

        // Create outgoing dialog
        let dialog_id = self
            .core
            .create_outgoing_dialog(from_uri, to_uri, None)
            .await
            .map_err(|_error| {
                error!("Failed to create outgoing dialog");
                ApiError::Dialog {
                    message: "Outgoing dialog creation failed".to_string(),
                }
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
            .map_err(|_error| {
                error!("Failed to process incoming INVITE from {}", source);
                ApiError::Dialog {
                    message: "Incoming INVITE processing failed".to_string(),
                }
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
        let method_label = method_class(&method);
        debug!("Sending {} request in dialog {}", method_label, dialog_id);

        self.core
            .send_request(dialog_id, method, body)
            .await
            .map_err(|e| {
                // Log SIP protocol validation errors as WARN (not ERROR) since they're often expected
                if e.to_string().contains("requires remote tag")
                    || e.to_string().contains("protocol error")
                {
                    warn!(
                        "SIP protocol validation failed for {} in dialog {}",
                        method_label, dialog_id
                    );
                } else {
                    error!(
                        "Failed to send {} request in dialog {}",
                        method_label, dialog_id
                    );
                }
                if e.to_string().contains("requires remote tag")
                    || e.to_string().contains("protocol error")
                {
                    ApiError::Protocol {
                        message: "SIP request validation failed".to_string(),
                    }
                } else {
                    ApiError::Dialog {
                        message: "SIP request send failed".to_string(),
                    }
                }
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
        debug!("Sending response for transaction");

        self.core
            .send_response(transaction_id, response)
            .await
            .map_err(|_error| {
                error!("Failed to send response for transaction");
                ApiError::Dialog {
                    message: "SIP response send failed".to_string(),
                }
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
            "Building response for transaction with status {}",
            status_code
        );

        // Get the original request from the transaction manager to copy required headers
        let original_request = self
            .core
            .transaction_manager()
            .original_request(transaction_id)
            .await
            .map_err(|_error| ApiError::Internal {
                message: "Failed to get original request".to_string(),
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
            "Successfully built response for transaction using proper RFC 3261 compliant headers"
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
        debug!("Sending status response {} for transaction", status_code);

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
            "Sending NOTIFY with event_present={} subscription_state_present={}",
            !event.is_empty(),
            subscription_state.is_some()
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
        self.core
            .send_prack(dialog_id, rseq)
            .await
            .map_err(|_error| {
                error!(
                    "Failed to send PRACK for dialog {} (RSeq={})",
                    dialog_id, rseq
                );
                ApiError::Dialog {
                    message: "PRACK send failed".to_string(),
                }
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
    #[allow(clippy::too_many_arguments)]
    pub async fn send_invite_with_auth(
        &self,
        dialog_id: &DialogId,
        sdp: Option<String>,
        auth_header_name: &str,
        auth_header_value: String,
        extras: Vec<rvoip_sip_core::types::TypedHeader>,
        from_display: Option<String>,
        contact_override: Option<String>,
    ) -> ApiResult<TransactionKey> {
        let body = sdp.map(bytes::Bytes::from);
        self.core
            .send_invite_with_auth(
                dialog_id,
                body,
                auth_header_name,
                auth_header_value,
                extras,
                from_display,
                contact_override,
            )
            .await
            .map_err(ApiError::from)
    }

    /// Authenticated initial-INVITE retry with accumulated origin/proxy
    /// credentials and the original structural routing/body policy.
    pub async fn send_invite_with_auth_options(
        &self,
        dialog_id: &DialogId,
        opts: crate::api::unified::InviteAuthRetryOptions,
    ) -> ApiResult<TransactionKey> {
        let body = opts.sdp.map(bytes::Bytes::from);
        self.core
            .send_invite_with_auth_options(
                dialog_id,
                body,
                opts.authorization_headers,
                opts.extra_headers,
                opts.from_display,
                opts.contact_uri,
                opts.outbound_proxy_uri,
                opts.supported_100rel,
            )
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

    /// Structural 422 retry retaining the original INVITE options and any
    /// accumulated proxy/origin credentials.
    pub async fn send_invite_with_session_timer_options(
        &self,
        dialog_id: &DialogId,
        opts: crate::api::unified::InviteAuthRetryOptions,
        session_secs: u32,
        min_se: u32,
    ) -> ApiResult<TransactionKey> {
        self.core
            .send_invite_with_session_timer_options(dialog_id, opts, session_secs, min_se)
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
            .map_err(|_error| {
                error!(
                    "Failed to cancel INVITE transaction for dialog {}",
                    dialog_id
                );
                ApiError::Dialog {
                    message: "INVITE cancellation failed".to_string(),
                }
            })?;

        info!("Successfully sent CANCEL for dialog {}", dialog_id);
        Ok(cancel_tx_id)
    }

    // ========================================
    // DIALOG MANAGEMENT (ALL MODES)
    // ========================================

    /// Get information about a dialog
    pub async fn get_dialog_info(&self, dialog_id: &DialogId) -> ApiResult<Dialog> {
        self.core.get_dialog(dialog_id).map_err(|e| {
            warn!("Failed to get dialog info for {}", dialog_id);
            ApiError::from(e)
        })
    }

    /// Get the current state of a dialog
    pub async fn get_dialog_state(&self, dialog_id: &DialogId) -> ApiResult<DialogState> {
        self.core.get_dialog_state(dialog_id).map_err(|e| {
            warn!("Failed to get dialog state for {}", dialog_id);
            ApiError::from(e)
        })
    }

    /// Terminate a dialog
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> ApiResult<()> {
        info!("Terminating dialog {}", dialog_id);
        self.core.terminate_dialog(dialog_id).await.map_err(|e| {
            error!("Failed to terminate dialog {}", dialog_id);
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
            .map_err(|_error| {
                error!(
                    "Failed to send ACK for 2xx response for dialog {}",
                    dialog_id
                );
                ApiError::Dialog {
                    message: "ACK send failed".to_string(),
                }
            })
    }
}

#[cfg(test)]
mod staged_initial_invite_tests {
    use super::*;
    use rvoip_sip_transport::error::Result as TransportResult;
    use rvoip_sip_transport::{Transport, TransportEvent};
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    #[derive(Debug)]
    struct CountingTransport {
        addr: SocketAddr,
        sends: AtomicUsize,
        cancel_sends: AtomicUsize,
        failures_remaining: AtomicUsize,
        closed: AtomicBool,
    }

    impl CountingTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                addr: "127.0.0.1:5060".parse().expect("test address"),
                sends: AtomicUsize::new(0),
                cancel_sends: AtomicUsize::new(0),
                failures_remaining: AtomicUsize::new(0),
                closed: AtomicBool::new(false),
            })
        }

        fn sends(&self) -> usize {
            self.sends.load(Ordering::SeqCst)
        }

        fn fail_next_sends(&self, count: usize) {
            self.failures_remaining.store(count, Ordering::SeqCst);
        }

        fn cancel_sends(&self) -> usize {
            self.cancel_sends.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl Transport for CountingTransport {
        fn local_addr(&self) -> TransportResult<SocketAddr> {
            Ok(self.addr)
        }

        async fn send_message(
            &self,
            message: rvoip_sip_core::Message,
            _destination: SocketAddr,
        ) -> TransportResult<()> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            if matches!(
                &message,
                rvoip_sip_core::Message::Request(request) if request.method() == Method::Cancel
            ) {
                self.cancel_sends.fetch_add(1, Ordering::SeqCst);
            }
            if self
                .failures_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    (remaining > 0).then(|| remaining - 1)
                })
                .is_ok()
            {
                return Err(rvoip_sip_transport::error::Error::TransportClosed);
            }
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

    async fn make_test_manager() -> (
        UnifiedDialogManager,
        Arc<CountingTransport>,
        mpsc::Receiver<DialogEvent>,
    ) {
        let transport = CountingTransport::new();
        let (_transport_tx, transport_rx) = mpsc::channel::<TransportEvent>(16);
        let mut timer_settings = crate::transaction::timer::TimerSettings::default();
        timer_settings.transaction_timeout = std::time::Duration::from_millis(100);
        let (transaction_manager, mut transaction_events) = TransactionManager::new_with_config(
            transport.clone(),
            transport_rx,
            Some(16),
            Some(timer_settings),
        )
        .await
        .expect("transaction manager");
        // Staged-INVITE tests exercise the dialog lifecycle directly rather
        // than installing the normal transaction-event dispatcher. Preserve
        // the primary channel as a live, drained authority: dropping it is a
        // fail-closed lifecycle event and must not be used as a test shortcut.
        tokio::spawn(async move { while transaction_events.recv().await.is_some() {} });
        let config = DialogManagerConfig::client(transport.addr)
            .with_from_uri("sip:alice@example.com")
            .build();
        let manager = UnifiedDialogManager::new(Arc::new(transaction_manager), config)
            .await
            .expect("dialog manager");
        let (event_tx, event_rx) = mpsc::channel(16);
        *manager.core.dialog_event_sender.write().await = Some(event_tx);
        (manager, transport, event_rx)
    }

    fn options(call_id: &str) -> crate::api::unified::InviteRequestOptions {
        crate::api::unified::InviteRequestOptions {
            from_uri: "sip:alice@example.com".to_string(),
            to_uri: "sip:bob@127.0.0.1:5099".to_string(),
            call_id: Some(call_id.to_string()),
            ..Default::default()
        }
    }

    async fn expect_incomplete_protocol_drain(manager: &UnifiedDialogManager) {
        let error = tokio::time::timeout(std::time::Duration::from_secs(5), manager.stop())
            .await
            .expect("manager stop deadline")
            .expect_err("wire-unknown stop must report incomplete protocol drain");
        match error {
            DialogError::InternalError { message, .. } => {
                assert!(message.contains("protocol drain incomplete"), "{message}");
                assert!(message.contains("local ownership preserved"), "{message}");
            }
            error => panic!("unexpected stop error: {error}"),
        }
    }

    #[tokio::test]
    async fn plan_and_install_have_separate_side_effect_boundaries() {
        let (manager, transport, mut events) = make_test_manager().await;
        let plan = manager
            .plan_initial_invite(Some("session-a".to_string()), options("call-a"))
            .await
            .expect("plan");

        assert_eq!(manager.core.dialog_count(), 0);
        assert_eq!(manager.initial_invite_installs.len(), 0);
        assert!(!manager.core.session_to_dialog.contains_key("session-a"));
        assert_eq!(transport.sends(), 0);
        assert!(events.try_recv().is_err());

        let dialog_id = plan.dialog_id().clone();
        let mut lifecycle_owner = None;
        let installed = manager
            .install_initial_invite_with_sink(plan, |candidate| {
                assert!(!manager.core.has_dialog(candidate.owner().dialog_id()));
                assert!(!manager.core.session_to_dialog.contains_key("session-a"));
                assert!(manager
                    .initial_invite_installs
                    .get(candidate.owner().dialog_id())
                    .is_some_and(|record| {
                        record.phase.load(Ordering::Acquire) == INITIAL_INVITE_INSTALLING
                    }));
                lifecycle_owner = Some(candidate.owner().clone());
                Ok(())
            })
            .expect("install");
        assert_eq!(lifecycle_owner.as_ref(), Some(installed.owner()));
        assert!(manager.core.has_dialog(&dialog_id));
        assert_eq!(
            manager
                .core
                .session_to_dialog
                .get("session-a")
                .map(|mapped| mapped.value().clone()),
            Some(dialog_id)
        );
        assert_eq!(transport.sends(), 0);
        assert!(events.try_recv().is_err());

        assert!(manager.compensate_initial_invite(installed.owner()).await);
    }

    #[tokio::test]
    async fn dropping_an_undispatched_install_compensates_exact_local_state() {
        let (manager, transport, mut events) = make_test_manager().await;
        let available_before = manager.initial_invite_install_slots.available_permits();
        let plan = manager
            .plan_initial_invite(Some("session-drop".to_string()), options("call-drop"))
            .await
            .expect("plan");
        let installed = manager.install_initial_invite(plan).expect("install");
        let dialog_id = installed.owner().dialog_id().clone();
        assert!(manager.core.has_dialog(&dialog_id));

        drop(installed);

        assert!(!manager.core.has_dialog(&dialog_id));
        assert!(!manager.core.session_to_dialog.contains_key("session-drop"));
        assert!(manager.initial_invite_installs.is_empty());
        assert_eq!(
            manager.initial_invite_install_slots.available_permits(),
            available_before
        );
        assert_eq!(transport.sends(), 0);
        assert!(events.try_recv().is_err());
        manager.stop().await.expect("stop manager");
    }

    #[tokio::test]
    async fn dispatch_sends_once_after_fast_response_indexes_exist() {
        let (manager, transport, _events) = make_test_manager().await;
        let plan = manager
            .plan_initial_invite(Some("session-fast".to_string()), options("call-fast"))
            .await
            .expect("plan");
        let installed = manager.install_initial_invite(plan).expect("install");
        let owner = installed.owner().clone();

        assert_eq!(
            manager.core.get_session_id(owner.dialog_id()).as_deref(),
            Some("session-fast")
        );
        assert_eq!(transport.sends(), 0);

        let completion = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            manager.dispatch_initial_invite(installed).wait(),
        )
        .await
        .expect("dispatch timeout");
        assert_eq!(completion.wire_outcome(), InitialInviteWireOutcome::Sent);
        assert!(completion.transaction_id().is_some());
        assert_eq!(transport.sends(), 1);

        assert!(!manager.compensate_initial_invite(&owner).await);
        assert!(manager.core.has_dialog(owner.dialog_id()));
        manager.stop().await.expect("stop manager");
    }

    #[tokio::test]
    async fn upper_protocol_teardown_retires_only_the_exact_sent_owner() {
        let (manager, transport, _events) = make_test_manager().await;
        let available_before = manager.initial_invite_install_slots.available_permits();
        let plan = manager
            .plan_initial_invite(
                Some("session-upper-teardown".to_string()),
                options("call-upper-teardown"),
            )
            .await
            .expect("plan");
        let installed = manager.install_initial_invite(plan).expect("install");
        let owner = installed.owner().clone();

        let completion = manager.dispatch_initial_invite(installed).wait().await;
        assert_eq!(completion.wire_outcome(), InitialInviteWireOutcome::Sent);
        assert_eq!(transport.sends(), 1);
        assert!(manager.initial_invite_owner_is_retained(&owner));

        assert!(manager.finish_initial_invite_teardown(&owner).await);
        assert!(!manager.initial_invite_owner_is_retained(&owner));
        assert!(!manager.core.has_dialog(owner.dialog_id()));
        assert!(!manager
            .core
            .session_to_dialog
            .contains_key("session-upper-teardown"));
        assert_eq!(
            manager.initial_invite_install_slots.available_permits(),
            available_before
        );
        assert!(!manager.finish_initial_invite_teardown(&owner).await);
        manager.stop().await.expect("stop manager");
    }

    #[tokio::test]
    async fn dropping_dispatch_handle_does_not_cancel_wire_work() {
        let (manager, transport, _events) = make_test_manager().await;
        let plan = manager
            .plan_initial_invite(None, options("call-detached"))
            .await
            .expect("plan");
        let installed = manager.install_initial_invite(plan).expect("install");
        let owner = installed.owner().clone();

        drop(manager.dispatch_initial_invite(installed));
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while transport.sends() == 0 {
                tokio::task::yield_now().await;
            }
            loop {
                let phase = manager
                    .initial_invite_installs
                    .get(owner.dialog_id())
                    .map(|record| record.phase.load(Ordering::Acquire));
                if phase == Some(INITIAL_INVITE_SENT) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached dispatch completion");

        assert_eq!(transport.sends(), 1);
        assert!(!manager.compensate_initial_invite(&owner).await);
        manager.stop().await.expect("stop manager");
    }

    #[tokio::test]
    async fn panic_after_dispatch_admission_retains_owner_and_starts_cleanup() {
        let (manager, transport, _events) = make_test_manager().await;
        manager
            .initial_invite_dispatch_test_hook
            .store(1, Ordering::Release);
        let plan = manager
            .plan_initial_invite(
                Some("session-dispatch-panic".to_string()),
                options("call-dispatch-panic"),
            )
            .await
            .expect("plan");
        let installed = manager.install_initial_invite(plan).expect("install");
        let owner = installed.owner().clone();

        drop(manager.dispatch_initial_invite(installed));
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let phase = manager
                    .initial_invite_installs
                    .get(owner.dialog_id())
                    .map(|record| record.phase.load(Ordering::Acquire));
                if phase == Some(INITIAL_INVITE_WIRE_UNKNOWN)
                    && manager.initial_invite_dispatch_tasks.is_empty()
                    && manager.initial_invite_cleanup_tasks.len() == 1
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("panic ownership handoff");

        assert_eq!(transport.sends(), 0);
        assert!(manager.core.has_dialog(owner.dialog_id()));
        assert!(!manager.compensate_initial_invite(&owner).await);
        expect_incomplete_protocol_drain(&manager).await;
        assert!(manager.initial_invite_dispatch_tasks.is_empty());
        assert!(manager.initial_invite_cleanup_tasks.is_empty());
        assert!(manager
            .initial_invite_installs
            .contains_key(owner.dialog_id()));
    }

    #[tokio::test]
    async fn stop_aborts_and_joins_dispatch_blocked_after_admission() {
        let (manager, transport, _events) = make_test_manager().await;
        manager
            .initial_invite_dispatch_test_hook
            .store(2, Ordering::Release);
        let plan = manager
            .plan_initial_invite(
                Some("session-dispatch-stop".to_string()),
                options("call-dispatch-stop"),
            )
            .await
            .expect("plan");
        let installed = manager.install_initial_invite(plan).expect("install");
        let owner = installed.owner().clone();

        drop(manager.dispatch_initial_invite(installed));
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let phase = manager
                    .initial_invite_installs
                    .get(owner.dialog_id())
                    .map(|record| record.phase.load(Ordering::Acquire));
                if phase == Some(INITIAL_INVITE_DISPATCHING)
                    && manager.initial_invite_dispatch_tasks.len() == 1
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("blocked dispatch admitted");

        expect_incomplete_protocol_drain(&manager).await;

        assert_eq!(transport.sends(), 0);
        assert!(manager.initial_invite_dispatch_tasks.is_empty());
        assert!(manager.initial_invite_cleanup_tasks.is_empty());
        assert!(manager
            .initial_invite_installs
            .contains_key(owner.dialog_id()));
    }

    #[tokio::test]
    async fn wire_unknown_retains_owner_and_refuses_local_compensation() {
        let (manager, transport, _events) = make_test_manager().await;
        transport.fail_next_sends(1);
        let plan = manager
            .plan_initial_invite(Some("session-unknown".to_string()), options("call-unknown"))
            .await
            .expect("plan");
        let installed = manager.install_initial_invite(plan).expect("install");
        let owner = installed.owner().clone();

        let completion = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            manager.dispatch_initial_invite(installed).wait(),
        )
        .await
        .expect("dispatch timeout");
        assert_eq!(completion.wire_outcome(), InitialInviteWireOutcome::Unknown);
        assert!(completion.error().is_some());
        assert!(manager.core.has_dialog(owner.dialog_id()));
        assert_eq!(
            manager
                .core
                .session_to_dialog
                .get("session-unknown")
                .map(|mapped| mapped.value().clone()),
            Some(owner.dialog_id().clone())
        );
        assert!(manager
            .initial_invite_installs
            .get(owner.dialog_id())
            .is_some_and(|record| {
                record.phase.load(Ordering::Acquire) == INITIAL_INVITE_WIRE_UNKNOWN
            }));
        assert!(!manager.compensate_initial_invite(&owner).await);
        assert!(manager.core.has_dialog(owner.dialog_id()));
        expect_incomplete_protocol_drain(&manager).await;
        assert!(manager
            .initial_invite_installs
            .contains_key(owner.dialog_id()));
    }

    #[tokio::test]
    async fn compatibility_failure_supervisor_retains_until_invite_is_terminal() {
        let (manager, transport, _events) = make_test_manager().await;
        transport.fail_next_sends(1);

        let result = manager
            .make_call_for_session(
                "session-compat-failure",
                "sip:alice@example.com",
                "sip:bob@127.0.0.1:5099",
                None,
                Some("call-compat-failure".to_string()),
            )
            .await;
        assert!(result.is_err());

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while transport.cancel_sends() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("exact CANCEL send");
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;

        assert_eq!(transport.sends(), 2, "one INVITE and one exact CANCEL");
        assert_eq!(transport.cancel_sends(), 1);
        assert_eq!(manager.initial_invite_installs.len(), 1);
        assert_eq!(manager.initial_invite_cleanup_tasks.len(), 1);
        assert_eq!(manager.core.dialog_count(), 1);
        assert_eq!(manager.core.invite_failover_plans.len(), 1);
        assert_eq!(
            manager
                .core
                .invite_failover_plan_reservations
                .load(Ordering::Acquire),
            1
        );
        assert!(manager
            .core
            .session_to_dialog
            .contains_key("session-compat-failure"));
        expect_incomplete_protocol_drain(&manager).await;
        assert_eq!(manager.initial_invite_installs.len(), 1);
        assert!(manager.initial_invite_cleanup_tasks.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn staged_install_capacity_is_atomic_and_owned_by_exact_records() {
        let (mut manager, transport, mut events) = make_test_manager().await;
        assert_eq!(
            manager.initial_invite_install_slots.available_permits(),
            manager.core.invite_failover_active_plan_capacity
        );

        // Keep the concurrency proof small while exercising the same owned
        // semaphore path used with the configured production capacity.
        const CAPACITY: usize = 4;
        manager.initial_invite_install_slots = Arc::new(tokio::sync::Semaphore::new(CAPACITY));
        let barrier = Arc::new(tokio::sync::Barrier::new(CAPACITY + 2));
        let mut tasks = Vec::new();
        for index in 0..=CAPACITY {
            let plan = manager
                .plan_initial_invite(
                    Some(format!("capacity-session-{index}")),
                    options(&format!("capacity-call-{index}")),
                )
                .await
                .expect("plan");
            let task_manager = manager.clone();
            let task_barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                task_barrier.wait().await;
                task_manager.install_initial_invite(plan)
            }));
        }
        barrier.wait().await;

        let mut installed = Vec::new();
        let mut rejected = 0usize;
        for task in tasks {
            match task.await.expect("install task") {
                Ok(value) => installed.push(value),
                Err(ApiError::Dialog { message }) => {
                    assert_eq!(message, "Initial INVITE admission capacity exhausted");
                    rejected = rejected.saturating_add(1);
                }
                Err(error) => panic!("unexpected install error: {error}"),
            }
        }

        assert_eq!(installed.len(), CAPACITY);
        assert_eq!(rejected, 1);
        assert_eq!(manager.initial_invite_installs.len(), CAPACITY);
        assert_eq!(manager.initial_invite_install_slots.available_permits(), 0);
        assert_eq!(transport.sends(), 0);
        assert!(events.try_recv().is_err());

        for value in installed {
            assert!(manager.compensate_initial_invite(value.owner()).await);
        }
        assert_eq!(
            manager.initial_invite_install_slots.available_permits(),
            CAPACITY
        );
        manager.stop().await.expect("stop manager");
    }

    #[tokio::test]
    async fn ambiguous_cancel_error_is_not_retried_and_retains_exact_owner() {
        let (manager, transport, _events) = make_test_manager().await;
        // The INVITE and then the exact CANCEL both cross the transport call
        // boundary and report an error.
        transport.fail_next_sends(2);

        let result = manager
            .make_call_for_session(
                "session-ambiguous-cancel",
                "sip:alice@example.com",
                "sip:bob@127.0.0.1:5099",
                None,
                Some("call-ambiguous-cancel".to_string()),
            )
            .await;
        assert!(result.is_err());

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while transport.cancel_sends() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("CANCEL attempt");
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;

        assert_eq!(transport.sends(), 2, "one INVITE and one CANCEL only");
        assert_eq!(
            transport.cancel_sends(),
            1,
            "ambiguous CANCEL is not retried"
        );
        assert_eq!(manager.initial_invite_installs.len(), 1);
        assert_eq!(manager.initial_invite_cleanup_tasks.len(), 1);
        let owner = manager
            .initial_invite_installs
            .iter()
            .next()
            .expect("retained exact owner");
        assert_eq!(
            owner.phase.load(Ordering::Acquire),
            INITIAL_INVITE_WIRE_UNKNOWN
        );
        assert!(manager.core.has_dialog(owner.key()));
        assert_eq!(
            manager
                .core
                .invite_failover_plan_reservations
                .load(Ordering::Acquire),
            1
        );
        let retained_plan = manager
            .core
            .invite_failover_plans
            .iter()
            .next()
            .expect("retained failover plan")
            .value()
            .clone();
        drop(owner);
        assert_eq!(
            retained_plan.lock().await.phase,
            crate::manager::transaction_integration::InviteFailoverPlanPhase::WireUnknown
        );

        expect_incomplete_protocol_drain(&manager).await;
        assert!(manager.initial_invite_cleanup_tasks.is_empty());
        assert_eq!(manager.initial_invite_installs.len(), 1);
    }

    #[tokio::test]
    async fn stop_aborts_and_joins_a_blocked_cleanup_task() {
        let (manager, transport, _events) = make_test_manager().await;
        transport.fail_next_sends(1);
        manager
            .initial_invite_cleanup_test_hook
            .store(1, Ordering::Release);

        let result = manager
            .make_call_for_session(
                "session-blocked-cleanup",
                "sip:alice@example.com",
                "sip:bob@127.0.0.1:5099",
                None,
                Some("call-blocked-cleanup".to_string()),
            )
            .await;
        assert!(result.is_err());
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while manager.initial_invite_cleanup_tasks.len() != 1
                || manager
                    .initial_invite_cleanup_test_hook
                    .load(Ordering::Acquire)
                    != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cleanup task entered injected block");
        assert_eq!(manager.initial_invite_cleanup_tasks.len(), 1);

        expect_incomplete_protocol_drain(&manager).await;

        assert_eq!(transport.cancel_sends(), 1);
        assert!(manager.initial_invite_cleanup_tasks.is_empty());
        assert_eq!(manager.initial_invite_installs.len(), 1);
    }

    #[tokio::test]
    async fn stop_drains_a_real_cancel_attempt_before_preserving_unknown_owner() {
        let (manager, transport, _events) = make_test_manager().await;
        transport.fail_next_sends(1);
        manager
            .initial_invite_cleanup_test_hook
            .store(2, Ordering::Release);

        let result = manager
            .make_call_for_session(
                "session-stop-protocol-drain",
                "sip:alice@example.com",
                "sip:bob@127.0.0.1:5099",
                None,
                Some("call-stop-protocol-drain".to_string()),
            )
            .await;
        assert!(result.is_err());
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while manager
                .initial_invite_cleanup_test_hook
                .load(Ordering::Acquire)
                != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cleanup entered pre-attempt delay");
        assert_eq!(transport.cancel_sends(), 0);

        expect_incomplete_protocol_drain(&manager).await;

        assert_eq!(transport.cancel_sends(), 1);
        assert_eq!(manager.initial_invite_installs.len(), 1);
        assert_eq!(manager.core.dialog_count(), 1);
        assert_eq!(manager.core.invite_failover_plans.len(), 1);
        assert!(manager.initial_invite_cleanup_tasks.is_empty());
    }

    #[tokio::test]
    async fn exact_compensation_preserves_replacement_session_mapping() {
        let (manager, _transport, _events) = make_test_manager().await;
        let plan = manager
            .plan_initial_invite(Some("session-reused".to_string()), options("call-old"))
            .await
            .expect("plan");
        let installed = manager.install_initial_invite(plan).expect("install");
        let old_owner = installed.owner().clone();

        let replacement = Dialog::new_early(
            "call-new".to_string(),
            "sip:alice@example.com".parse().expect("local URI"),
            "sip:carol@127.0.0.1:5098".parse().expect("remote URI"),
            None,
            None,
            true,
        );
        let replacement_id = replacement.id.clone();
        manager
            .core
            .dialogs
            .insert(replacement_id.clone(), replacement);
        manager
            .core
            .session_to_dialog
            .insert("session-reused".to_string(), replacement_id.clone());
        manager
            .core
            .dialog_to_session
            .insert(replacement_id.clone(), "session-reused".to_string());

        assert!(manager.compensate_initial_invite(&old_owner).await);
        assert_eq!(
            manager
                .core
                .session_to_dialog
                .get("session-reused")
                .map(|mapped| mapped.value().clone()),
            Some(replacement_id.clone())
        );
        assert!(manager.core.has_dialog(&replacement_id));
        manager.core.cleanup_dialog_storage(&replacement_id);
    }
}
