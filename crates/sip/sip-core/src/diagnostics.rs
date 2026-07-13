//! Log-safe diagnostic helpers shared by the SIP crates.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::LazyLock;

/// Per-process keyed hasher used to turn protocol identifiers into bounded
/// diagnostic correlations.
///
/// The random key deliberately changes on restart. The result is useful for
/// grouping events inside one process, but is not a stable identifier and must
/// not be used for protocol routing, authentication, or durable storage.
static DIAGNOSTIC_CORRELATION_HASHER: LazyLock<RandomState> = LazyLock::new(RandomState::new);

/// Derive a bounded, opaque, per-process correlation for diagnostic grouping.
///
/// Keeping this helper in `sip-core` means transport and dialog snapshots use
/// the same correlation for the same SIP Call-ID without either layer storing
/// or exposing the raw identifier.
pub fn opaque_call_correlation(call_id: &str) -> String {
    let digest = DIAGNOSTIC_CORRELATION_HASHER.hash_one(call_id.as_bytes());
    format!("sip-{digest:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_correlation_is_bounded_opaque_and_consistent() {
        const MALICIOUS_CALL_ID: &str = "private-call-id\r\nAuthorization: Bearer secret";

        let first = opaque_call_correlation(MALICIOUS_CALL_ID);
        let second = opaque_call_correlation(MALICIOUS_CALL_ID);
        let different = opaque_call_correlation("another-call-id");

        assert_eq!(first, second);
        assert_ne!(first, different);
        assert_eq!(first.len(), 20);
        assert!(first.starts_with("sip-"));
        assert!(!first.contains(MALICIOUS_CALL_ID));
        assert!(!first.contains("secret"));
    }
}
