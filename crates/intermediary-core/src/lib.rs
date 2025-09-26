//! # RVoIP Intermediary Core
//!
//! Core library for building SIP intermediaries including proxies, B2BUAs, SBCs, and gateways.
//!
//! This library provides the foundational components needed to build various types of
//! SIP intermediaries that sit between User Agents in a SIP network.
//!
//! ## Features
//!
//! - **Proxy Mode**: Stateless and stateful proxy operations
//! - **B2BUA Mode**: Back-to-back user agent functionality
//! - **Routing Engine**: Flexible routing with policies and rules
//! - **Policy Framework**: Extensible policy enforcement
//! - **Session Coordination**: Multi-session management and bridging
//!
//! ## Architecture
//!
//! The library is organized into several modules:
//!
//! - `proxy`: Proxy-specific operations and state management
//! - `b2bua`: B2BUA operations including session coordination
//! - `routing`: Common routing logic and decision making
//! - `policy`: Policy enforcement and configuration
//! - `common`: Shared types and utilities
//! - `api`: High-level APIs for different use cases

pub mod common;
pub mod routing;
pub mod policy;

#[cfg(feature = "proxy")]
pub mod proxy;

#[cfg(feature = "b2bua")]
pub mod b2bua;

pub mod api;

// Re-export key types
pub use common::types::*;
pub use routing::RoutingEngine;
pub use policy::PolicyEngine;