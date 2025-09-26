// Adapters for dialog and media integration
pub mod b2bua_signaling_handler;
pub mod dialog_adapter;
pub mod media_adapter;
pub mod event_router;
pub mod routing_adapter;
pub mod session_event_handler;
pub mod signaling_interceptor;

// Re-export adapters
pub use b2bua_signaling_handler::B2buaSignalingHandler;
pub use dialog_adapter::DialogAdapter;
pub use media_adapter::{MediaAdapter, NegotiatedConfig};
pub use event_router::EventRouter;
pub use routing_adapter::{
    RoutingAdapter, RoutingDecision, RoutingRule, MatchType,
    MediaMode, LoadBalanceAlgorithm, FailoverConfig, BackendHealth, BackendState
};
pub use session_event_handler::SessionCrossCrateEventHandler;
pub use signaling_interceptor::{SignalingInterceptor, SignalingHandler, DefaultSignalingHandler, SignalingDecision};