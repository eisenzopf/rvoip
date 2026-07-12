//! SIP-owned options for transport-neutral outbound origination.
//!
//! [`rvoip_core::OriginateRequest`] carries these options opaquely. The SIP
//! adapter validates and retains them before allocating any SIP session or
//! sending signaling. Header values and authentication material are omitted
//! from every diagnostic representation.
//!
//! This admission layer does not apply the retained values to the wire. The
//! Gate 7 activation step must use one duplicate-preserving append path for
//! both the initial INVITE and authenticated retry, and must validate every
//! generated [`crate::auth::ClientAuthHeader`] value again before converting
//! it to a raw header (an AKA provider can generate arbitrary output). Its
//! capture-UAS suite is the first end-to-end consumer of this context.

use crate::api::headers::policy::{classify, HeaderRole};
use crate::auth::SipClientAuth;
use rvoip_sip_core::types::headers::HeaderName;
use rvoip_sip_core::types::Method;
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use zeroize::Zeroize;

/// Maximum number of ordered application headers on an initial SIP INVITE.
pub const MAX_SIP_INITIAL_HEADERS: usize = 32;
/// Maximum UTF-8 size of one initial SIP header name.
pub const MAX_SIP_INITIAL_HEADER_NAME_BYTES: usize = 128;
/// Maximum UTF-8 size of one initial SIP header value.
pub const MAX_SIP_INITIAL_HEADER_VALUE_BYTES: usize = 4_096;
/// Maximum aggregate UTF-8 size of initial SIP header names and values.
pub const MAX_SIP_INITIAL_HEADER_BYTES: usize = 16 * 1_024;
/// Maximum UTF-8 size of a per-call SIP From URI.
pub const MAX_SIP_ORIGINATE_FROM_URI_BYTES: usize = 4_096;
/// Maximum number of non-nested authentication alternatives retained by one call.
pub const MAX_SIP_ORIGINATE_AUTH_OPTIONS: usize = 8;
/// Maximum UTF-8 size of a Digest or Basic username.
pub const MAX_SIP_ORIGINATE_AUTH_USERNAME_BYTES: usize = 256;
/// Maximum UTF-8 size of a Digest or Basic password.
pub const MAX_SIP_ORIGINATE_AUTH_PASSWORD_BYTES: usize = 4_096;
/// Maximum UTF-8 size of a Digest realm constraint.
pub const MAX_SIP_ORIGINATE_AUTH_REALM_BYTES: usize = 256;
/// Maximum UTF-8 size of a Bearer token.
pub const MAX_SIP_ORIGINATE_BEARER_TOKEN_BYTES: usize = 4_096;
/// Maximum aggregate UTF-8 size of static authentication material.
pub const MAX_SIP_ORIGINATE_AUTH_BYTES: usize = 16 * 1_024;

/// Validation failure for an application-supplied initial SIP header set.
///
/// Variants deliberately carry no supplied names or values, so formatting an
/// error cannot disclose call context or authentication material.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum SipInitialHeadersError {
    /// More headers were supplied than one outbound INVITE may retain.
    #[error("too many initial SIP headers")]
    TooManyHeaders,
    /// A header name was empty.
    #[error("an initial SIP header name is empty")]
    EmptyName,
    /// A header name exceeded its size limit.
    #[error("an initial SIP header name is too large")]
    NameTooLarge,
    /// A header name was not an RFC SIP token.
    #[error("an initial SIP header name is invalid")]
    InvalidName,
    /// A header is owned by the SIP stack, a proxy hop, authentication, or
    /// rvoip's internal namespace.
    #[error("an initial SIP header is forbidden")]
    ForbiddenHeader,
    /// A header value exceeded its size limit.
    #[error("an initial SIP header value is too large")]
    ValueTooLarge,
    /// A header value contained CR, LF, NUL, or another forbidden control.
    #[error("an initial SIP header value is invalid")]
    InvalidValue,
    /// The combined names and values exceeded the aggregate size limit.
    #[error("initial SIP headers are too large in aggregate")]
    AggregateTooLarge,
}

