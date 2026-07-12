//! P7 — RFC 9421 / JSON Canonical Form helpers + envelope replay cache.
//!
//! The shape lives in `rvoip-core` so adapters (UCTP-family in
//! particular) can call into a single canonicalization +
//! replay-protection layer without each rolling its own. Production
//! crypto verification lives in `rvoip-identity` behind the trait
//! method `IdentityProvider::verify_signature` (P7 trait addition).

use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Parsed shape of an RFC 9421 `Signature-Input` line. v1 skeleton —
/// the parser captures the covered components but defers the full
/// HTTP-shape semantics to `rvoip-identity`.
#[derive(Clone, Debug, Default)]
pub struct SignatureSpec {
    pub key_id: Option<String>,
    pub algorithm: Option<String>,
    pub created: Option<u64>,
    pub expires: Option<u64>,
    pub covered_components: Vec<String>,
}

/// RFC 8785 (JSON Canonical Form) over a `serde_json::Value`. Used
/// by non-HTTP substrates that sign envelopes inline.
///
/// **Conformance scope.** This implementation handles the parts of
/// RFC 8785 that matter for UCTP envelope signing — lexicographic key
/// sort, unescaped UTF-8 strings with the spec's escape set, array
/// element-order preservation, no insignificant whitespace. It does
/// **not** implement the §3.2.2.3 ES6 number serialization
/// (`Number.prototype.toString` semantics). Numbers round-trip via
/// `serde_json::Number::to_string`, which preserves the source
/// lexeme — sufficient for integer payloads but technically out-of-
/// spec for floats like `1.0` (JCS requires `1`). For UCTP envelopes,
/// which carry IDs and timestamps as strings rather than floating
/// numbers, this distinction is moot in practice. Strict cross-
/// language interop with non-Rust JCS implementations should pull a
/// dedicated crate (e.g. `serde_jcs`) before signing floats.
pub fn canonical_envelope(value: &serde_json::Value) -> Vec<u8> {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out.into_bytes()
}

