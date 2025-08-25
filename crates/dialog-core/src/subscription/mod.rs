//! Subscription management for SIP event subscriptions (RFC 6665)
//!
//! This module provides the subscription manager and related types for handling
//! SIP event subscriptions, including subscription lifecycle, refresh timers,
//! and event package support.

pub mod manager;
pub mod event_package;

pub use manager::SubscriptionManager;
pub use event_package::{EventPackage, PresencePackage};