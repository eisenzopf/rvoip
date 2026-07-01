//! Translate inbound SIP custom headers into Amazon Connect contact
//! attributes.
//!
//! Amazon Connect contact attributes are the screen-pop channel: the
//! key-value pairs passed to `StartWebRTCContact` become standard contact
//! attributes, readable in the contact flow and via the agent's Streams API
//! `contact.getAttributes()`. Connect imposes two hard rules we enforce here:
//!
//! * **Key charset** — attribute keys may contain only alphanumerics, `-`, and
//!   `_`. SIP header names like `X-Vapi-Customer-Id` are therefore sanitized
//!   (`.` → `_`, etc.) unless an explicit rename is configured.
//! * **Size cap** — there can be at most 32,768 UTF-8 bytes across *all*
//!   key-value pairs per contact. We drop pairs (in insertion order) once the
//!   running total would exceed the cap, and report how many were dropped.
//!
//! The mapping is intentionally pure (no I/O) so it is cheap to unit-test and
//! reuse from application glue independent of the adapter.

use std::collections::BTreeMap;

/// Amazon Connect's documented per-contact attribute byte budget (sum of all
/// key + value byte lengths). See the `StartWebRTCContact` API reference.
pub const MAX_ATTRIBUTE_BYTES: usize = 32_768;

/// Policy for unmapped (not explicitly renamed) headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UnmappedPolicy {
    /// Drop any header that is not in the explicit `rename` table.
    Drop,
    /// Pass through headers matching the configured prefix (after sanitizing
    /// the key and stripping the prefix).
    #[default]
    PassPrefixed,
}

/// Configurable SIP-header → Connect-attribute translation.
#[derive(Clone, Debug)]
pub struct AttributeMapping {
    /// Explicit header→attribute-key renames, applied case-insensitively on the
    /// SIP header name. Values are used verbatim as the Connect attribute key
    /// (and are themselves sanitized to the legal charset).
    pub rename: BTreeMap<String, String>,
    /// Only headers whose name starts with this prefix (case-insensitive) are
    /// considered for pass-through under [`UnmappedPolicy::PassPrefixed`].
    /// The prefix is stripped from the resulting attribute key.
    pub passthrough_prefix: String,
    /// What to do with headers that are not in `rename`.
    pub unmapped: UnmappedPolicy,
}

impl Default for AttributeMapping {
    fn default() -> Self {
        Self {
            rename: BTreeMap::new(),
            // Vapi and most SIP customizations namespace under `X-`.
            passthrough_prefix: "X-".to_string(),
            unmapped: UnmappedPolicy::PassPrefixed,
        }
    }
}

/// Outcome of translating a header set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MappedAttributes {
    /// The Connect contact attributes (sanitized keys, within the byte cap).
    pub attributes: BTreeMap<String, String>,
    /// Number of pairs dropped because they would exceed [`MAX_ATTRIBUTE_BYTES`].
    pub dropped_for_size: usize,
    /// Header names skipped because no rename matched and the unmapped policy
    /// was [`UnmappedPolicy::Drop`] (or the prefix did not match).
    pub skipped: Vec<String>,
}

impl AttributeMapping {
    /// Add an explicit rename (builder-style).
    pub fn rename(mut self, header: impl Into<String>, attribute: impl Into<String>) -> Self {
        self.rename.insert(header.into(), attribute.into());
        self
    }

    /// Set the unmapped-header policy (builder-style).
    pub fn with_unmapped(mut self, policy: UnmappedPolicy) -> Self {
        self.unmapped = policy;
        self
    }

    /// Translate `headers` (header-name → value) into Connect contact
    /// attributes per this mapping.
    ///
    /// Iteration order of the input is preserved for the size-cap decision, so
    /// callers that care which pairs survive truncation should pass an ordered
    /// map (e.g. `BTreeMap` or `Vec`-backed iteration).
    pub fn translate<I, K, V>(&self, headers: I) -> MappedAttributes
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut out = MappedAttributes::default();
        let mut used_bytes = 0usize;

        for (name, value) in headers {
            let name = name.as_ref();
            let value = value.as_ref();

            let Some(key) = self.attribute_key_for(name) else {
                out.skipped.push(name.to_string());
                continue;
            };
            if key.is_empty() {
                out.skipped.push(name.to_string());
                continue;
            }

            let pair_bytes = key.len() + value.len();
            if used_bytes + pair_bytes > MAX_ATTRIBUTE_BYTES {
                out.dropped_for_size += 1;
                continue;
            }
            used_bytes += pair_bytes;
            out.attributes.insert(key, value.to_string());
        }

