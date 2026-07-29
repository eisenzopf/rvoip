//! Transaction-stateful SIP proxy primitives.
//!
//! `rvoip-sip-proxy` rides on the `TransactionManager` primitives from
//! `rvoip-sip-dialog` but deliberately does NOT consume `DialogManager`.
//! A stateful proxy is dialog-agnostic: it pairs an upstream
//! server-transaction (the leg facing the originating UAC) with one or
//! more downstream client-transactions (the legs facing the target
//! UAS), and forwards requests downstream + responses upstream while
//! implementing a bounded subset of the RFC 3261 §16 processing model.
//!
//! ## Scope
//!
//! - **Transaction-stateful proxy.** An inbound request may fan out
//!   sequentially or in parallel to downstream client transactions.
//! - **Timer C candidate** (§16.8) — tracked per downstream INVITE
//!   branch and greater than three minutes by default.
//! - **§16.6 request processing**: decrement `Max-Forwards`, push own
//!   `Via` with a fresh `z9hG4bK…` branch, leave the route set / body
//!   intact.
//! - **§16.7 response-processing candidate**: pop the top `Via` (the
//!   proxy's own), aggregate failures, and forward matched INVITE 2xx.
//!
//! ## Conformance status
//!
//! The implementation is **partial**, not an RFC-conformance claim.
//! See `docs/RFC3261_CONFORMANCE.md` and
//! `docs/CONFORMANCE_STATUS.md` in the crate source for the normative
//! matrix, known gaps, isolated baseline, and required qualification
//! evidence.

pub mod error;
pub mod local_response;
pub mod proxy;
pub mod routing;

pub use error::{ProxyBuildError, ProxyError, ProxyResult};
pub use local_response::local_response_from_request;
pub use proxy::{
    ForkMode, ProxyConfig, ProxyEvent, ProxyRetentionSnapshot, ProxyRuntimeOptions,
    RedirectDecision, RedirectInfo, RedirectInterceptor, RouteDecision, RouteFn, StatefulProxy,
    UriRouteDecision, UriRouteFn,
};
pub use routing::{
    DefaultProxyResolver, PreparedTarget, ProxyRoutingPolicy, ProxyTarget, RecordRoutePolicy,
    RequestRejection, RoutingPolicyError,
};
