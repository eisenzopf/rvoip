//! Core types for registrar and presence functionality

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ============ Registration Types ============

/// Represents a user's registration information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRegistration {
    /// Unique user identifier (e.g., "alice")
    pub user_id: String,
    
    /// List of contact addresses where user can be reached
    pub contacts: Vec<ContactInfo>,
    
    /// When this registration expires
    pub expires: DateTime<Utc>,
    
    /// Whether presence is enabled for this user
    pub presence_enabled: bool,
    
    /// User capabilities (e.g., ["audio", "video", "messaging"])
    pub capabilities: Vec<String>,
    
    /// Registration timestamp
    pub registered_at: DateTime<Utc>,
    
    /// Custom attributes
    pub attributes: HashMap<String, String>,
}

/// Contact information for a registered endpoint
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContactInfo {
    /// SIP URI (e.g., "sip:alice@192.168.1.100:5060")
    pub uri: String,
    
    /// Unique instance identifier for this device/endpoint
    pub instance_id: String,
    
    /// Transport protocol
    pub transport: Transport,
    
    /// User agent string (client software identification)
    pub user_agent: String,
    
    /// When this contact binding expires
    pub expires: DateTime<Utc>,
    
    /// Priority value (0.0 to 1.0, higher is preferred)
    pub q_value: f32,
    
    /// Actual source address if behind NAT
    pub received: Option<String>,
    
    /// Path vector for routing (RFC 3327)
    pub path: Vec<String>,
    
    /// Methods this contact supports
    pub methods: Vec<String>,
}

/// SIP transport protocols
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Transport {
    UDP,
    TCP,
    TLS,
    WS,
    WSS,
    SCTP,
}

// ============ Presence Types ============

/// Complete presence state for a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceState {
    /// User identifier
    pub user_id: String,
    
    /// Basic presence (RFC 3863)
    pub basic_status: BasicStatus,
    
    /// Extended status information
    pub extended_status: Option<ExtendedStatus>,
    
    /// Human-readable note
    pub note: Option<String>,
    
    /// Current activities
    pub activities: Vec<Activity>,
    
    /// Per-device presence information
    pub devices: Vec<DevicePresence>,
    
    /// Last update timestamp
    pub last_updated: DateTime<Utc>,
    
    /// When this presence state expires
    pub expires: Option<DateTime<Utc>>,
    
    /// Priority for presence aggregation
    pub priority: i32,
}

/// Basic presence status (RFC 3863)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BasicStatus {
    /// Available for communication
    Open,
    /// Not available
    Closed,
}

/// Extended presence status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExtendedStatus {
    Available,
    Away,
    Busy,
    DoNotDisturb,
    OnThePhone,
    InMeeting,
    Offline,
    Custom(String),
}

/// Simplified presence status for API
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PresenceStatus {
    Available,
    Busy,
    Away,
    DoNotDisturb,
    Offline,
    InCall,
    Custom(String),
}

impl From<PresenceStatus> for BasicStatus {
    fn from(status: PresenceStatus) -> Self {
        match status {
            PresenceStatus::Available | PresenceStatus::InCall => BasicStatus::Open,
            _ => BasicStatus::Closed,
        }
    }
}

impl From<PresenceStatus> for ExtendedStatus {
    fn from(status: PresenceStatus) -> Self {
        match status {
            PresenceStatus::Available => ExtendedStatus::Available,
            PresenceStatus::Busy => ExtendedStatus::Busy,
            PresenceStatus::Away => ExtendedStatus::Away,
            PresenceStatus::DoNotDisturb => ExtendedStatus::DoNotDisturb,
            PresenceStatus::Offline => ExtendedStatus::Offline,
            PresenceStatus::InCall => ExtendedStatus::OnThePhone,
            PresenceStatus::Custom(s) => ExtendedStatus::Custom(s),
        }
    }
}

/// User activity information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Activity {
    Meeting,
    Lunch,
    Travel,
    Holiday,
    Working,
    Presenting,
    Custom(String),
}

/// Per-device presence information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevicePresence {
    /// Device/instance identifier
    pub instance_id: String,
    
    /// Device-specific status
    pub status: BasicStatus,
    
    /// Device-specific note
    pub note: Option<String>,
    
    /// Device capabilities
    pub capabilities: Vec<String>,
    
    /// Device type (e.g., "mobile", "desktop", "web")
    pub device_type: Option<String>,
    
    /// Last seen timestamp
    pub last_seen: DateTime<Utc>,
}

/// Device information (alias for compatibility)
pub type DeviceInfo = DevicePresence;

// ============ Subscription Types ============

/// Presence subscription information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    /// Unique subscription identifier
    pub id: String,
    
    /// Who is watching (subscriber)
    pub subscriber: String,
    
    /// Who they're watching (presentity)
    pub target: String,
    
    /// Current subscription state
    pub state: SubscriptionState,
    
    /// When this subscription expires
    pub expires_at: DateTime<Utc>,
    
    /// Event sequence number for ordering
    pub event_id: u32,
    
    /// Accepted content types (e.g., ["application/pidf+xml"])
    pub accept_types: Vec<String>,
    
    /// Subscription creation time
    pub created_at: DateTime<Utc>,
    
    /// Last notification time
    pub last_notify: Option<DateTime<Utc>>,
    
    /// Number of notifications sent
    pub notify_count: u32,
}

/// Subscription state (RFC 6665)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubscriptionState {
    /// Subscription pending approval
    Pending,
    /// Subscription active
    Active,
    /// Subscription terminated
    Terminated,
}

// ============ Buddy List Types ============

/// Buddy information for UI display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyInfo {
    /// User identifier
    pub user_id: String,
    
    /// Display name
    pub display_name: Option<String>,
    
    /// Current presence status
    pub status: PresenceStatus,
    
    /// Status note
    pub note: Option<String>,
    
    /// Last update time
    pub last_updated: DateTime<Utc>,
    
    /// Whether this buddy is online (any device)
    pub is_online: bool,
    
    /// Number of active devices
    pub active_devices: usize,
}

/// Buddy list for a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyList {
    /// Owner of this buddy list
    pub user_id: String,
    
    /// List of buddies
    pub buddies: Vec<BuddyInfo>,
    
    /// Last update time
    pub last_updated: DateTime<Utc>,
}

// ============ Configuration Types ============

/// Configuration for the registrar service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrarConfig {
    /// Default registration expiry in seconds
    pub default_expires: u32,
    
    /// Maximum registration expiry in seconds
    pub max_expires: u32,
    
    /// Minimum registration expiry in seconds
    pub min_expires: u32,
    
    /// Enable automatic buddy lists
    pub auto_buddy_lists: bool,
    
    /// Enable presence by default
    pub default_presence_enabled: bool,
    
    /// Maximum contacts per user
    pub max_contacts_per_user: usize,
    
    /// Maximum subscriptions per user
    pub max_subscriptions_per_user: usize,
    
    /// Expiry check interval in seconds
    pub expiry_check_interval: u64,
}

impl Default for RegistrarConfig {
    fn default() -> Self {
        Self {
            default_expires: 3600,      // 1 hour
            max_expires: 86400,         // 24 hours
            min_expires: 60,            // 1 minute
            auto_buddy_lists: true,
            default_presence_enabled: true,
            max_contacts_per_user: 10,
            max_subscriptions_per_user: 100,
            expiry_check_interval: 30,  // Check every 30 seconds
        }
    }
}