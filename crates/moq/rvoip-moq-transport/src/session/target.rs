// SPDX-FileCopyrightText: 2026 Bridgefu contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::ops::Deref;

use url::{Position, Url};

/// Canonical draft-19 MOQT session target.
///
/// Draft-19 identifies a session with a `moqt://` URI regardless of whether
/// the connection ultimately uses raw QUIC or WebTransport. WebTransport
/// derives its HTTPS request URL by replacing only the scheme.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct SessionTarget {
    canonical: Url,
    authority: String,
    path_and_query: String,
}

impl SessionTarget {
    /// Conservative bound shared with draft-19 session redirect URIs.
    pub const MAX_URI_BYTES: usize = 8_192;

    /// Parse a canonical `moqt://` session target.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, SessionTargetError> {
        let value = value.as_ref();
        if value.len() > Self::MAX_URI_BYTES {
            return Err(SessionTargetError::TooLong);
        }
        if !value.is_ascii() || value.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(SessionTargetError::Malformed(
                "MOQT session URIs must use RFC 3986 ASCII encoding".into(),
            ));
        }
        if raw_uri_authority_contains_userinfo(value) {
            return Err(SessionTargetError::MalformedAuthority);
        }
        let url =
            Url::parse(value).map_err(|error| SessionTargetError::Malformed(error.to_string()))?;
        Self::try_from_url(url)
    }

    /// Validate an already-parsed canonical `moqt://` session target.
    pub fn try_from_url(mut url: Url) -> Result<Self, SessionTargetError> {
        Self::validate_canonical(&url)?;

        // The WHATWG URL model applies a few HTTPS-specific canonicalizations
        // (notably an empty path, dot segments, and the default port). Apply
        // those once to the canonical MOQT target so replacing the scheme for
        // WebTransport cannot change its server-visible identity.
        let https = replace_scheme(&url, "https")?;
        url = replace_scheme(&https, "moqt")?;
        let normalized_host = url
            .host_str()
            .expect("validated target must have a host")
            .to_ascii_lowercase();
        url.set_host(Some(&normalized_host))
            .map_err(|_| SessionTargetError::MalformedAuthority)?;
        let authority = url[Position::BeforeUsername..Position::AfterPort].to_string();
        let path_and_query = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_string(),
        };
        Ok(Self {
            canonical: url,
            authority,
            path_and_query,
        })
    }

    /// Convert the HTTPS URL used by a WebTransport CONNECT request back to
    /// the canonical `moqt://` session target.
    pub fn from_webtransport_url(url: &Url) -> Result<Self, SessionTargetError> {
        if url.scheme() != "https" {
            return Err(SessionTargetError::UnsupportedScheme(
                url.scheme().to_string(),
            ));
        }
        let canonical = replace_scheme(url, "moqt")?;
        Self::try_from_url(canonical)
    }

    /// Build a canonical target from the native-QUIC AUTHORITY and PATH setup
    /// options. `path_and_query` is the exact PATH option value.
    pub fn from_setup_parts(
        authority: &str,
        path_and_query: &str,
    ) -> Result<Self, SessionTargetError> {
        validate_authority_text(authority)?;
        validate_path_and_query(path_and_query)?;
        let value = format!("moqt://{authority}{path_and_query}");
        let url = Url::parse(&value).map_err(|_| {
            SessionTargetError::MalformedPath("PATH cannot be represented as an MOQT URI")
        })?;
        Self::try_from_url(url)
    }

    /// Canonical `moqt://` URI, including any application-local fragment.
    pub fn canonical_url(&self) -> &Url {
        &self.canonical
    }

    /// Canonical network URI with its local-only fragment removed.
    pub fn network_url(&self) -> Url {
        let mut url = self.canonical.clone();
        url.set_fragment(None);
        url
    }

    /// HTTPS URL used for a WebTransport extended CONNECT request.
    pub fn webtransport_url(&self) -> Url {
        replace_scheme(&self.network_url(), "https")
            .expect("a validated MOQT target must convert to HTTPS")
    }

    /// URI authority (`host[:port]`) sent in native QUIC AUTHORITY.
    pub fn authority(&self) -> &str {
        &self.authority
    }

    /// RFC 3986 path-abempty component, without the query.
    pub fn path(&self) -> &str {
        self.canonical.path()
    }

    /// Optional URI query without the leading `?`.
    pub fn query(&self) -> Option<&str> {
        self.canonical.query()
    }

    /// Exact PATH setup value: path-abempty followed by `?query` when present.
    pub fn path_and_query(&self) -> &str {
        &self.path_and_query
    }

    /// Routing identity retained for the legacy connection-path accessor.
    /// Empty and root-only paths remain unscoped; a query makes the target
    /// non-root and is retained consistently on both substrates.
    pub fn routing_path(&self) -> Option<&str> {
        let target = self.path_and_query();
        if target.is_empty() || target == "/" {
            None
        } else {
            Some(target)
        }
    }

    /// Bounded URI identity suitable for logs and diagnostics. The query is
    /// intentionally never returned because it may contain application
    /// credentials; trusted routing continues to use [`Self::routing_path`].
    pub fn redacted_for_logging(&self) -> String {
        const MAX_DIAGNOSTIC_BYTES: usize = 256;
        let query = if self.query().is_some() {
            "?<redacted>"
        } else {
            ""
        };
        let mut value = format!("moqt://{}{}", self.authority(), self.path());
        if value.len() + query.len() > MAX_DIAGNOSTIC_BYTES {
            let original_len = value.len() + self.query().map_or(0, |query| query.len() + 1);
            let suffix = format!("…<truncated;uri_bytes={original_len}>{query}");
            let keep = MAX_DIAGNOSTIC_BYTES.saturating_sub(suffix.len());
            value.truncate(keep);
            value.push_str(&suffix);
        } else {
            value.push_str(query);
        }
        value
    }

    /// Whether this target's host is the host used for the accepted transport.
    /// Ports are deliberately not compared because a relay may be reached
    /// through a public port mapped to a different local listener port.
    pub fn has_same_host(&self, other: &Url) -> bool {
        self.canonical
            .host_str()
            .zip(other.host_str())
            .is_some_and(|(ours, theirs)| ours.eq_ignore_ascii_case(theirs))
    }

    fn validate_canonical(url: &Url) -> Result<(), SessionTargetError> {
        if url.as_str().len() > Self::MAX_URI_BYTES {
            return Err(SessionTargetError::TooLong);
        }
        if url.scheme() != "moqt" {
            return Err(SessionTargetError::UnsupportedScheme(
                url.scheme().to_string(),
            ));
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url[Position::BeforeUsername..Position::AfterPort].contains('@')
        {
            return Err(SessionTargetError::MalformedAuthority);
        }
        if url.host_str().is_none_or(str::is_empty) {
            return Err(SessionTargetError::MissingAuthority);
        }
        validate_authority_text(&url[Position::BeforeUsername..Position::AfterPort])?;
        validate_path_and_query(&match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_string(),
        })?;
        if let Some(fragment) = url.fragment() {
            validate_fragment(fragment)?;
        }
        Ok(())
    }
}

