//! Transfer coordination module
//!
//! Provides unified transfer logic for all three transfer types:
//! - Blind Transfer: Immediate transfer without consultation
//! - Attended Transfer: Transfer after consultation with target
//! - Managed Transfer: Conference-based transfer
//!
//! This module provides 70% code reuse across all transfer types
//! by centralizing the common operations in TransferCoordinator.

pub mod coordinator;
pub mod notify;
pub mod types;

pub use coordinator::TransferCoordinator;
pub use notify::TransferNotifyHandler;
pub use types::{TransferOptions, TransferProgress, TransferResult};