/// Ordered, duplicate-preserving headers for the first outbound SIP INVITE.
///
/// The collection accepts application-controlled SIP headers only. The SIP
/// stack continues to own dialog, transaction, routing, body framing, proxy,
/// authentication, and internal `X-Rvoip*` fields. Application namespaces,
/// including `X-Bridgefu-*`, remain subject to the caller's own allowlist.
/// Values are redacted from `Debug` and zeroized when the collection is
/// released.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct SipInitialHeaders {
    entries: Vec<(HeaderName, String)>,
}

impl SipInitialHeaders {
    /// Validate and retain headers in insertion order.
    ///
    /// Duplicate names remain distinct and retain their relative order.
    pub fn new<I, N, V>(entries: I) -> Result<Self, SipInitialHeadersError>
    where
        I: IntoIterator<Item = (N, V)>,
        N: Into<String>,
        V: Into<String>,
    {
        let mut aggregate_bytes = 0usize;
        let mut validated = Self {
            entries: Vec::with_capacity(MAX_SIP_INITIAL_HEADERS),
        };
        for (supplied_name, value) in entries {
            let mut supplied_name = supplied_name.into();
            let mut value = value.into();
            if validated.entries.len() >= MAX_SIP_INITIAL_HEADERS {
                supplied_name.zeroize();
                value.zeroize();
                return Err(SipInitialHeadersError::TooManyHeaders);
            }
            let next_aggregate = aggregate_bytes
                .checked_add(supplied_name.len())
                .and_then(|total| total.checked_add(value.len()));
            let Some(next_aggregate) = next_aggregate else {
                supplied_name.zeroize();
                value.zeroize();
                return Err(SipInitialHeadersError::AggregateTooLarge);
            };
            if next_aggregate > MAX_SIP_INITIAL_HEADER_BYTES {
                supplied_name.zeroize();
                value.zeroize();
                return Err(SipInitialHeadersError::AggregateTooLarge);
            }
            let result = validate_initial_header(&supplied_name, &value);
            supplied_name.zeroize();
            let name = match result {
                Ok(name) => name,
                Err(error) => {
                    value.zeroize();
                    return Err(error);
                }
            };
            aggregate_bytes = next_aggregate;
            validated.entries.push((name, value));
        }
        Ok(validated)
    }

    /// Number of retained headers, including duplicates.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no initial headers are configured.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over retained headers in their exact insertion order.
    ///
    /// Header values may contain sensitive application context. Avoid logging
    /// or formatting them.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&HeaderName, &str)> {
        self.entries
            .iter()
            .map(|(name, value)| (name, value.as_str()))
    }
}

impl fmt::Debug for SipInitialHeaders {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipInitialHeaders")
            .field("header_count", &self.entries.len())
            .finish()
    }
}

impl Drop for SipInitialHeaders {
    fn drop(&mut self) {
        for (name, value) in &mut self.entries {
            zeroize_header_name(name);
            value.zeroize();
        }
    }
}

/// Validation failure for SIP-owned outbound originate options.
///
/// Errors contain no caller-supplied URI or header values.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum SipOriginateContextError {
    /// The configured From URI was empty or not a valid SIP/SIPS URI.
    #[error("the SIP originate From URI is invalid")]
    InvalidFromUri,
    /// The configured From URI exceeded its admission bound.
    #[error("the SIP originate From URI is too large")]
    FromUriTooLarge,
    /// Static authentication material was empty or contained a forbidden byte.
    #[error("the SIP originate authentication material is invalid")]
    InvalidAuthMaterial,
    /// A Digest or Basic username exceeded its admission bound.
    #[error("the SIP originate authentication username is too large")]
    AuthUsernameTooLarge,
    /// A Digest or Basic password exceeded its admission bound.
    #[error("the SIP originate authentication password is too large")]
    AuthPasswordTooLarge,
    /// A Digest realm constraint exceeded its admission bound.
    #[error("the SIP originate authentication realm is too large")]
    AuthRealmTooLarge,
    /// A Bearer token exceeded its admission bound.
    #[error("the SIP originate Bearer token is too large")]
    BearerTokenTooLarge,
    /// A composite contained too many authentication alternatives.
    #[error("too many SIP originate authentication alternatives")]
    TooManyAuthOptions,
    /// Composite authentication alternatives may not contain composites.
    #[error("nested SIP originate authentication alternatives are forbidden")]
    NestedAuthOptions,
    /// Authentication material exceeded its aggregate admission bound.
    #[error("SIP originate authentication material is too large in aggregate")]
    AuthAggregateTooLarge,
}

