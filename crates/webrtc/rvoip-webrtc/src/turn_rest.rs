//! Helper for the TURN REST API (RFC 7635 / draft-uberti-rtcweb-turn-rest-00).
//!
//! Generates an [`IceServerConfig`] with an ephemeral username/credential pair
//! that a TURN server can validate against a shared secret without contacting
//! a credential service per session.
//!
//! ## Algorithm
//!
//! - `username = ${expiry_unix_secs}:${username_hint}` (or just
//!   `${expiry_unix_secs}` when no hint is given).
//! - `credential = base64(HMAC-SHA256(secret, username))`
//!
//! The original RFC uses HMAC-SHA1 for parity with TURN's long-term-credential
//! mechanism. Modern coturn supports SHA-256 via the `--lt-cred-mech` /
//! `--oauth` flags. If you must interop with classic HMAC-SHA1, swap the
//! algorithm in [`compute_credential`] or generate the credential externally
//! and feed the result into [`IceServerConfig`] directly.
//!
//! ## Example
//!
//! ```no_run
//! use std::time::Duration;
//! use rvoip_webrtc::turn_rest::generate_ephemeral;
//!
//! let ice = generate_ephemeral(
//!     "turn:turn.example.com:3478",
//!     b"shared-secret",
//!     Duration::from_secs(3600),
//!     Some("alice"),
//! );
//! ```

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::config::IceServerConfig;

type HmacSha256 = Hmac<Sha256>;

/// Build an [`IceServerConfig`] whose `username`/`credential` pair is a TURN
/// REST credential good for `ttl`, signed with `secret`.
pub fn generate_ephemeral(
    url: impl Into<String>,
    secret: &[u8],
    ttl: Duration,
    username_hint: Option<&str>,
) -> IceServerConfig {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expiry = now.saturating_add(ttl.as_secs());
    let username = match username_hint {
        Some(hint) if !hint.is_empty() => format!("{expiry}:{hint}"),
        _ => format!("{expiry}"),
    };
    let credential = compute_credential(secret, &username);
    IceServerConfig {
        urls: vec![url.into()],
        username: Some(username),
        credential: Some(credential),
    }
}

/// Compute a TURN REST credential: `base64(HMAC-SHA256(secret, username))`.
pub fn compute_credential(secret: &[u8], username: &str) -> String {
    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(mac) => mac,
        Err(_) => unreachable!("HMAC-SHA256 accepts keys of any length"),
    };
    mac.update(username.as_bytes());
    let tag = mac.finalize().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_credential_is_stable_for_fixed_inputs() {
        let username = "1700000000:alice";
        let secret = b"swordfish";
        let cred1 = compute_credential(secret, username);
        let cred2 = compute_credential(secret, username);
        assert_eq!(cred1, cred2, "HMAC must be deterministic");
        assert!(!cred1.is_empty());
    }

    #[test]
    fn generate_includes_url_and_username_with_expiry() {
        let cfg = generate_ephemeral(
            "turn:turn.example.com:3478",
            b"s",
            Duration::from_secs(3600),
            Some("bob"),
        );
        assert_eq!(cfg.urls, vec!["turn:turn.example.com:3478"]);
        let username = cfg.username.clone().expect("username");
        assert!(username.ends_with(":bob"));
        let expiry: u64 = username
            .split(':')
            .next()
            .unwrap()
            .parse()
            .expect("expiry parses");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(
            expiry >= now + 3500 && expiry <= now + 3700,
            "expiry near now+1h"
        );
        assert!(cfg.credential.is_some());
    }

    #[test]
    fn generate_without_hint_omits_colon() {
        let cfg = generate_ephemeral("turn:x", b"k", Duration::from_secs(60), None);
        let username = cfg.username.clone().unwrap();
        assert!(!username.contains(':'), "no hint → no colon: {username}");
    }
}
