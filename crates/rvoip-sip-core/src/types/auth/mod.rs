//! # SIP Authentication Types
//!
//! This module provides types for handling SIP authentication as defined in
//! [RFC 3261 Section 22](https://datatracker.ietf.org/doc/html/rfc3261#section-22) and
//! [RFC 7616](https://datatracker.ietf.org/doc/html/rfc7616) (HTTP Digest Authentication).

// Import submodules
mod authentication_info;
mod authorization;
mod challenge;
mod credentials;
mod params;
mod proxy_authenticate;
mod proxy_authorization;
mod scheme;
mod www_authenticate;

// Re-export all public types
pub use self::authentication_info::AuthenticationInfo;
pub use self::authorization::Authorization;
pub use self::challenge::Challenge;
pub use self::credentials::Credentials;
pub use self::params::{AuthParam, AuthenticationInfoParam, DigestParam};
pub use self::proxy_authenticate::ProxyAuthenticate;
pub use self::proxy_authorization::ProxyAuthorization;
pub use self::scheme::{Algorithm, AuthScheme, Qop};
pub use self::www_authenticate::WwwAuthenticate;