/// SIP-specific options carried opaquely by an outbound originate request.
///
/// Use this value with [`rvoip_core::OriginateRequest::with_context`]. The SIP
/// adapter is the only component that interprets it. Authentication, header
/// values, and the optional From URI are all redacted from diagnostics.
#[derive(Clone, Default)]
pub struct SipOriginateContext {
    from_uri: Option<String>,
    auth: Option<SipClientAuth>,
    initial_headers: SipInitialHeaders,
}

impl SipOriginateContext {
    /// Construct empty SIP options that inherit the adapter configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the From URI for this outbound SIP call.
    pub fn with_from_uri(
        mut self,
        from_uri: impl Into<String>,
    ) -> Result<Self, SipOriginateContextError> {
        let mut from_uri = from_uri.into();
        if from_uri.len() > MAX_SIP_ORIGINATE_FROM_URI_BYTES {
            from_uri.zeroize();
            return Err(SipOriginateContextError::FromUriTooLarge);
        }
        let parsed = match rvoip_sip_core::Uri::from_str(&from_uri) {
            Ok(parsed) => parsed,
            Err(_) => {
                from_uri.zeroize();
                return Err(SipOriginateContextError::InvalidFromUri);
            }
        };
        if from_uri.chars().any(char::is_control)
            || !matches!(
                parsed.scheme(),
                rvoip_sip_core::types::uri::Scheme::Sip | rvoip_sip_core::types::uri::Scheme::Sips
            )
        {
            from_uri.zeroize();
            return Err(SipOriginateContextError::InvalidFromUri);
        }
        if let Some(previous) = self.from_uri.as_mut() {
            previous.zeroize();
        }
        self.from_uri = Some(from_uri);
        Ok(self)
    }

    /// Supply typed UAC authentication for challenged INVITE retries.
    pub fn with_auth(mut self, mut auth: SipClientAuth) -> Result<Self, SipOriginateContextError> {
        if let Err(error) = validate_client_auth(&auth) {
            zeroize_client_auth(&mut auth);
            return Err(error);
        }
        if let Some(previous) = self.auth.as_mut() {
            zeroize_client_auth(previous);
        }
        self.auth = Some(auth);
        Ok(self)
    }

    /// Supply validated application-controlled headers for the first INVITE.
    pub fn with_initial_headers(mut self, headers: SipInitialHeaders) -> Self {
        self.initial_headers = headers;
        self
    }

    /// Optional per-call From URI. Treat it as sensitive routing data.
    pub fn from_uri(&self) -> Option<&str> {
        self.from_uri.as_deref()
    }

    /// Optional per-call UAC authentication material.
    pub fn auth(&self) -> Option<&SipClientAuth> {
        self.auth.as_ref()
    }

    /// Validated first-INVITE application headers.
    pub fn initial_headers(&self) -> &SipInitialHeaders {
        &self.initial_headers
    }

