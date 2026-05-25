#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub mod auth;

#[cfg(feature = "signaling-whip")]
pub mod whip;

#[cfg(feature = "signaling-ws")]
pub mod websocket;

#[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
pub use auth::{
    extract_bearer, AnonymousAuth, AuthContext, AuthRejection, BearerStaticTokenAuth,
    WhipAuthHook, WsAuthHook,
};
