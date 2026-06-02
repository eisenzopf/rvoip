//! Stateful SIP proxy primitives — RFC 3261 §16.
//!
//! `rvoip-sip-proxy` rides on the `TransactionManager` primitives from
//! `rvoip-sip-dialog` but deliberately does NOT consume `DialogManager`.
//! A stateful proxy is dialog-agnostic: it pairs an upstream
//! server-transaction (the leg facing the originating UAC) with one or
//! more downstream client-transactions (the legs facing the target
//! UAS), and forwards requests downstream + responses upstream while
//! enforcing the §16.6 / §16.7 processing rules.
//!
//! ## Scope (Phase 6)
//!
//! - **Single-target stateful proxy.** One inbound INVITE / non-INVITE
//!   request fans to exactly one downstream client transaction.
//!   Multi-target forking lives in `forking` in Phase 7.
//! - **Timer C** (§16.8) — INVITE proxy transaction times out at 3 min
//!   by default; app-overridable.
//! - **§16.6 request processing**: decrement `Max-Forwards`, push own
//!   `Via` with a fresh `z9hG4bK…` branch, leave the route set / body
//!   intact.
//! - **§16.7 response processing**: pop the top `Via` (the proxy's
//!   own), forward the rest verbatim upstream.
//!
//! ## What's NOT in Phase 6
//!
//! - Forking + 3xx redirect-set (Phase 7).
//! - Per-flow STIR/SHAKEN re-signing (rides on the existing Phase 2
//!   `pre_send_request` hook on the downstream client transaction).
//! - Recursive routing / DNS lookup (callers supply the destination —
//!   typically via [`rvoip_sip_transport::resolver::Resolver`]).

pub mod error;
pub mod proxy;

pub use error::{ProxyError, ProxyResult};
pub use proxy::{
    ForkMode, ProxyConfig, ProxyEvent, RedirectDecision, RedirectInfo, RedirectInterceptor,
    RouteDecision, RouteFn, StatefulProxy,
};
