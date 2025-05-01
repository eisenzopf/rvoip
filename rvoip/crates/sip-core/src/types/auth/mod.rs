//! # SIP Authentication Types
//! 
//! This module provides types for handling SIP authentication as defined in 
//! [RFC 3261 Section 22](https://datatracker.ietf.org/doc/html/rfc3261#section-22) and
//! [RFC 7616](https://datatracker.ietf.org/doc/html/rfc7616) (HTTP Digest Authentication).

// Import submodules
mod scheme;
mod params;
mod challenge;
mod credentials;
mod www_authenticate;
mod authorization;
mod proxy_authenticate;
mod proxy_authorization;
mod authentication_info;

// Re-export all public types
pub use self::scheme::{AuthScheme, Algorithm, Qop};
pub use self::params::{AuthParam, DigestParam, AuthenticationInfoParam};
pub use self::challenge::Challenge;
pub use self::credentials::Credentials;
pub use self::www_authenticate::WwwAuthenticate;
pub use self::authorization::Authorization;
pub use self::proxy_authenticate::ProxyAuthenticate;
pub use self::proxy_authorization::ProxyAuthorization;
pub use self::authentication_info::AuthenticationInfo; 