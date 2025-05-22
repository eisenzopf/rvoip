//! MIKEY payload definitions
//!
//! This module defines the payload types used in MIKEY messages as specified in RFC 3830.

/// MIKEY payload types as defined in RFC 3830
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PayloadType {
    /// Last payload (used to mark the end of payloads)
    Last = 0,
    /// Key data transport payload
    KeyData = 1,
    /// Timestamp payload
    Timestamp = 2,
    /// Random number (RAND) payload
    Rand = 3,
    /// Security policy payload
    SecurityPolicy = 4,
    /// Key validation data payload (for PSK mode)
    KeyValidationData = 5,
    /// General extensions payload
    GeneralExtension = 6,
    /// MAC (Message Authentication Code) payload
    Mac = 9,
    /// Unknown payload type
    Unknown = 255,
}

/// MIKEY Common Header (HDR) payload
#[derive(Debug, Clone)]
pub struct CommonHeader {
    /// Protocol version (3 bits)
    pub version: u8,
    /// Message type (3 bits)
    pub data_type: u8,
    /// Next payload (8 bits)
    pub next_payload: u8,
    /// Verification flag (1 bit)
    pub v_flag: bool,
    /// PRF function (7 bits)
    pub prf_func: u8,
    /// CSP ID map type (8 bits)
    pub csp_id: u16,
    /// CS counter (8 bits)
    pub cs_count: u8,
    /// CS ID map type (8 bits)
    pub cs_id_map_type: u8,
}

/// MIKEY Key Data payload
#[derive(Debug, Clone)]
pub struct KeyDataPayload {
    /// Key type (1 byte)
    pub key_type: u8,
    /// Key data
    pub key_data: Vec<u8>,
    /// Salt data (optional)
    pub salt_data: Option<Vec<u8>>,
    /// Key validity data (optional)
    pub kv_data: Option<Vec<u8>>,
}

/// MIKEY Security Policy payload
#[derive(Debug, Clone)]
pub struct SecurityPolicyPayload {
    /// Policy number (1 byte)
    pub policy_no: u8,
    /// Policy type (1 byte)
    pub policy_type: u8,
    /// Policy parameters
    pub policy_param: Vec<u8>,
}

/// MIKEY Key Validation Data payload
#[derive(Debug, Clone)]
pub struct KeyValidationData {
    /// Validation data (MAC or signature)
    pub validation_data: Vec<u8>,
}

/// MIKEY General Extension payload
#[derive(Debug, Clone)]
pub struct GeneralExtensionPayload {
    /// Extension type (1 byte)
    pub ext_type: u8,
    /// Extension data
    pub ext_data: Vec<u8>,
} 