    /// Revalidate all retained admission bounds before SIP allocates a route.
    ///
    /// Builders validate eagerly; the adapter calls this again at the opaque
    /// context boundary so future construction paths cannot bypass admission.
    pub fn validate(&self) -> Result<(), SipOriginateContextError> {
        if let Some(from_uri) = self.from_uri.as_deref() {
            if from_uri.len() > MAX_SIP_ORIGINATE_FROM_URI_BYTES {
                return Err(SipOriginateContextError::FromUriTooLarge);
            }
        }
        if let Some(auth) = self.auth.as_ref() {
            validate_client_auth(auth)?;
        }
        Ok(())
    }
}

impl fmt::Debug for SipOriginateContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipOriginateContext")
            .field("has_from_uri", &self.from_uri.is_some())
            .field("has_auth", &self.auth.is_some())
            .field("initial_header_count", &self.initial_headers.len())
            .finish()
    }
}

impl Drop for SipOriginateContext {
    fn drop(&mut self) {
        if let Some(from_uri) = self.from_uri.as_mut() {
            from_uri.zeroize();
        }
        if let Some(auth) = self.auth.as_mut() {
            zeroize_client_auth(auth);
        }
    }
}

fn validate_client_auth(auth: &SipClientAuth) -> Result<(), SipOriginateContextError> {
    let options: &[SipClientAuth] = match auth {
        SipClientAuth::Composite(options) => {
            if options.is_empty() {
                return Err(SipOriginateContextError::InvalidAuthMaterial);
            }
            if options.len() > MAX_SIP_ORIGINATE_AUTH_OPTIONS {
                return Err(SipOriginateContextError::TooManyAuthOptions);
            }
            if options
                .iter()
                .any(|option| matches!(option, SipClientAuth::Composite(_)))
            {
                return Err(SipOriginateContextError::NestedAuthOptions);
            }
            options
        }
        auth => std::slice::from_ref(auth),
    };

    let mut aggregate_bytes = 0usize;
    for auth in options {
        match auth {
            SipClientAuth::Digest(credentials) => {
                validate_auth_string(
                    &credentials.username,
                    MAX_SIP_ORIGINATE_AUTH_USERNAME_BYTES,
                    SipOriginateContextError::AuthUsernameTooLarge,
                    &mut aggregate_bytes,
                )?;
                validate_auth_string(
                    &credentials.password,
                    MAX_SIP_ORIGINATE_AUTH_PASSWORD_BYTES,
                    SipOriginateContextError::AuthPasswordTooLarge,
                    &mut aggregate_bytes,
                )?;
                if let Some(realm) = credentials.realm.as_deref() {
                    validate_auth_string(
                        realm,
                        MAX_SIP_ORIGINATE_AUTH_REALM_BYTES,
                        SipOriginateContextError::AuthRealmTooLarge,
                        &mut aggregate_bytes,
                    )?;
                }
            }
            SipClientAuth::BearerToken(token)
            | SipClientAuth::BearerTokenCleartextAllowed(token) => validate_auth_string(
                token,
                MAX_SIP_ORIGINATE_BEARER_TOKEN_BYTES,
                SipOriginateContextError::BearerTokenTooLarge,
                &mut aggregate_bytes,
            )?,
            SipClientAuth::Basic {
                username, password, ..
            } => {
                validate_auth_string(
                    username,
                    MAX_SIP_ORIGINATE_AUTH_USERNAME_BYTES,
                    SipOriginateContextError::AuthUsernameTooLarge,
                    &mut aggregate_bytes,
                )?;
                if username.contains(':') {
                    return Err(SipOriginateContextError::InvalidAuthMaterial);
                }
                validate_auth_string(
                    password,
                    MAX_SIP_ORIGINATE_AUTH_PASSWORD_BYTES,
                    SipOriginateContextError::AuthPasswordTooLarge,
                    &mut aggregate_bytes,
                )?;
            }
            SipClientAuth::Aka(_) => {}
            SipClientAuth::Composite(_) => {
                return Err(SipOriginateContextError::NestedAuthOptions);
            }
        }
    }
    Ok(())
}

