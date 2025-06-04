//! BYE Request Handler for Dialog-Core
//!
//! This module handles BYE requests according to RFC 3261 Section 15.
//! BYE requests terminate established SIP dialogs and clean up associated resources.
//!
//! ## BYE Processing Steps
//!
//! 1. **Dialog Identification**: Match BYE to existing dialog using Call-ID and tags
//! 2. **Authorization Check**: Verify BYE is from dialog participant
//! 3. **State Validation**: Ensure dialog is in confirmable state for termination
//! 4. **Resource Cleanup**: Terminate dialog and clean up associated state
//! 5. **Response Generation**: Send 200 OK to acknowledge BYE receipt
//!
//! ## Error Handling
//!
//! - **481 Call/Transaction Does Not Exist**: No matching dialog found
//! - **403 Forbidden**: BYE from unauthorized party
//! - **500 Server Internal Error**: Processing failures
//!
//! ## Implementation Status
//!
//! Currently, BYE handling is implemented directly in DialogManager's protocol_handlers.
//! This module serves as a placeholder for future modularization.

// TODO: Implement BYE handler as separate module
// For now, BYE handling is implemented directly in DialogManager 