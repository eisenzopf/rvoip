// Re-export the core types and modules
mod types;
pub use types::{CallDirection, CallState, StateChangeError};

mod events;
pub use events::CallEvent;

mod registry_interface;
pub use registry_interface::CallRegistryInterface;

mod weak_call;
pub use weak_call::WeakCall;

mod call;
pub use call::Call;

// Module for utility functions
mod utils;

// Internal function for validating state transitions
pub(crate) use utils::is_valid_state_transition; 