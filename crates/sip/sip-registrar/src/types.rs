//! Core types for registrar and presence functionality

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

// ============ Registration Types ============

/// Canonical SIP address-of-record for registered users.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AddressOfRecord(String);

impl fmt::Debug for AddressOfRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AddressOfRecord")
            .field("scheme", &self.scheme())
            .field("user_present", &!self.user().is_empty())
            .field("domain_present", &!self.domain().is_empty())
            .field("bytes", &self.0.len())
            .finish()
    }
}

impl AddressOfRecord {
    pub fn parse(input: &str) -> std::result::Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("AOR cannot be empty".to_string());
        }

        let lower = trimmed.to_ascii_lowercase();
        let (scheme, rest) = if lower.starts_with("sips:") {
            ("sips", &trimmed[5..])
        } else if lower.starts_with("sip:") {
            ("sip", &trimmed[4..])
        } else if trimmed.contains('@') {
            ("sip", trimmed)
        } else {
            return Err(format!(
                "AOR {trimmed} must include a SIP scheme or user@domain"
            ));
        };

        let without_headers = rest.split('?').next().unwrap_or(rest);
        let without_params = without_headers.split(';').next().unwrap_or(without_headers);
        let (user, domain) = without_params
            .split_once('@')
            .ok_or_else(|| format!("AOR {trimmed} must include user@domain"))?;
        if user.is_empty() || domain.is_empty() {
            return Err(format!(
                "AOR {trimmed} must include non-empty user and domain"
            ));
        }

        Ok(Self(format!(
            "{}:{}@{}",
            scheme.to_ascii_lowercase(),
            user,
            domain.to_ascii_lowercase()
        )))
    }

    pub fn from_user_domain(user: &str, domain: &str) -> std::result::Result<Self, String> {
        Self::parse(&format!("sip:{user}@{domain}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn user(&self) -> &str {
        let without_scheme = self
            .0
            .split_once(':')
            .map_or(self.0.as_str(), |(_, rest)| rest);
        without_scheme.split('@').next().unwrap_or(without_scheme)
    }

    pub fn domain(&self) -> &str {
        self.0.split('@').nth(1).unwrap_or("")
    }

    pub fn scheme(&self) -> &str {
        self.0.split_once(':').map_or("sip", |(scheme, _)| scheme)
    }

    pub fn with_domain(&self, domain: &str) -> std::result::Result<Self, String> {
        Self::parse(&format!("{}:{}@{}", self.scheme(), self.user(), domain))
    }
}

impl fmt::Display for AddressOfRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AddressOfRecord {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Represents a user's registration information
#[derive(Clone, Serialize, Deserialize)]
pub struct UserRegistration {
    /// Unique user identifier (e.g., "alice")
    pub user_id: String,

    /// Canonical SIP address-of-record for this registration, when available.
    pub aor: Option<AddressOfRecord>,

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

impl fmt::Debug for UserRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserRegistration")
            .field("user_present", &!self.user_id.is_empty())
            .field("aor_present", &self.aor.is_some())
            .field("contact_count", &self.contacts.len())
            .field("presence_enabled", &self.presence_enabled)
            .field("capability_count", &self.capabilities.len())
            .field("attribute_count", &self.attributes.len())
            .finish()
    }
}

/// Contact information for a registered endpoint
#[derive(Clone, Serialize, Deserialize, PartialEq)]
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

    /// RFC 5626 reg-id for a specific outbound flow.
    pub reg_id: Option<u32>,

    /// Stable flow identifier for transport-level routing.
    pub flow_id: Option<String>,

    /// Current reachability/qualify state for this binding.
    pub reachability: ContactReachability,
}

impl fmt::Debug for ContactInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContactInfo")
            .field("uri_present", &!self.uri.is_empty())
            .field("instance_id_present", &!self.instance_id.is_empty())
            .field("transport", &self.transport)
            .field("user_agent_present", &!self.user_agent.is_empty())
            .field("q_value", &self.q_value)
            .field("received_present", &self.received.is_some())
            .field("path_count", &self.path.len())
            .field("method_count", &self.methods.len())
            .field("reg_id_present", &self.reg_id.is_some())
            .field("flow_id_present", &self.flow_id.is_some())
            .field("reachability", &self.reachability)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactReachability {
    Unknown,
    Reachable,
    Unreachable,
}

impl Default for ContactReachability {
    fn default() -> Self {
        Self::Unknown
    }
}

impl ContactInfo {
    pub fn supports_method(&self, method: &str) -> bool {
        self.methods.is_empty()
            || self
                .methods
                .iter()
                .any(|registered| registered.eq_ignore_ascii_case(method))
    }