fn validate_auth_string(
    value: &str,
    max_bytes: usize,
    too_large: SipOriginateContextError,
    aggregate_bytes: &mut usize,
) -> Result<(), SipOriginateContextError> {
    if value.len() > max_bytes {
        return Err(too_large);
    }
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(SipOriginateContextError::InvalidAuthMaterial);
    }
    *aggregate_bytes = aggregate_bytes
        .checked_add(value.len())
        .ok_or(SipOriginateContextError::AuthAggregateTooLarge)?;
    if *aggregate_bytes > MAX_SIP_ORIGINATE_AUTH_BYTES {
        return Err(SipOriginateContextError::AuthAggregateTooLarge);
    }
    Ok(())
}

fn zeroize_client_auth(auth: &mut SipClientAuth) {
    match auth {
        SipClientAuth::Digest(credentials) => {
            credentials.username.zeroize();
            credentials.password.zeroize();
            if let Some(realm) = credentials.realm.as_mut() {
                realm.zeroize();
            }
        }
        SipClientAuth::BearerToken(token) | SipClientAuth::BearerTokenCleartextAllowed(token) => {
            token.zeroize()
        }
        SipClientAuth::Basic {
            username, password, ..
        } => {
            username.zeroize();
            password.zeroize();
        }
        SipClientAuth::Aka(_) => {}
        SipClientAuth::Composite(options) => {
            for option in options {
                zeroize_client_auth(option);
            }
        }
    }
}

fn validate_initial_header(
    supplied_name: &str,
    value: &str,
) -> Result<HeaderName, SipInitialHeadersError> {
    if supplied_name.is_empty() {
        return Err(SipInitialHeadersError::EmptyName);
    }
    if supplied_name.len() > MAX_SIP_INITIAL_HEADER_NAME_BYTES {
        return Err(SipInitialHeadersError::NameTooLarge);
    }
    if !supplied_name
        .bytes()
        .all(rvoip_sip_core::parser::token::is_token_char)
    {
        return Err(SipInitialHeadersError::InvalidName);
    }
    if value.len() > MAX_SIP_INITIAL_HEADER_VALUE_BYTES {
        return Err(SipInitialHeadersError::ValueTooLarge);
    }
    if value
        .chars()
        .any(|character| character != '\t' && character.is_control())
    {
        return Err(SipInitialHeadersError::InvalidValue);
    }

    let mut name =
        HeaderName::from_str(supplied_name).map_err(|_| SipInitialHeadersError::InvalidName)?;
    if initial_header_is_forbidden(&name)
        || !matches!(
            classify(Method::Invite, &name),
            HeaderRole::ApplicationControlled
        )
    {
        zeroize_header_name(&mut name);
        return Err(SipInitialHeadersError::ForbiddenHeader);
    }
    Ok(name)
}

fn zeroize_header_name(name: &mut HeaderName) {
    if let HeaderName::Other(name) = name {
        name.zeroize();
    }
}

