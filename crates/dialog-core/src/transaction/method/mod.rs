//! # Method-specific utilities for SIP transactions
//! 
//! This module contains helpers and utilities for specific SIP methods that
//! require special handling at the transaction layer according to RFC 3261 and related RFCs.
//! 
//! ## Method Specialization in SIP Transactions
//! 
//! Several SIP methods have unique behaviors that require special handling:
//! 
//! * **ACK** - Has dual nature depending on response type:
//!   - For non-2xx responses: Part of the INVITE transaction
//!   - For 2xx responses: Creates a separate transaction
//!   - See RFC 3261 Section 17.1.1.3
//! 
//! * **CANCEL** - Has a unique relationship to INVITE:
//!   - Must match an existing INVITE transaction
//!   - Creates its own transaction but affects the matched INVITE
//!   - See RFC 3261 Section 9.1
//! 
//! * **UPDATE** - Used to modify session parameters before establishment:
//!   - Follows special rules for in-dialog requests
//!   - See RFC 3311
//! 
//! ## Transaction Layer Context
//! 
//! The transaction layer must handle these special methods differently from
//! regular requests. The utilities in this module provide the functionality
//! needed to create, validate, and process these special method requests
//! in compliance with the relevant RFCs.
//! 
//! ## Diagram: Method Specialization in Transaction Layer
//! 
//! ```text
//! +----------------+       +----------------+       +----------------+
//! |     ACK        |       |     CANCEL     |       |     UPDATE     |
//! | (ack.rs)       |       | (cancel.rs)    |       | (update.rs)    |
//! +-------+--------+       +-------+--------+       +-------+--------+
//!         |                        |                        |
//!         v                        v                        v
//! +------------------------------------------------+
//! |              Transaction Layer                 |
//! |                                                |
//! | - Special state machine handling               |
//! | - Method-specific matching rules               |
//! | - Specialized timer behavior                   |
//! +------------------------------------------------+
//! ```

pub mod cancel;
pub mod update;
pub mod ack; 