    pub fn is_live_for(&self, method: &str, now: DateTime<Utc>) -> bool {
        self.expires > now
            && self.reachability != ContactReachability::Unreachable
            && self.supports_method(method)
    }
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
#[derive(Clone, Serialize, Deserialize)]
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

impl fmt::Debug for PresenceState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PresenceState")
            .field("user_present", &!self.user_id.is_empty())
            .field("basic_status", &self.basic_status)
            .field(
                "extended_status",
                &self
                    .extended_status
                    .as_ref()
                    .map(ExtendedStatus::diagnostic_kind),
            )
            .field("note_present", &self.note.is_some())
            .field("activity_count", &self.activities.len())
            .field("device_count", &self.devices.len())
            .field("expires_present", &self.expires.is_some())
            .field("priority", &self.priority)
            .finish()
    }
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
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
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

impl ExtendedStatus {
    pub const fn diagnostic_kind(&self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Away => "away",
            Self::Busy => "busy",
            Self::DoNotDisturb => "do-not-disturb",
            Self::OnThePhone => "on-the-phone",
            Self::InMeeting => "in-meeting",
            Self::Offline => "offline",
            Self::Custom(_) => "custom",
        }
    }
}

impl fmt::Debug for ExtendedStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.diagnostic_kind())
    }
}

/// Simplified presence status for API
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PresenceStatus {
    Available,
    Busy,
    Away,
    DoNotDisturb,
    Offline,
    InCall,
    Custom(String),
}

impl PresenceStatus {
    pub const fn diagnostic_kind(&self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Busy => "busy",
            Self::Away => "away",
            Self::DoNotDisturb => "do-not-disturb",
            Self::Offline => "offline",
            Self::InCall => "in-call",
            Self::Custom(_) => "custom",
        }
    }
}

impl fmt::Debug for PresenceStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.diagnostic_kind())
    }
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
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Activity {
    Meeting,
    Lunch,
    Travel,
    Holiday,
    Working,
    Presenting,
    Custom(String),
}

impl Activity {
    pub const fn diagnostic_kind(&self) -> &'static str {
        match self {
            Self::Meeting => "meeting",
            Self::Lunch => "lunch",
            Self::Travel => "travel",
            Self::Holiday => "holiday",
            Self::Working => "working",
            Self::Presenting => "presenting",
            Self::Custom(_) => "custom",
        }
    }
}

impl fmt::Debug for Activity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.diagnostic_kind())
    }
}

/// Per-device presence information
#[derive(Clone, Serialize, Deserialize)]
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

impl fmt::Debug for DevicePresence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DevicePresence")
            .field("instance_id_present", &!self.instance_id.is_empty())
            .field("status", &self.status)
            .field("note_present", &self.note.is_some())
            .field("capability_count", &self.capabilities.len())
            .field("device_type_present", &self.device_type.is_some())
            .finish()
    }
}

/// Device information (alias for compatibility)
pub type DeviceInfo = DevicePresence;

// ============ Subscription Types ============

/// Presence subscription information
#[derive(Clone, Serialize, Deserialize)]
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

impl fmt::Debug for Subscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Subscription")
            .field("id_present", &!self.id.is_empty())
            .field("subscriber_present", &!self.subscriber.is_empty())
            .field("target_present", &!self.target.is_empty())
            .field("state", &self.state)
            .field("accept_type_count", &self.accept_types.len())
            .field("last_notify_present", &self.last_notify.is_some())
            .field("notify_count", &self.notify_count)
            .finish()
    }
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
#[derive(Clone, Serialize, Deserialize)]
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

impl fmt::Debug for BuddyInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuddyInfo")
            .field("user_present", &!self.user_id.is_empty())
            .field("display_name_present", &self.display_name.is_some())
            .field("status", &self.status.diagnostic_kind())
            .field("note_present", &self.note.is_some())
            .field("is_online", &self.is_online)
            .field("active_devices", &self.active_devices)
            .finish()
    }
}

/// Buddy list for a user
#[derive(Clone, Serialize, Deserialize)]
pub struct BuddyList {
    /// Owner of this buddy list
    pub user_id: String,

    /// List of buddies
    pub buddies: Vec<BuddyInfo>,

    /// Last update time
    pub last_updated: DateTime<Utc>,
}

