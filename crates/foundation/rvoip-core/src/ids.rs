//! V2.A — ID newtypes moved to `rvoip-core-traits` to break the
//! `rvoip-core → rvoip-vcon → rvoip-auth-core → rvoip-core` cycle.
//! This module re-exports everything so `use rvoip_core::ids::*`
//! call sites keep working unchanged.

pub use rvoip_core_traits::ids::*;
