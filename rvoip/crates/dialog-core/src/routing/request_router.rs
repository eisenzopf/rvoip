//! SIP Request Routing for Dialog Management
//!
//! This module implements RFC 3261 compliant request routing that determines
//! whether incoming SIP requests belong to existing dialogs or require new dialog creation.
//!
//! ## Routing Algorithm
//!
//! For each incoming request:
//! 1. **Extract Dialog Identifiers**: Call-ID, From tag, To tag from headers
//! 2. **Dialog Lookup**: Search existing dialogs using bidirectional matching
//! 3. **Method-Specific Routing**: Apply method-specific routing rules
//! 4. **Fallback Handling**: Route to appropriate handler when no dialog exists
//!
//! ## Dialog Identification (RFC 3261 Section 12.2)
//!
//! - **UAC Perspective**: Local=From tag, Remote=To tag  
//! - **UAS Perspective**: Local=To tag, Remote=From tag
//! - **Bidirectional Search**: Try both perspectives for robust matching
//!
//! ## Method-Specific Rules
//!
//! - **INVITE**: Can create new dialogs or be re-INVITE in existing dialog
//! - **BYE**: Must be in existing dialog (481 if not found)
//! - **CANCEL**: Must reference existing INVITE transaction
//! - **ACK**: Can be dialog-level (2xx) or transaction-level (non-2xx)
//! - **OPTIONS**: Usually stateless but can be in-dialog

// TODO: Implement request routing logic 