        out
    }

    /// Compute the Connect attribute key for a SIP header name, or `None` when
    /// the header should be skipped under the current policy.
    fn attribute_key_for(&self, header: &str) -> Option<String> {
        // Explicit rename wins (case-insensitive match on the header name).
        for (from, to) in &self.rename {
            if from.eq_ignore_ascii_case(header) {
                return Some(sanitize_key(to));
            }
        }

        match self.unmapped {
            UnmappedPolicy::Drop => None,
            UnmappedPolicy::PassPrefixed => {
                if header.len() >= self.passthrough_prefix.len()
                    && header[..self.passthrough_prefix.len()]
                        .eq_ignore_ascii_case(&self.passthrough_prefix)
                {
                    let stripped = &header[self.passthrough_prefix.len()..];
                    Some(sanitize_key(stripped))
                } else {
                    None
                }
            }
        }
    }
}

/// Coerce an arbitrary string into Connect's legal attribute-key charset
/// (`[A-Za-z0-9_-]`). Any other byte becomes `_`. Runs of `_` are collapsed and
/// leading/trailing `_` trimmed so `X-Vapi.Customer Id` → `Vapi_Customer_Id`.
fn sanitize_key(raw: &str) -> String {
    let mut s = String::with_capacity(raw.len());
    let mut last_underscore = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            s.push(ch);
            last_underscore = false;
        } else {
            // Collapse any run of illegal chars into a single underscore.
            if !last_underscore {
                s.push('_');
                last_underscore = true;
            }
        }
    }
    s.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn passes_prefixed_headers_with_sanitized_keys() {
        let m = AttributeMapping::default();
        let out = m.translate(headers(&[
            ("X-Vapi-Customer-Id", "cust_42"),
            ("X-Account.Tier", "gold"),
        ]));
        assert_eq!(
            out.attributes.get("Vapi-Customer-Id"),
            Some(&"cust_42".to_string())
        );
        assert_eq!(
            out.attributes.get("Account_Tier"),
            Some(&"gold".to_string())
        );
        assert!(out.skipped.is_empty());
        assert_eq!(out.dropped_for_size, 0);
    }

    #[test]
    fn explicit_rename_wins_and_is_case_insensitive() {
        let m = AttributeMapping::default().rename("X-Vapi-Customer-Id", "customerId");
        let out = m.translate(headers(&[("x-vapi-customer-id", "abc")]));
        assert_eq!(out.attributes.get("customerId"), Some(&"abc".to_string()));
        assert!(out.attributes.get("Vapi-Customer-Id").is_none());
    }

    #[test]
    fn non_prefixed_headers_are_skipped_by_default() {
        let m = AttributeMapping::default();
        let out = m.translate(headers(&[("Subject", "hi"), ("Referred-By", "sip:a@b")]));
        assert!(out.attributes.is_empty());
        assert_eq!(out.skipped.len(), 2);
    }

    #[test]
    fn drop_policy_drops_everything_not_renamed() {
        let m = AttributeMapping::default()
            .with_unmapped(UnmappedPolicy::Drop)
            .rename("X-Keep", "keep");
        let out = m.translate(headers(&[("X-Keep", "1"), ("X-Drop", "2")]));
        assert_eq!(out.attributes.len(), 1);
        assert_eq!(out.attributes.get("keep"), Some(&"1".to_string()));
        assert_eq!(out.skipped, vec!["X-Drop".to_string()]);
    }

    #[test]
    fn enforces_byte_cap_and_counts_drops() {
        let big = "v".repeat(MAX_ATTRIBUTE_BYTES); // single value already at the cap
        let m = AttributeMapping::default();
        let out = m.translate(headers(&[("X-Small", "ok"), ("X-Big", big.as_str())]));
        // X-Small fits; X-Big blows the budget and is dropped.
        assert_eq!(out.attributes.get("Small"), Some(&"ok".to_string()));
        assert!(out.attributes.get("Big").is_none());
        assert_eq!(out.dropped_for_size, 1);
    }

    #[test]
    fn sanitize_trims_and_collapses() {
        assert_eq!(sanitize_key("X-Vapi.Customer Id"), "X-Vapi_Customer_Id");
        assert_eq!(sanitize_key("___weird@@key___"), "weird_key");
        assert_eq!(sanitize_key("clean-Key_1"), "clean-Key_1");
    }
}
