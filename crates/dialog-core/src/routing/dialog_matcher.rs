//! Dialog Matching Logic for SIP Request Routing
//!
//! This module implements the core RFC 3261 dialog matching algorithm used
//! to determine if incoming SIP requests belong to existing dialogs.
//!
//! ## Matching Algorithm
//!
//! According to RFC 3261 Section 12.2, dialogs are identified by:
//! - Call-ID
//! - Local tag (From tag for UAC, To tag for UAS)
//! - Remote tag (To tag for UAC, From tag for UAS)
//!
//! ## Implementation Status
//!
//! Currently implemented in manager/message_routing.rs and manager/utils.rs.
//! This module will contain the extracted matching logic when fully modularized.

// TODO: Extract dialog matching logic from manager modules 