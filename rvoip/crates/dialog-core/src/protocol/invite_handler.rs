//! INVITE Request Handler for Dialog-Core
//!
//! This module provides specialized handling for INVITE requests according to RFC 3261.
//! INVITE is the most complex SIP method as it can create dialogs, modify sessions,
//! and requires careful state management.
//!
//! ## INVITE Types Handled
//!
//! - **Initial INVITE**: Creates new dialogs and establishes sessions
//! - **Re-INVITE**: Modifies existing sessions within established dialogs  
//! - **Refresh INVITE**: Refreshes session timers and state
//!
//! ## Dialog Creation Process
//!
//! 1. Parse INVITE request and validate headers
//! 2. Create early dialog upon receiving INVITE
//! 3. Send provisional responses (100 Trying, 180 Ringing)
//! 4. Confirm dialog with 2xx response
//! 5. Complete with ACK reception
//!
//! ## Implementation Status
//!
//! Currently, INVITE handling is implemented directly in DialogManager's protocol_handlers.
//! This module serves as a placeholder for future modularization when INVITE logic
//! becomes complex enough to warrant separation.

// TODO: Implement INVITE handler as separate module
// For now, INVITE handling is implemented directly in DialogManager 