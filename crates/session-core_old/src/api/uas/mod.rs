//! UAS (User Agent Server) API
//! 
//! Provides high-level server APIs for handling incoming SIP calls with
//! progressive disclosure of complexity.

pub mod simple;
pub mod standard;
pub mod builder;
pub mod types;
pub mod traits;
pub mod handler;
pub mod call;

pub use simple::SimpleUasServer;
pub use standard::UasServer;
pub use builder::UasBuilder;
pub use types::*;
pub use traits::*;
pub use handler::*;
pub use call::UasCallHandle;