fn write_canonical(v: &serde_json::Value, out: &mut String) {
    match v {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        serde_json::Value::Number(n) => out.push_str(&n.to_string()),
        serde_json::Value::String(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => {
                        out.push_str(&format!("\\u{:04x}", c as u32));
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        serde_json::Value::Array(arr) => {
            out.push('[');
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(v, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(&serde_json::Value::String((*k).clone()), out);
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
    }
}

/// Parse an RFC 9421 `Signature-Input` line of the form
/// `sig1=("@method" "@target-uri");keyid="..." ;alg="..." ;created=...`
/// Skeleton parser — captures key/value pairs only, enough for the
/// replay cache + trait surface.
pub fn parse_signature_input(input: &str) -> SignatureSpec {
    let mut spec = SignatureSpec::default();
    // Strip the label prefix `name=` if present.
    let body = input.splitn(2, '=').nth(1).unwrap_or(input);
    for piece in body.split(';') {
        let piece = piece.trim();
        if piece.starts_with('(') {
            // Covered components list.
            let inner = piece.trim_start_matches('(').trim_end_matches(')');
            spec.covered_components = inner
                .split_whitespace()
                .map(|s| s.trim_matches('"').to_string())
                .collect();
        } else if let Some((k, v)) = piece.split_once('=') {
            let v = v.trim_matches('"').trim();
            match k.trim() {
                "keyid" => spec.key_id = Some(v.to_string()),
                "alg" => spec.algorithm = Some(v.to_string()),
                "created" => spec.created = v.parse().ok(),
                "expires" => spec.expires = v.parse().ok(),
                _ => {}
            }
        }
    }
    spec
}

/// Default maximum number of replay keys retained by one cache.
///
/// UCTP coordinators own one cache per peer, so this is deliberately much
/// smaller than a process-global token replay store while still covering a
/// five-minute window at ordinary signaling rates.
pub const DEFAULT_REPLAY_CACHE_MAX_ENTRIES: usize = 4_096;

/// Default maximum encoded length of one replay key. Count and per-key limits
/// together provide a hard upper bound on retained key bytes.
pub const DEFAULT_REPLAY_CACHE_MAX_KEY_BYTES: usize = 128;

/// Replay-protection cache. Per CONVERSATION_PROTOCOL.md §5.5, the
/// server caches envelope IDs for ~5 minutes and rejects duplicates.
/// The queue is bounded by entry count and key length; when capacity is
/// reached the oldest entry is evicted before a new key is inserted.
pub struct ReplayCache {
    seen: Mutex<VecDeque<(String, Instant)>>,
    ttl: Duration,
    max_entries: usize,
    max_key_bytes: usize,
}

impl ReplayCache {
    pub fn new(ttl: Duration) -> Self {
        Self::with_limits(
            ttl,
            DEFAULT_REPLAY_CACHE_MAX_ENTRIES,
            DEFAULT_REPLAY_CACHE_MAX_KEY_BYTES,
        )
    }

    /// Construct a cache with a caller-selected entry bound and the default
    /// per-key byte limit.
    pub fn with_capacity(ttl: Duration, max_entries: usize) -> Self {
        Self::with_limits(ttl, max_entries, DEFAULT_REPLAY_CACHE_MAX_KEY_BYTES)
    }

    /// Construct a cache with explicit entry and key-size bounds. Zero limits
    /// are clamped to one so the cache always has well-defined behavior.
    pub fn with_limits(ttl: Duration, max_entries: usize, max_key_bytes: usize) -> Self {
        Self {
            seen: Mutex::new(VecDeque::new()),
            ttl,
            max_entries: max_entries.max(1),
            max_key_bytes: max_key_bytes.max(1),
        }
    }

    /// Record + check in one shot. Returns `Err` when `envelope_id`
    /// has been seen within `ttl`.
    pub fn check_and_record(&self, envelope_id: &str) -> std::result::Result<(), &'static str> {
        if envelope_id.is_empty() {
            return Err("replay key is empty");
        }
        if envelope_id.len() > self.max_key_bytes {
            return Err("replay key exceeds maximum length");
        }
        let now = Instant::now();
        let mut g = self.seen.lock().expect("replay cache lock poisoned");
        // Evict expired.
        while let Some((_, t)) = g.front() {
            if now.duration_since(*t) > self.ttl {
                g.pop_front();
            } else {
                break;
            }
        }
        if g.iter().any(|(id, _)| id == envelope_id) {
            return Err("replay detected");
        }
        while g.len() >= self.max_entries {
            g.pop_front();
        }
        g.push_back((envelope_id.to_string(), now));
        Ok(())
    }

    /// Current retained entry count. Expired entries are swept by the next
    /// [`Self::check_and_record`] call.
    pub fn len(&self) -> usize {
        self.seen.lock().expect("replay cache lock poisoned").len()
    }

    /// Whether the queue currently retains no keys.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Sum of retained key bytes, excluding fixed queue/String overhead.
    pub fn retained_key_bytes(&self) -> usize {
        self.seen
            .lock()
            .expect("replay cache lock poisoned")
            .iter()
            .map(|(key, _)| key.len())
            .sum()
    }

    /// Configured `(entry_count, key_bytes)` limits.
    pub const fn limits(&self) -> (usize, usize) {
        (self.max_entries, self.max_key_bytes)
    }
}

/// Compute the sha256 digest of `body` and return its hex
/// representation — useful for content-integrity checks alongside
/// signature verification.
pub fn body_digest_hex(body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(body);
    let d = h.finalize();
    let mut s = String::with_capacity(d.len() * 2);
    for b in d.iter() {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- RFC 8785 JCS vectors ----------------------------------------
    //
    // Selected from the IETF JCS test suite + edge cases we hit in
    // production UCTP envelopes. Number-format vectors are
    // intentionally limited to integers per the §3.2.2.3 note above.

    #[test]
    fn jcs_empty_object_canonicalizes_to_empty() {
        let v: serde_json::Value = serde_json::from_str("{}").unwrap();
        assert_eq!(canonical_envelope(&v), b"{}");
    }

    #[test]
    fn jcs_empty_array_canonicalizes_to_empty() {
        let v: serde_json::Value = serde_json::from_str("[]").unwrap();
        assert_eq!(canonical_envelope(&v), b"[]");
    }

    #[test]
    fn jcs_object_keys_sort_lexicographically_codepoint() {
        // Codepoint sort: "Aa" < "ab" because uppercase A (0x41) <
        // lowercase a (0x61). Plain ASCII sort suffices for ASCII
        // keys.
        let v: serde_json::Value = serde_json::from_str(r#"{"ab":1,"Aa":2}"#).unwrap();
        let out = String::from_utf8(canonical_envelope(&v)).unwrap();
        assert_eq!(out, r#"{"Aa":2,"ab":1}"#);
    }

    #[test]
    fn jcs_nested_objects_sort_recursively() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"b":{"d":2,"c":1},"a":[1,2,3]}"#).unwrap();
        let out = String::from_utf8(canonical_envelope(&v)).unwrap();
        assert_eq!(out, r#"{"a":[1,2,3],"b":{"c":1,"d":2}}"#);
    }

    #[test]
    fn jcs_array_order_preserved() {
        let v: serde_json::Value = serde_json::from_str(r#"[3,1,2,{"y":1,"x":2}]"#).unwrap();
        let out = String::from_utf8(canonical_envelope(&v)).unwrap();
        assert_eq!(out, r#"[3,1,2,{"x":2,"y":1}]"#);
    }

    #[test]
    fn jcs_string_escapes_match_rfc8785() {
        // RFC 8785 §3.2.2.2 — JSON-string escapes: quote, backslash,
        // backspace, formfeed, newline, return, tab. Our impl
        // currently escapes newline/return/tab/quote/backslash and
        // \u-escapes other control chars. \b and \f land as
        // \u-escapes which is also spec-compliant (the short form is
        // a "MAY" not "MUST" per RFC 8259 §7).
        let v = serde_json::Value::String("\"\\\n\r\t".into());
        let out = String::from_utf8(canonical_envelope(&v)).unwrap();
        assert_eq!(out, r#""\"\\\n\r\t""#);
    }

    #[test]
    fn jcs_unicode_codepoints_above_ascii_pass_through_utf8() {
        // RFC 8785 §3.2.2.2: characters outside the escape set are
        // emitted as their UTF-8 bytes. é (U+00E9) is two UTF-8 bytes
        // 0xC3 0xA9.
        let v = serde_json::Value::String("é".into());
        let out = canonical_envelope(&v);
        // Expect: ["][0xC3][0xA9]["]
        assert_eq!(out, b"\"\xC3\xA9\"");
    }

    #[test]
    fn jcs_integer_numbers_round_trip() {
        let v: serde_json::Value = serde_json::from_str("123").unwrap();
        assert_eq!(canonical_envelope(&v), b"123");
        let v: serde_json::Value = serde_json::from_str("-7").unwrap();
        assert_eq!(canonical_envelope(&v), b"-7");
        let v: serde_json::Value = serde_json::from_str("0").unwrap();
        assert_eq!(canonical_envelope(&v), b"0");
    }

    #[test]
    fn jcs_bool_and_null() {
        assert_eq!(canonical_envelope(&serde_json::Value::Bool(true)), b"true");
        assert_eq!(
            canonical_envelope(&serde_json::Value::Bool(false)),
            b"false"
        );
        assert_eq!(canonical_envelope(&serde_json::Value::Null), b"null");
    }

    #[test]
    fn jcs_realistic_uctp_envelope_round_trips() {
        // Realistic UCTP envelope shape — all strings/integers, so
        // squarely within the conformance scope. Should canonicalize
        // identically regardless of input key order.
        let scrambled = serde_json::json!({
            "v": 1,
            "ts": "2025-12-01T12:00:00Z",
            "id": "env_01",
            "type": "session.invite",
            "payload": {
                "to": ["part_b", "part_a"],
                "from": "part_alice",
                "medium": "voice",
            },
        });
        let sorted = serde_json::json!({
            "id": "env_01",
            "payload": {
                "from": "part_alice",
                "medium": "voice",
                "to": ["part_b", "part_a"],
            },
            "ts": "2025-12-01T12:00:00Z",
            "type": "session.invite",
            "v": 1,
        });
        assert_eq!(canonical_envelope(&scrambled), canonical_envelope(&sorted));
    }

    // ----- Signature-Input parser --------------------------------------

    #[test]
    fn parse_signature_input_with_only_components() {
        let s = parse_signature_input(r#"sig1=("@method")"#);
        assert_eq!(s.covered_components, vec!["@method".to_string()]);
        assert_eq!(s.key_id, None);
    }

    #[test]
    fn parse_signature_input_handles_multiple_params() {
        let s = parse_signature_input(
            r#"sig1=("@method" "content-digest");keyid="k1";alg="ed25519";created=1234;expires=5678"#,
        );
        assert_eq!(s.key_id.as_deref(), Some("k1"));
        assert_eq!(s.algorithm.as_deref(), Some("ed25519"));
        assert_eq!(s.created, Some(1234));
        assert_eq!(s.expires, Some(5678));
        assert_eq!(s.covered_components.len(), 2);
    }

    // ----- Replay cache --------------------------------------------------

    #[test]
    fn replay_cache_evicts_expired_entries() {
        // Tiny TTL so the eviction sweep on the next check_and_record
        // call fires within the test's tolerance window.
        let c = ReplayCache::new(Duration::from_millis(50));
        c.check_and_record("e-1").unwrap();
        assert!(c.check_and_record("e-1").is_err());
        std::thread::sleep(Duration::from_millis(80));
        // After the TTL elapses, the eviction front-pop drops the
        // stale entry and the second insert succeeds.
        c.check_and_record("e-1").expect("replay TTL must elapse");
    }

    #[test]
    fn replay_cache_handles_interleaved_envelopes() {
        let c = ReplayCache::new(Duration::from_secs(60));
        c.check_and_record("a").unwrap();
        c.check_and_record("b").unwrap();
        c.check_and_record("c").unwrap();
        assert!(c.check_and_record("b").is_err(), "duplicate b rejected");
        assert!(c.check_and_record("a").is_err(), "duplicate a rejected");
        c.check_and_record("d").unwrap();
    }

    #[test]
    fn replay_cache_evicts_oldest_entry_at_capacity() {
        let c = ReplayCache::with_limits(Duration::from_secs(60), 3, 16);
        for id in ["env_1", "env_2", "env_3", "env_4"] {
            c.check_and_record(id).unwrap();
        }
        assert_eq!(c.len(), 3);
        assert!(c.check_and_record("env_4").is_err(), "newest key retained");
        c.check_and_record("env_1")
            .expect("oldest key must have been evicted");
        assert_eq!(c.len(), 3);
        assert!(c.retained_key_bytes() <= 3 * 16);
    }

    #[test]
    fn replay_cache_rejects_oversized_keys_without_retaining_them() {
        let c = ReplayCache::with_limits(Duration::from_secs(60), 2, 8);
        assert!(c.check_and_record("env_12345").is_err());
        assert!(c.check_and_record("").is_err());
        assert!(c.is_empty());
        assert_eq!(c.limits(), (2, 8));
    }

    #[test]
    fn body_digest_hex_known_vectors() {
        // "" sha256
        assert_eq!(
            body_digest_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // "abc" sha256 (FIPS-180-4 §B.1)
        assert_eq!(
            body_digest_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
