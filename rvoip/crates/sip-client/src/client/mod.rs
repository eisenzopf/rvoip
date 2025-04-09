// Re-export the core types and modules
mod events;
pub use events::SipClientEvent;

mod registration;
pub(crate) use registration::Registration;

mod lightweight;
pub(crate) use lightweight::LightweightClient;

mod utils;
pub(crate) use utils::{ChannelTransformer, add_response_headers};

mod client;
pub use client::SipClient; 