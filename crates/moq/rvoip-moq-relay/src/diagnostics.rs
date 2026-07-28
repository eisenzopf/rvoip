// SPDX-FileCopyrightText: 2026 Bridgefu contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use url::Url;

/// Produce a bounded URL identity for logs. User-info, query values, and
/// fragments are never emitted; callers retain the original URL for trusted
/// routing only.
pub fn redact_url_for_logging(url: &Url) -> String {
    const MAX_BYTES: usize = 256;
    let mut value = format!("{}://", url.scheme());
    match url.host() {
        Some(url::Host::Ipv6(host)) => value.push_str(&format!("[{host}]")),
        Some(host) => value.push_str(&host.to_string()),
        None => value.push_str("<missing-host>"),
    }
    if let Some(port) = url.port() {
        value.push_str(&format!(":{port}"));
    }
    value.push_str(url.path());
    let query_marker = if url.query().is_some() {
        "?<redacted>"
    } else {
        ""
    };
    if value.len() + query_marker.len() > MAX_BYTES {
        let original_len = url.as_str().len();
        let suffix = format!("…<truncated;url_bytes={original_len}>{query_marker}");
        value.truncate(MAX_BYTES.saturating_sub(suffix.len()));
        value.push_str(&suffix);
    } else {
        value.push_str(query_marker);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_diagnostics_are_bounded_and_secret_free() {
        let url = Url::parse(&format!(
            "https://user:password@relay.example/{}?token=very-secret#fragment",
            "a".repeat(1_000)
        ))
        .unwrap();
        let diagnostic = redact_url_for_logging(&url);
        assert!(diagnostic.len() <= 256);
        assert!(diagnostic.contains("?<redacted>"));
        for secret in ["user", "password", "very-secret", "fragment"] {
            assert!(!diagnostic.contains(secret));
        }
    }

    #[test]
    fn url_log_fields_use_the_bounded_redactor() {
        for source in [
            include_str!("api.rs"),
            include_str!("relay.rs"),
            include_str!("bin/moq-relay-ietf/main.rs"),
            include_str!("bin/moq-relay-ietf/api_coordinator.rs"),
            include_str!("bin/moq-relay-ietf/file_coordinator.rs"),
        ] {
            for line in source.lines().filter(|line| line.contains("_url = %")) {
                assert!(
                    line.contains("redact_url_for_logging"),
                    "raw URL diagnostic: {line}"
                );
            }
        }
    }
}
