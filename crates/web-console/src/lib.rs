//! # RVOIP Web Console
//!
//! A web-based management console for the rvoip SIP/VoIP stack.
//!
//! Provides a unified HTTP + WebSocket server that aggregates APIs from
//! `call-engine`, `registrar-core`, and `users-core` into a single
//! management interface, along with an embedded React frontend.
//!
//! ## Features
//!
//! - **Dashboard**: Real-time KPI metrics (active calls, registrations, queue depth, agents)
//! - **Call Management**: Live call monitoring, call detail view
//! - **Agent Management**: Agent CRUD, status tracking, skill assignment
//! - **Queue Management**: Queue configuration, real-time depth, SLA monitoring
//! - **SIP Registrations**: Registration list, contact bindings, device tracking
//! - **System Health**: Node status, transport stats, health checks
//! - **Event Stream**: Real-time event tail via WebSocket
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │              Web Browser (React)              │
//! └───────────────┬──────────────┬───────────────┘
//!                 │ HTTP/REST    │ WebSocket
//! ┌───────────────┴──────────────┴───────────────┐
//! │           rvoip-web-console                   │
//! │  ┌─────────┐ ┌─────────┐ ┌────────────────┐  │
//! │  │ API GW  │ │ WS Hub  │ │ Static Serve   │  │
//! │  └────┬────┘ └────┬────┘ │ (rust-embed)   │  │
//! │       │           │      └────────────────┘  │
//! ├───────┴───────────┴──────────────────────────┤
//! │  call-engine  │  registrar-core  │  infra    │
//! └──────────────────────────────────────────────┘
//! ```

pub mod api;
pub mod audit;
pub mod auth;
pub mod rate_limit;
pub mod sip_providers;
pub mod ws;
pub mod server;
pub mod error;
pub mod static_files;

pub use server::WebConsoleServer;
pub use error::{ConsoleError, ConsoleResult};
