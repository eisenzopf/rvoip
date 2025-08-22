//! UAC (User Agent Client) API
//! 
//! Provides high-level client APIs for making outgoing SIP calls with
//! progressive disclosure of complexity.

pub mod simple;
pub mod standard;
pub mod builder;
pub mod call;
pub mod types;
pub mod traits;

pub use simple::{SimpleUacClient, SimpleCall};
pub use standard::UacClient;
pub use builder::UacBuilder;
pub use call::{UacCall, UacCallHandle};
pub use types::*;
pub use traits::*;