impl Deref for SessionTarget {
    type Target = Url;

    fn deref(&self) -> &Self::Target {
        self.canonical_url()
    }
}

impl std::fmt::Display for SessionTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.canonical.fmt(formatter)
    }
}

impl std::fmt::Debug for SessionTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("SessionTarget")
            .field(&self.redacted_for_logging())
            .finish()
    }
}

impl TryFrom<Url> for SessionTarget {
    type Error = SessionTargetError;

    fn try_from(url: Url) -> Result<Self, Self::Error> {
        Self::try_from_url(url)
    }
}

impl std::str::FromStr for SessionTarget {
    type Err = SessionTargetError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SessionTargetError {
    #[error("unsupported MOQT session URI scheme: {0}")]
    UnsupportedScheme(String),
    #[error("MOQT session URI is missing an authority")]
    MissingAuthority,
    #[error("MOQT session URI exceeds 8192 bytes")]
    TooLong,
    #[error("malformed MOQT session authority")]
    MalformedAuthority,
    #[error("malformed MOQT session path: {0}")]
    MalformedPath(&'static str),
    #[error("malformed MOQT session URI: {0}")]
    Malformed(String),
}

fn validate_authority_text(authority: &str) -> Result<(), SessionTargetError> {
    if authority.is_empty()
        || !authority.is_ascii()
        || authority.bytes().any(|byte| byte.is_ascii_whitespace())
        || authority.contains(['/', '?', '#', '@'])
    {
        return Err(SessionTargetError::MalformedAuthority);
    }

    validate_component(authority, |byte| {
        is_unreserved(byte) || is_sub_delim(byte) || matches!(byte, b':' | b'[' | b']')
    })
    .map_err(|_| SessionTargetError::MalformedAuthority)?;

    let probe = Url::parse(&format!("https://{authority}/"))
        .map_err(|_| SessionTargetError::MalformedAuthority)?;
    if !probe.username().is_empty()
        || probe.password().is_some()
        || probe.host_str().is_none_or(str::is_empty)
    {
        return Err(SessionTargetError::MalformedAuthority);
    }
    Ok(())
}

fn raw_uri_authority_contains_userinfo(value: &str) -> bool {
    value
        .split_once("://")
        .map(|(_, remainder)| {
            remainder
                .split(['/', '?', '#'])
                .next()
                .is_some_and(|authority| authority.contains('@'))
        })
        .unwrap_or(false)
}

fn validate_path_and_query(value: &str) -> Result<(), SessionTargetError> {
    if !value.is_ascii() || value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(SessionTargetError::MalformedPath(
            "PATH must use RFC 3986 ASCII encoding",
        ));
    }
    if !value.is_empty() && !value.starts_with('/') && !value.starts_with('?') {
        return Err(SessionTargetError::MalformedPath(
            "path-abempty must be empty or start with '/'",
        ));
    }

    let (path, query) = value
        .split_once('?')
        .map_or((value, None), |(path, query)| (path, Some(query)));
    validate_component(path, |byte| is_pchar(byte) || byte == b'/').map_err(|_| {
        SessionTargetError::MalformedPath("path-abempty contains an invalid character")
    })?;
    if let Some(query) = query {
        validate_component(query, |byte| is_pchar(byte) || matches!(byte, b'/' | b'?')).map_err(
            |_| SessionTargetError::MalformedPath("query contains an invalid character"),
        )?;
    }
    Ok(())
}

fn validate_fragment(fragment: &str) -> Result<(), SessionTargetError> {
    let (kind, value) = fragment.split_once(':').ok_or_else(|| {
        SessionTargetError::Malformed("fragment must contain a registered type and ':'".into())
    })?;
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(SessionTargetError::Malformed(
            "fragment type must contain only lowercase letters, digits, and hyphens".into(),
        ));
    }
    validate_component(value, |byte| is_pchar(byte) || matches!(byte, b'/' | b'?'))
        .map_err(|_| SessionTargetError::Malformed("fragment value is not RFC 3986 syntax".into()))
}

