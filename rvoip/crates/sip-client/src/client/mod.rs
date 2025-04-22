// Re-export the core types and modules
mod events;
pub use events::SipClientEvent;

mod registration;
pub use registration::Registration;

mod lightweight;
pub use lightweight::LightweightClient;

mod utils;
pub use utils::{ChannelTransformer, add_response_headers};

mod client;
pub use client::SipClient; 