fn initial_header_is_forbidden(name: &HeaderName) -> bool {
    let wire_name = name.as_str();
    let normalized = wire_name.to_ascii_lowercase();
    if normalized == "x-rvoip"
        || normalized.starts_with("x-rvoip-")
        || normalized == "content"
        || normalized.starts_with("content-")
        || matches!(
            normalized.as_str(),
            // HTTP/WebSocket hop-by-hop names can otherwise enter through
            // HeaderName::Other and are never meaningful on a SIP INVITE.
            "connection"
                | "keep-alive"
                | "proxy-connection"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
                | "host"
                // Identity, feature negotiation, dialog state, and transfer
                // fields remain owned by typed SIP operations.
                | "p-asserted-identity"
                | "p-preferred-identity"
                | "supported"
                | "require"
                | "unsupported"
                | "allow"
                | "allow-events"
                | "session-expires"
                | "min-se"
                | "rack"
                | "rseq"
                | "event"
                | "subscription-state"
                | "refer-to"
                | "referred-by"
                | "replaces"
                | "join"
                | "target-dialog"
                | "path"
                | "service-route"
                | "sip-etag"
                | "sip-if-match"
                | "subject"
        )
    {
        return true;
    }
    matches!(
        name,
        HeaderName::CallId
            | HeaderName::Contact
            | HeaderName::ContentLength
            | HeaderName::ContentType
            | HeaderName::CSeq
            | HeaderName::From
            | HeaderName::MaxForwards
            | HeaderName::To
            | HeaderName::Via
            | HeaderName::RecordRoute
            | HeaderName::Route
            | HeaderName::ProxyRequire
            | HeaderName::Authorization
            | HeaderName::ProxyAuthorization
            | HeaderName::WwwAuthenticate
            | HeaderName::ProxyAuthenticate
            | HeaderName::AuthenticationInfo
            | HeaderName::Identity
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_headers_preserve_exact_insertion_order() {
        let headers = SipInitialHeaders::new([
            ("X-Correlation-Id", "first"),
            ("X-App-Context", "middle"),
            ("x-correlation-id", "second"),
        ])
        .expect("valid headers");
        let retained = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value))
            .collect::<Vec<_>>();
        assert_eq!(
            retained,
            vec![
                ("X-Correlation-Id", "first"),
                ("X-App-Context", "middle"),
                ("x-correlation-id", "second"),
            ]
        );
    }

    #[test]
    fn caller_owned_application_namespaces_are_allowed() {
        let headers = SipInitialHeaders::new([
            ("X-Bridgefu-Correlation-Id", "caller-validated"),
            ("X-Provider-Metadata", "caller-validated"),
        ])
        .expect("rvoip leaves application namespaces to caller allowlists");
        assert_eq!(headers.len(), 2);
    }

    #[test]
    fn count_name_value_and_aggregate_limits_are_enforced() {
        let too_many = (0..=MAX_SIP_INITIAL_HEADERS)
            .map(|index| (format!("X-Test-{index}"), "v".to_string()))
            .collect::<Vec<_>>();
        assert_eq!(
            SipInitialHeaders::new(too_many),
            Err(SipInitialHeadersError::TooManyHeaders)
        );
        assert_eq!(
            SipInitialHeaders::new([(format!("X{}", "n".repeat(128)), String::from("v"))]),
            Err(SipInitialHeadersError::NameTooLarge)
        );
        assert_eq!(
            SipInitialHeaders::new([("X-Test".to_string(), "v".repeat(4097))]),
            Err(SipInitialHeadersError::ValueTooLarge)
        );
        let aggregate = (0..5)
            .map(|index| (format!("X-Aggregate-{index}"), "v".repeat(4090)))
            .collect::<Vec<_>>();
        assert_eq!(
            SipInitialHeaders::new(aggregate),
            Err(SipInitialHeadersError::AggregateTooLarge)
        );

        let consumed = std::cell::Cell::new(0usize);
        let unbounded = std::iter::from_fn(|| {
            let index = consumed.get();
            consumed.set(index + 1);
            Some((format!("X-Unbounded-{index}"), "v".to_string()))
        });
        assert_eq!(
            SipInitialHeaders::new(unbounded),
            Err(SipInitialHeadersError::TooManyHeaders)
        );
        assert_eq!(
            consumed.get(),
            MAX_SIP_INITIAL_HEADERS + 1,
            "validation must stop at the first over-limit entry"
        );
    }

    #[test]
    fn invalid_forbidden_and_internal_names_are_rejected_without_echo() {
        for name in [
            "",
            "Bad Header",
            "Bad\rName",
            "Bad\nName",
            "X-Ünicode",
            "X#Bad",
            "X$Bad",
            "X&Bad",
            "X^Bad",
            "X|Bad",
            "Via",
            "v",
            "From",
            "f",
            "To",
            "t",
            "Call-ID",
            "i",
            "Contact",
            "m",
            "Content-Length",
            "l",
            "Content-Type",
            "c",
            "Content-Encoding",
            "Route",
            "Record-Route",
            "Proxy-Require",
            "Authorization",
            "Proxy-Authorization",
            "WWW-Authenticate",
            "Proxy-Authenticate",
            "Authentication-Info",
            "Identity",
            "P-Asserted-Identity",
            "p-PrEfErReD-iDeNtItY",
            "Supported",
            "Require",
            "Unsupported",
            "Allow",
            "Allow-Events",
            "Session-Expires",
            "Min-SE",
            "RAck",
            "RSeq",
            "Event",
            "Subscription-State",
            "Refer-To",
            "Referred-By",
            "Replaces",
            "Join",
            "Target-Dialog",
            "Path",
            "Service-Route",
            "SIP-ETag",
            "SIP-If-Match",
            "Subject",
            "X-Rvoip",
            "X-Rvoip-Data-Label",
            "CoNnEcTiOn",
            "KEEP-ALIVE",
            "Proxy-Connection",
            "tE",
            "Trailer",
            "Transfer-Encoding",
            "uPgRaDe",
            "HOST",
        ] {
            assert!(
                SipInitialHeaders::new([(name, "secret")]).is_err(),
                "{name}"
            );
        }
        for value in [
            "line\rbreak",
            "line\nbreak",
            "nul\0value",
            "bell\u{7}value",
            "delete\u{7f}value",
        ] {
            assert_eq!(
                SipInitialHeaders::new([("X-Test", value)]),
                Err(SipInitialHeadersError::InvalidValue)
            );
        }

        let debug = format!(
            "{:?}",
            SipInitialHeaders::new([("X-Correlation-Id", "context-secret")]).unwrap()
        );
        assert!(!debug.contains("context-secret"));
        assert!(!debug.contains("X-Correlation-Id"));
        let error = SipInitialHeaders::new([("Via", "header-secret")]).unwrap_err();
        let formatted = format!("{error:?} {error}");
        assert!(!formatted.contains("Via"));
        assert!(!formatted.contains("header-secret"));
    }

    #[test]
    fn context_and_auth_debug_are_redacted() {
        let context = SipOriginateContext::new()
            .with_from_uri("sip:private-user@example.test")
            .unwrap()
            .with_auth(SipClientAuth::basic("auth-user", "auth-password"))
            .unwrap()
            .with_initial_headers(SipInitialHeaders::new([("X-Secret", "header-secret")]).unwrap());
        let debug = format!("{context:?}");
        for secret in [
            "private-user",
            "example.test",
            "auth-user",
            "auth-password",
            "header-secret",
        ] {
            assert!(!debug.contains(secret));
        }

        let auths = [
            SipClientAuth::bearer_token("bearer-secret"),
            SipClientAuth::digest("digest-user", "digest-password"),
            SipClientAuth::basic("basic-user", "basic-password"),
        ];
        for auth in &auths {
            let auth_debug = format!("{auth:?}");
            for secret in [
                "bearer-secret",
                "digest-user",
                "digest-password",
                "basic-user",
                "basic-password",
            ] {
                assert!(!auth_debug.contains(secret));
            }
        }
        let composite_debug = format!("{:?}", SipClientAuth::any(auths));
        for secret in ["bearer-secret", "digest-user", "basic-user"] {
            assert!(!composite_debug.contains(secret));
        }

        let credentials = crate::types::Credentials::new("credential-user", "credential-password")
            .with_realm("private-realm");
        let credentials_debug = format!("{credentials:?}");
        for secret in ["credential-user", "credential-password", "private-realm"] {
            assert!(!credentials_debug.contains(secret));
        }

        let selected = crate::auth::ClientAuthHeader {
            value: "Bearer selected-header-secret".to_string(),
            scheme: crate::auth::SipAuthScheme::Bearer,
            digest_challenge: None,
            stale: false,
        };
        assert!(!format!("{selected:?}").contains("selected-header-secret"));
    }

    #[test]
    fn auth_admission_bounds_structure_strings_and_aggregate() {
        for auth in [
            SipClientAuth::digest("", "password"),
            SipClientAuth::digest("username", ""),
            SipClientAuth::basic("", "password"),
            SipClientAuth::basic("user:name", "password"),
            SipClientAuth::basic("username", ""),
            SipClientAuth::bearer_token(""),
            SipClientAuth::bearer_token("token\r\ninjected"),
        ] {
            assert_eq!(
                SipOriginateContext::new().with_auth(auth).unwrap_err(),
                SipOriginateContextError::InvalidAuthMaterial
            );
        }

        assert_eq!(
            SipOriginateContext::new()
                .with_auth(SipClientAuth::digest(
                    "u".repeat(MAX_SIP_ORIGINATE_AUTH_USERNAME_BYTES + 1),
                    "password",
                ))
                .unwrap_err(),
            SipOriginateContextError::AuthUsernameTooLarge
        );
        assert_eq!(
            SipOriginateContext::new()
                .with_auth(SipClientAuth::basic(
                    "username",
                    "p".repeat(MAX_SIP_ORIGINATE_AUTH_PASSWORD_BYTES + 1),
                ))
                .unwrap_err(),
            SipOriginateContextError::AuthPasswordTooLarge
        );
        assert_eq!(
            SipOriginateContext::new()
                .with_auth(SipClientAuth::Digest(
                    crate::types::Credentials::new("username", "password")
                        .with_realm("r".repeat(MAX_SIP_ORIGINATE_AUTH_REALM_BYTES + 1)),
                ))
                .unwrap_err(),
            SipOriginateContextError::AuthRealmTooLarge
        );
        assert_eq!(
            SipOriginateContext::new()
                .with_auth(SipClientAuth::bearer_token(
                    "t".repeat(MAX_SIP_ORIGINATE_BEARER_TOKEN_BYTES + 1)
                ))
                .unwrap_err(),
            SipOriginateContextError::BearerTokenTooLarge
        );

        let too_many = (0..=MAX_SIP_ORIGINATE_AUTH_OPTIONS)
            .map(|index| SipClientAuth::bearer_token(format!("token-{index}")));
        assert_eq!(
            SipOriginateContext::new()
                .with_auth(SipClientAuth::any(too_many))
                .unwrap_err(),
            SipOriginateContextError::TooManyAuthOptions
        );
        assert_eq!(
            SipOriginateContext::new()
                .with_auth(SipClientAuth::any([SipClientAuth::any([
                    SipClientAuth::bearer_token("nested-token"),
                ])]))
                .unwrap_err(),
            SipOriginateContextError::NestedAuthOptions
        );

        let aggregate = (0..5).map(|index| {
            SipClientAuth::bearer_token(format!(
                "{index}{}",
                "t".repeat(MAX_SIP_ORIGINATE_BEARER_TOKEN_BYTES - 1)
            ))
        });
        assert_eq!(
            SipOriginateContext::new()
                .with_auth(SipClientAuth::any(aggregate))
                .unwrap_err(),
            SipOriginateContextError::AuthAggregateTooLarge
        );

        SipOriginateContext::new()
            .with_auth(SipClientAuth::any((0..MAX_SIP_ORIGINATE_AUTH_OPTIONS).map(
                |index| SipClientAuth::bearer_token(format!("token-{index}")),
            )))
            .expect("the bounded non-nested maximum is valid")
            .validate()
            .expect("retained authentication revalidates at admission");
    }

    #[test]
    fn from_uri_admission_is_bounded_and_redacted() {
        let secret = format!(
            "sip:{}@example.test",
            "s".repeat(MAX_SIP_ORIGINATE_FROM_URI_BYTES)
        );
        let error = SipOriginateContext::new()
            .with_from_uri(secret.clone())
            .unwrap_err();
        assert_eq!(error, SipOriginateContextError::FromUriTooLarge);
        assert!(!format!("{error:?} {error}").contains(&secret));
    }
}