fn validate_component(value: &str, allowed_literal: impl Fn(u8) -> bool) -> Result<(), ()> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return Err(());
            }
            index += 3;
        } else if allowed_literal(bytes[index]) {
            index += 1;
        } else {
            return Err(());
        }
    }
    Ok(())
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn is_sub_delim(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
    )
}

fn is_pchar(byte: u8) -> bool {
    is_unreserved(byte) || is_sub_delim(byte) || matches!(byte, b':' | b'@')
}

fn replace_scheme(url: &Url, scheme: &str) -> Result<Url, SessionTargetError> {
    let (_, remainder) = url
        .as_str()
        .split_once(':')
        .ok_or_else(|| SessionTargetError::Malformed("missing URI scheme".into()))?;
    Url::parse(&format!("{scheme}:{remainder}"))
        .map_err(|error| SessionTargetError::Malformed(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_target_preserves_components_and_derives_webtransport_url() {
        let target =
            SessionTarget::parse("moqt://Example.COM:4443/live/a%2Fb?token=x%2Fy#track:audio")
                .unwrap();

        assert_eq!(target.authority(), "example.com:4443");
        assert_eq!(target.path(), "/live/a%2Fb");
        assert_eq!(target.query(), Some("token=x%2Fy"));
        assert_eq!(target.path_and_query(), "/live/a%2Fb?token=x%2Fy");
        assert_eq!(
            target.webtransport_url().as_str(),
            "https://example.com:4443/live/a%2Fb?token=x%2Fy"
        );
        assert_eq!(
            target.network_url().as_str(),
            "moqt://example.com:4443/live/a%2Fb?token=x%2Fy"
        );
    }

    #[test]
    fn setup_parts_round_trip_encoded_path_and_query() {
        let target = SessionTarget::from_setup_parts("relay.example:443", "/a%2Fb?x=%2F").unwrap();
        assert_eq!(target.authority(), "relay.example");
        assert_eq!(target.path_and_query(), "/a%2Fb?x=%2F");
    }

    #[test]
    fn diagnostic_target_redacts_query_credentials() {
        let target = SessionTarget::parse("moqt://relay.example/live?token=very-secret").unwrap();
        let diagnostic = target.redacted_for_logging();
        assert_eq!(diagnostic, "moqt://relay.example/live?<redacted>");
        assert!(!diagnostic.contains("very-secret"));
        assert_eq!(target.routing_path(), Some("/live?token=very-secret"));
        assert!(!format!("{target:?}").contains("very-secret"));
        assert!(format!("{target}").contains("very-secret"));
        assert!(target.canonical_url().as_str().contains("very-secret"));

        let long = SessionTarget::parse(format!(
            "moqt://relay.example/{}?token=secret",
            "a".repeat(1_000)
        ))
        .unwrap();
        let diagnostic = long.redacted_for_logging();
        assert!(diagnostic.len() <= 256);
        assert!(diagnostic.contains("truncated;uri_bytes="));
        assert!(!diagnostic.contains("secret"));
    }

    #[test]
    fn canonical_constructor_rejects_https_while_wt_conversion_accepts_it() {
        let https = Url::parse("https://relay.example/live?q=1").unwrap();
        assert!(matches!(
            SessionTarget::try_from_url(https.clone()),
            Err(SessionTargetError::UnsupportedScheme(scheme)) if scheme == "https"
        ));
        assert_eq!(
            SessionTarget::from_webtransport_url(&https)
                .unwrap()
                .canonical_url()
                .as_str(),
            "moqt://relay.example/live?q=1"
        );
    }

    #[test]
    fn malformed_setup_components_are_rejected_without_normalizing_identity() {
        assert!(matches!(
            SessionTarget::from_setup_parts("relay.example:invalid", "/live"),
            Err(SessionTargetError::MalformedAuthority)
        ));
        assert!(matches!(
            SessionTarget::from_setup_parts("relay.example", "relative"),
            Err(SessionTargetError::MalformedPath(_))
        ));
        assert!(matches!(
            SessionTarget::from_setup_parts("relay.example", "/live#fragment"),
            Err(SessionTargetError::MalformedPath(_))
        ));
    }

    #[test]
    fn canonical_targets_reject_userinfo_without_diagnostic_leakage() {
        for value in [
            "moqt://user@relay.example/live",
            "moqt://user:password@relay.example/live",
            "moqt://@relay.example/live",
        ] {
            let error = SessionTarget::parse(value).unwrap_err();
            assert_eq!(error, SessionTargetError::MalformedAuthority);
            let diagnostic = format!("{error:?} {error}");
            assert!(!diagnostic.contains("user"));
            assert!(!diagnostic.contains("password"));
        }

        let webtransport = Url::parse("https://user:password@relay.example/live").unwrap();
        assert_eq!(
            SessionTarget::from_webtransport_url(&webtransport),
            Err(SessionTargetError::MalformedAuthority)
        );
    }

    #[test]
    fn canonicalization_matches_https_transport_semantics() {
        let root = SessionTarget::parse("moqt://EXAMPLE.com:443").unwrap();
        assert_eq!(root.to_string(), "moqt://example.com/");
        assert_eq!(root.authority(), "example.com");
        assert_eq!(root.path_and_query(), "/");
        assert_eq!(root.routing_path(), None);

        let dots = SessionTarget::parse("moqt://example.com/a/../live").unwrap();
        assert_eq!(dots.path(), "/live");
        assert_eq!(dots.webtransport_url().as_str(), "https://example.com/live");
    }

    #[test]
    fn rfc3986_components_and_fragment_type_are_validated() {
        assert_eq!(
            SessionTarget::parse("moqt://user:pass@example.com/live"),
            Err(SessionTargetError::MalformedAuthority)
        );

        assert!(matches!(
            SessionTarget::from_setup_parts("relay.example", "/bad%XX"),
            Err(SessionTargetError::MalformedPath(_))
        ));
        assert!(matches!(
            SessionTarget::from_setup_parts("bad%XX.example", "/live"),
            Err(SessionTargetError::MalformedAuthority)
        ));
        assert!(SessionTarget::parse("moqt://relay.example/live#track:audio").is_ok());
        assert!(SessionTarget::parse("moqt://relay.example/live#Track:audio").is_err());
        assert!(SessionTarget::parse("moqt://relay.example/live#missing-type").is_err());
    }
}
