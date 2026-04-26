// Adapters for dialog and media integration
pub mod dialog_adapter;
pub mod media_adapter;
pub mod session_event_handler;
pub mod registration_adapter;
pub mod session_api_event;
pub mod srtp_negotiator;

// Re-export adapters
pub use dialog_adapter::DialogAdapter;
pub use media_adapter::{MediaAdapter, NegotiatedConfig};
pub use session_event_handler::SessionCrossCrateEventHandler;
pub use registration_adapter::RegistrationAdapter;
pub use session_api_event::{SessionApiCrossCrateEvent, SESSION_TO_APP_CHANNEL};
pub use srtp_negotiator::{SrtpNegotiator, SrtpPair};
