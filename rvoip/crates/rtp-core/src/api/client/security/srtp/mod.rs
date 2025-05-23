//! SRTP functionality for client security
//!
//! This module contains components for SRTP key extraction and management.

pub mod keys;
pub mod sdes;

pub use sdes::{SdesClient, SdesClientConfig, SdesClientState}; 