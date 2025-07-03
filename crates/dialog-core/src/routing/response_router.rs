//! SIP Response Routing for Dialog Management
//!
//! This module handles routing of SIP responses to the appropriate dialogs and
//! implements proper dialog state transitions based on response status codes.
//!
//! ## Response Categories
//!
//! - **1xx Provisional**: Update dialog state, may create early dialogs
//! - **2xx Success**: Confirm dialogs, complete transactions
//! - **3xx Redirection**: Handle call forwarding and redirects  
//! - **4xx Client Error**: Handle authentication, not found, etc.
//! - **5xx Server Error**: Handle server failures and overload
//! - **6xx Global Failure**: Handle permanent failures
//!
//! ## Dialog State Transitions
//!
//! - **180 Ringing + To-tag**: Initial → Early dialog
//! - **200 OK INVITE**: Early/Initial → Confirmed dialog
//! - **4xx-6xx INVITE**: Terminate early dialog
//! - **200 OK BYE**: Confirmed → Terminated dialog
//!
//! ## Transaction Association
//!
//! Responses are routed using transaction-to-dialog mappings maintained by
//! the transaction integration layer. This ensures responses reach the correct
//! dialog even in complex forking scenarios.

// TODO: Implement response routing logic 