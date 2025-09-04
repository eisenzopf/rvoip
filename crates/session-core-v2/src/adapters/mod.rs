// Adapters for dialog and media integration
pub mod dialog_adapter;
pub mod media_adapter;
pub mod event_router;
pub mod session_event_handler;

// Re-export adapters
pub use dialog_adapter::DialogAdapter;
pub use media_adapter::{MediaAdapter, NegotiatedConfig};
pub use event_router::EventRouter;
pub use session_event_handler::SessionCrossCrateEventHandler;