impl fmt::Debug for BuddyList {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuddyList")
            .field("user_present", &!self.user_id.is_empty())
            .field("buddy_count", &self.buddies.len())
            .finish()
    }
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

    /// Maximum contacts per address-of-record.
    pub max_contacts_per_aor: usize,

    /// Replace the lowest-priority existing contact when max contacts is reached.
    pub remove_existing: bool,

    /// Remove unreachable contacts before rejecting or replacing live contacts.
    pub remove_unavailable: bool,

    /// Interval in seconds for SIP OPTIONS qualification. None disables active qualify.
    pub qualify_frequency: Option<u64>,

    /// Timeout in seconds for SIP OPTIONS qualification.
    pub qualify_timeout: u64,

    /// Persist and return RFC 3327 Path vectors.
    pub support_path: bool,

    /// Maximum subscriptions per user
    pub max_subscriptions_per_user: usize,

    /// Expiry check interval in seconds
    pub expiry_check_interval: u64,
}

impl Default for RegistrarConfig {
    fn default() -> Self {
        Self {
            default_expires: 3600, // 1 hour
            max_expires: 86400,    // 24 hours
            min_expires: 60,       // 1 minute
            auto_buddy_lists: true,
            default_presence_enabled: true,
            max_contacts_per_user: 10,
            max_contacts_per_aor: 10,
            remove_existing: false,
            remove_unavailable: true,
            qualify_frequency: None,
            qualify_timeout: 3,
            support_path: true,
            max_subscriptions_per_user: 100,
            expiry_check_interval: 30, // Check every 30 seconds
        }
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn public_registrar_types_redact_debug_without_changing_serde() {
        const CANARY: &str = "registrar-type-direct-secret-canary";
        let now = Utc::now();
        let aor = AddressOfRecord::parse(&format!("sip:{CANARY}@example.test")).unwrap();
        let contact = ContactInfo {
            uri: format!("sip:{CANARY}@192.0.2.1"),
            instance_id: CANARY.into(),
            transport: Transport::WSS,
            user_agent: CANARY.into(),
            expires: now,
            q_value: 0.5,
            received: Some(CANARY.into()),
            path: vec![CANARY.into()],
            methods: vec![CANARY.into()],
            reg_id: Some(7),
            flow_id: Some(CANARY.into()),
            reachability: ContactReachability::Reachable,
        };
        let registration = UserRegistration {
            user_id: CANARY.into(),
            aor: Some(aor.clone()),
            contacts: vec![contact.clone()],
            expires: now,
            presence_enabled: true,
            capabilities: vec![CANARY.into()],
            registered_at: now,
            attributes: HashMap::from([(CANARY.into(), CANARY.into())]),
        };
        let device = DevicePresence {
            instance_id: CANARY.into(),
            status: BasicStatus::Open,
            note: Some(CANARY.into()),
            capabilities: vec![CANARY.into()],
            device_type: Some(CANARY.into()),
            last_seen: now,
        };
        let presence = PresenceState {
            user_id: CANARY.into(),
            basic_status: BasicStatus::Open,
            extended_status: Some(ExtendedStatus::Custom(CANARY.into())),
            note: Some(CANARY.into()),
            activities: vec![Activity::Custom(CANARY.into())],
            devices: vec![device.clone()],
            last_updated: now,
            expires: Some(now),
            priority: 1,
        };
        let subscription = Subscription {
            id: CANARY.into(),
            subscriber: CANARY.into(),
            target: CANARY.into(),
            state: SubscriptionState::Active,
            expires_at: now,
            event_id: 1,
            accept_types: vec![CANARY.into()],
            created_at: now,
            last_notify: Some(now),
            notify_count: 2,
        };
        let buddy = BuddyInfo {
            user_id: CANARY.into(),
            display_name: Some(CANARY.into()),
            status: PresenceStatus::Custom(CANARY.into()),
            note: Some(CANARY.into()),
            last_updated: now,
            is_online: true,
            active_devices: 1,
        };
        let buddy_list = BuddyList {
            user_id: CANARY.into(),
            buddies: vec![buddy.clone()],
            last_updated: now,
        };

        let rendered = [
            format!("{aor:?}"),
            format!("{contact:?}"),
            format!("{registration:?}"),
            format!("{device:?}"),
            format!("{presence:?}"),
            format!("{subscription:?}"),
            format!("{buddy:?}"),
            format!("{buddy_list:?}"),
            format!("{:?}", ExtendedStatus::Custom(CANARY.into())),
            format!("{:?}", PresenceStatus::Custom(CANARY.into())),
            format!("{:?}", Activity::Custom(CANARY.into())),
        ];
        for debug in rendered {
            assert!(!debug.contains(CANARY), "payload leaked: {debug}");
        }

        assert!(serde_json::to_string(&registration)
            .unwrap()
            .contains(CANARY));
        assert!(serde_json::to_string(&presence).unwrap().contains(CANARY));
        assert!(serde_json::to_string(&subscription)
            .unwrap()
            .contains(CANARY));
        assert!(serde_json::to_string(&buddy_list).unwrap().contains(CANARY));
    }
}
