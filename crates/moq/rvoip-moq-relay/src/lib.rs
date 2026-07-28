// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! MoQ Relay library for building Media over QUIC relay servers.
//!
//! This crate provides the core relay functionality that can be embedded
//! into other applications. The relay handles:
//!
//! - Accepting QUIC connections from publishers and subscribers
//! - Routing media between local and remote endpoints
//! - Coordinating namespace/track registration across relay clusters
//!
//! The `relay-runtime` feature provides the embeddable relay without its HTTP
//! status server or CLI dependencies. The default `runtime` feature adds those
//! process-facing components and the binary. Disable default features to depend
//! only on the admission contract, including [`SessionAdmission`],
//! [`AdmissionLease`], and [`AdmissionSessionId`].
//!
//! # Example
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use moq_relay_ietf::{Relay, RelayConfig, FileCoordinator};
//!
//! // Create a coordinator (FileCoordinator for multi-relay deployments)
//! let coordinator = FileCoordinator::new("/path/to/coordination/file", "https://relay.example.com");
//!
//! // Configure and create the relay
//! let relay = Relay::new(RelayConfig {
//!     bind: "[::]:443".parse().unwrap(),
//!     tls: tls_config,
//!     coordinator,
//!     // ... other options
//! })?;
//!
//! // Run the relay
//! relay.run().await?;
//! ```

mod admission;
#[cfg(feature = "runtime")]
mod api;
#[cfg(feature = "relay-runtime")]
mod capacity;
#[cfg(feature = "relay-runtime")]
mod consumer;
#[cfg(feature = "relay-runtime")]
mod coordinator;
#[cfg(feature = "relay-runtime")]
mod diagnostics;
#[cfg(feature = "relay-runtime")]
mod local;
#[cfg(feature = "relay-runtime")]
pub mod metrics;
#[cfg(feature = "relay-runtime")]
mod producer;
#[cfg(feature = "relay-runtime")]
mod relay;
#[cfg(feature = "relay-runtime")]
mod remote;
#[cfg(feature = "relay-runtime")]
mod session;
#[cfg(feature = "runtime")]
mod web;

pub use admission::*;
#[cfg(feature = "runtime")]
pub use api::*;
#[cfg(feature = "relay-runtime")]
pub use capacity::*;
#[cfg(feature = "relay-runtime")]
pub use consumer::*;
#[cfg(feature = "relay-runtime")]
pub use coordinator::*;
#[cfg(feature = "relay-runtime")]
pub use diagnostics::*;
#[cfg(feature = "relay-runtime")]
pub use local::*;
#[cfg(feature = "relay-runtime")]
pub use producer::*;
#[cfg(feature = "relay-runtime")]
pub use relay::*;
#[cfg(feature = "relay-runtime")]
pub use remote::{
    RemoteCapacityError, RemoteCapacityResource, RemoteManager, RemoteManagerLimits,
    RemoteManagerLimitsError, RemoteManagerSnapshot,
};
#[cfg(feature = "relay-runtime")]
pub use session::*;
#[cfg(feature = "runtime")]
pub use web::*;
