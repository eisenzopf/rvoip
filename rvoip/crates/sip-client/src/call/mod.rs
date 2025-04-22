// Re-export the core types and modules
mod types;
pub use types::{CallDirection, CallState, StateChangeError};

mod events;
pub use events::CallEvent;

mod registry_interface;
pub use registry_interface::CallRegistryInterface;

mod weak_call;
pub use weak_call::WeakCall;

// Core Call implementation split into separate modules
pub mod call_struct;
mod api;
mod sip_handlers;
mod state;
mod media;
mod dialog;

// Re-export the Call struct
mod call;
pub use call_struct::Call;

// Module for utility functions
mod utils;

// Internal function for validating state transitions
pub use utils::is_valid_state_transition; 