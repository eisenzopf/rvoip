//! # SIP Authentication Challenge
//!
//! This module defines the Challenge type used in WWW-Authenticate and Proxy-Authenticate headers.

use crate::types::auth::params::{AuthParam, DigestParam};
use crate::types::auth::scheme::Algorithm;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents a challenge (WWW-Authenticate, Proxy-Authenticate)
///
/// A challenge is sent by a server in 401 Unauthorized or 407 Proxy Authentication Required
/// responses to request authentication from a client. Challenges can use different
/// authentication schemes, with Digest being the most common in SIP.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Challenge {
    /// Digest authentication challenge with associated parameters
    Digest { params: Vec<DigestParam> },
    /// Basic authentication challenge (typically just realm)
    Basic { params: Vec<AuthParam> }, // Typically just realm
    /// Bearer authentication challenge (RFC 8898)
    Bearer {
        /// The authentication realm
        realm: String,
        /// Optional scope requirement
        scope: Option<String>,
        /// Optional error code
        error: Option<String>,
        /// Optional error description
        error_description: Option<String>,
    },
    /// Other authentication scheme challenges
    Other {
        scheme: String,
        params: Vec<AuthParam>,
    },
}

fn digest_algorithm_class(algorithm: &Algorithm) -> &'static str {
    match algorithm {
        Algorithm::Md5 => "md5",
        Algorithm::Md5Sess => "md5-sess",
        Algorithm::Sha256 => "sha-256",
        Algorithm::Sha256Sess => "sha-256-sess",
        Algorithm::Sha512 => "sha-512-256",
        Algorithm::Sha512Sess => "sha-512-256-sess",
        Algorithm::Other(_) => "other",
    }
}

impl fmt::Debug for Challenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Digest { params } => {
                let mut algorithm_counts = [0usize; 7];
                for algorithm in params.iter().filter_map(|param| match param {
                    DigestParam::Algorithm(algorithm) => Some(algorithm),
                    _ => None,
                }) {
                    let index = match digest_algorithm_class(algorithm) {
                        "md5" => 0,
                        "md5-sess" => 1,
                        "sha-256" => 2,
                        "sha-256-sess" => 3,
                        "sha-512-256" => 4,
                        "sha-512-256-sess" => 5,
                        _ => 6,
                    };
                    algorithm_counts[index] += 1;
                }
                formatter
                    .debug_struct("Challenge")
                    .field("scheme", &"digest")
                    .field("param_count", &params.len())
                    .field(
                        "realm_present",
                        &params
                            .iter()
                            .any(|param| matches!(param, DigestParam::Realm(_))),
                    )
                    .field(
                        "realm_bytes",
                        &params
                            .iter()
                            .find_map(|param| match param {
                                DigestParam::Realm(value) => Some(value.len()),
                                _ => None,
                            })
                            .unwrap_or(0),
                    )
                    .field(
                        "nonce_present",
                        &params
                            .iter()
                            .any(|param| matches!(param, DigestParam::Nonce(_))),
                    )
                    .field(
                        "nonce_bytes",
                        &params
                            .iter()
                            .find_map(|param| match param {
                                DigestParam::Nonce(value) => Some(value.len()),
                                _ => None,
                            })
                            .unwrap_or(0),
                    )
                    .field(
                        "opaque_present",
                        &params
                            .iter()
                            .any(|param| matches!(param, DigestParam::Opaque(_))),
                    )
                    .field(
                        "opaque_bytes",
                        &params
                            .iter()
                            .find_map(|param| match param {
                                DigestParam::Opaque(value) => Some(value.len()),
                                _ => None,
                            })
                            .unwrap_or(0),
                    )
                    .field(
                        "domain_count",
                        &params
                            .iter()
                            .filter_map(|param| match param {
                                DigestParam::Domain(domains) => Some(domains.len()),
                                _ => None,
                            })
                            .sum::<usize>(),
                    )
                    .field(
                        "qop_count",
                        &params
                            .iter()
                            .filter_map(|param| match param {
                                DigestParam::Qop(qop) => Some(qop.len()),
                                _ => None,
                            })
                            .sum::<usize>(),
                    )
                    .field("algorithm_md5_count", &algorithm_counts[0])
                    .field("algorithm_md5_sess_count", &algorithm_counts[1])
                    .field("algorithm_sha256_count", &algorithm_counts[2])
                    .field("algorithm_sha256_sess_count", &algorithm_counts[3])
                    .field("algorithm_sha512_256_count", &algorithm_counts[4])
                    .field("algorithm_sha512_256_sess_count", &algorithm_counts[5])
                    .field("algorithm_other_count", &algorithm_counts[6])
                    .finish()
            }
            Self::Basic { params } => formatter
                .debug_struct("Challenge")
                .field("scheme", &"basic")
                .field("param_count", &params.len())
                .field(
                    "realm_present",
                    &params
                        .iter()
                        .any(|param| param.name.eq_ignore_ascii_case("realm")),
                )
                .field(
                    "realm_bytes",
                    &params
                        .iter()
                        .find(|param| param.name.eq_ignore_ascii_case("realm"))
                        .map_or(0, |param| param.value.len()),
                )
                .finish(),
            Self::Bearer {
                realm,
                scope,
                error,
                error_description,
            } => formatter
                .debug_struct("Challenge")
                .field("scheme", &"bearer")
                .field("realm_present", &!realm.is_empty())
                .field("realm_bytes", &realm.len())
                .field("scope_present", &scope.is_some())
                .field("scope_bytes", &scope.as_ref().map_or(0, String::len))
                .field("error_present", &error.is_some())
                .field("error_bytes", &error.as_ref().map_or(0, String::len))
                .field("description_present", &error_description.is_some())
                .field(
                    "description_bytes",
                    &error_description.as_ref().map_or(0, String::len),
                )
                .finish(),
            Self::Other { scheme, params } => formatter
                .debug_struct("Challenge")
                .field("scheme", &"other")
                .field("scheme_present", &!scheme.is_empty())
                .field("scheme_bytes", &scheme.len())
                .field("param_count", &params.len())
                .finish(),
        }
    }
}

impl fmt::Display for Challenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Challenge::Digest { params } => {
                write!(f, "Digest ")?;
                let params_str = params
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{}", params_str)
            }
            Challenge::Basic { params } => {
                write!(f, "Basic ")?;
                let params_str = params
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{}", params_str)
            }
            Challenge::Bearer {
                realm,
                scope,
                error,
                error_description,
            } => {
                write!(f, "Bearer realm=\"{}\"", realm)?;
                if let Some(scope) = scope {
                    write!(f, ", scope=\"{}\"", scope)?;
                }
                if let Some(error) = error {
                    write!(f, ", error=\"{}\"", error)?;
                }
                if let Some(error_desc) = error_description {
                    write!(f, ", error_description=\"{}\"", error_desc)?;
                }
                Ok(())
            }
            Challenge::Other { scheme, params } => {
                write!(f, "{} ", scheme)?;
                let params_str = params
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{}", params_str)
            }
        }
    }
}
