//! Wire-readiness validation for outbound SIP messages.
//!
//! These checks cover the RFC 3261 framing/header requirements that must be
//! true before a message is serialized and handed to transport.

use crate::types::{
    headers::{HeaderName, HeaderValue, TypedHeader},
    sip_request::Request,
    sip_response::Response,
    Method,
};
use crate::{Error, Result};

/// Maximum encoded size of an outbound `Authorization` or
/// `Proxy-Authorization` header value.
///
/// The bound is deliberately large enough for existing Digest, Basic, Bearer,
/// and IMS AKA deployments while preventing an authentication provider or
/// precomputed-value caller from creating an unbounded SIP header.
pub const MAX_AUTHORIZATION_HEADER_VALUE_BYTES: usize = 16 * 1024;

fn validation_error(message: impl Into<String>) -> Error {
    Error::ValidationError(message.into())
}

fn has_header(headers: &[TypedHeader], name: HeaderName) -> bool {
    headers.iter().any(|h| h.name() == name)
}

fn header_count(headers: &[TypedHeader], name: HeaderName) -> usize {
    headers.iter().filter(|h| h.name() == name).count()
}

fn is_authorization_header_name(name: &HeaderName) -> bool {
    name.as_str().eq_ignore_ascii_case("Authorization")
        || name.as_str().eq_ignore_ascii_case("Proxy-Authorization")
}

/// Validate one outbound `Authorization` or `Proxy-Authorization` value.
///
/// Error messages never include the supplied value. All Unicode control
/// characters are rejected, which includes CR, LF, NUL, HTAB, and the C1
/// control range. This keeps generated and precomputed credentials confined to
/// one SIP header line before serialization.
///
/// # Errors
///
/// Returns a secret-free validation error when the value is empty, exceeds
/// [`MAX_AUTHORIZATION_HEADER_VALUE_BYTES`], or contains a control character.
pub fn validate_authorization_header_value(value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(validation_error(
            "SIP authorization header value must not be empty",
        ));
    }
    if value.len() > MAX_AUTHORIZATION_HEADER_VALUE_BYTES {
        return Err(validation_error(
            "SIP authorization header value exceeds the size limit",
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(validation_error(
            "SIP authorization header value contains a forbidden control character",
        ));
    }
    Ok(())
}

/// Construct a validated raw authorization header without exposing its value
/// in diagnostics.
///
/// Only `Authorization` and `Proxy-Authorization` names are accepted. Callers
/// should use this helper at the final generated/precomputed-value insertion
/// boundary instead of constructing `HeaderValue::Raw` directly.
///
/// # Errors
///
/// Returns a secret-free validation error for a different header name or any
/// value rejected by [`validate_authorization_header_value`].
pub fn validated_authorization_header(
    name: HeaderName,
    value: impl Into<String>,
) -> Result<TypedHeader> {
    if !is_authorization_header_name(&name) {
        return Err(validation_error(
            "validated SIP authorization value used with a non-authorization header",
        ));
    }
    let value = value.into();
    validate_authorization_header_value(&value)?;
    Ok(TypedHeader::Other(
        name,
        HeaderValue::Raw(value.into_bytes()),
    ))
}

fn validate_authorization_headers(headers: &[TypedHeader]) -> Result<()> {
    for header in headers {
        match header {
            TypedHeader::Authorization(value) => {
                validate_authorization_header_value(&value.to_string())?;
            }
            TypedHeader::ProxyAuthorization(value) => {
                validate_authorization_header_value(&value.to_string())?;
            }
            TypedHeader::Other(name, value) if is_authorization_header_name(name) => match value {
                HeaderValue::Raw(bytes) => {
                    let value = std::str::from_utf8(bytes).map_err(|_| {
                        validation_error("SIP authorization header value is not valid UTF-8")
                    })?;
                    validate_authorization_header_value(value)?;
                }
                HeaderValue::Authorization(value) => {
                    validate_authorization_header_value(&value.to_string())?;
                }
                HeaderValue::ProxyAuthorization(value) => {
                    validate_authorization_header_value(&value.to_string())?;
                }
                _ => {
                    return Err(validation_error(
                        "SIP authorization header has an unsupported value type",
                    ));
                }
            },
            _ => {}
        }
    }
    Ok(())
}

fn content_length_value(headers: &[TypedHeader]) -> Result<Option<u32>> {
    headers
        .iter()
        .rev()
        .find_map(|h| match h {
            TypedHeader::ContentLength(content_length) => Some(Ok(content_length.0)),
            TypedHeader::Other(name, HeaderValue::ContentLength(content_length))
                if *name == HeaderName::ContentLength =>
            {
                Some(Ok(content_length.0))
            }
            TypedHeader::Other(name, HeaderValue::Raw(raw))
                if *name == HeaderName::ContentLength =>
            {
                let value = std::str::from_utf8(raw).map_err(|_| {
                    validation_error("SIP message Content-Length header is not valid UTF-8")
                });
                Some(value.and_then(|value| {
                    value.trim().parse::<u32>().map_err(|_| {
                        validation_error(format!(
                            "SIP message Content-Length header is not a valid integer: {}",
                            value.trim()
                        ))
                    })
                }))
            }
            _ if h.name() == HeaderName::ContentLength => Some(Err(validation_error(
                "SIP message Content-Length header has unsupported value type",
            ))),
            _ => None,
        })
        .transpose()
}

/// Validate that a SIP message's Content-Length header is present and matches
/// the body length in bytes.
pub fn validate_content_length(headers: &[TypedHeader], body_len: usize) -> Result<()> {
    let Some(content_length) = content_length_value(headers)? else {
        return Err(validation_error(
            "SIP message missing Content-Length header",
        ));
    };

    if content_length as usize != body_len {
        return Err(validation_error(format!(
            "SIP message Content-Length mismatch: header={}, body={}",
            content_length, body_len
        )));
    }

    Ok(())
}

/// Validate that an outbound SIP request is ready to be serialized onto the
/// wire.
pub fn validate_wire_request(request: &Request) -> Result<()> {
    let headers = &request.headers;

    validate_authorization_headers(headers)?;

    for (name, label) in [
        (HeaderName::Via, "Via"),
        (HeaderName::From, "From"),
        (HeaderName::To, "To"),
        (HeaderName::CallId, "Call-ID"),
        (HeaderName::CSeq, "CSeq"),
        (HeaderName::MaxForwards, "Max-Forwards"),
    ] {
        if !has_header(headers, name) {
            return Err(validation_error(format!(
                "{} request missing {} header",
                request.method, label
            )));
        }
    }

    validate_content_length(headers, request.body.len())?;

    if !request.body.is_empty() && !has_header(headers, HeaderName::ContentType) {
        return Err(validation_error(format!(
            "{} request with body missing Content-Type header",
            request.method
        )));
    }

    if matches!(request.method, Method::Invite | Method::Refer) {
        let contact_count = header_count(headers, HeaderName::Contact);
        if contact_count != 1 {
            return Err(validation_error(format!(
                "{} request must contain exactly one Contact header, found {}",
                request.method, contact_count
            )));
        }
    }

    if request.method == Method::Refer && !has_header(headers, HeaderName::ReferTo) {
        return Err(validation_error("REFER request missing Refer-To header"));
    }

    if request.method == Method::Subscribe && !has_header(headers, HeaderName::Event) {
        return Err(validation_error("SUBSCRIBE request missing Event header"));
    }

    if request.method == Method::Notify {
        if !has_header(headers, HeaderName::Event) {
            return Err(validation_error("NOTIFY request missing Event header"));
        }
        if !has_header(headers, HeaderName::SubscriptionState) {
            return Err(validation_error(
                "NOTIFY request missing Subscription-State header",
            ));
        }
    }

    if matches!(
        request.method,
        Method::Update | Method::Subscribe | Method::Notify
    ) && !has_header(headers, HeaderName::Contact)
    {
        tracing::warn!(
            method = %request.method,
            "target-refresh request missing recommended Contact header"
        );
    }

    Ok(())
}

/// Validate that an outbound SIP response is ready to be serialized onto the
/// wire.
pub fn validate_wire_response(response: &Response) -> Result<()> {
    let headers = &response.headers;

    for (name, label) in [
        (HeaderName::Via, "Via"),
        (HeaderName::From, "From"),
        (HeaderName::To, "To"),
        (HeaderName::CallId, "Call-ID"),
        (HeaderName::CSeq, "CSeq"),
    ] {
        if !has_header(headers, name) {
            return Err(validation_error(format!(
                "{} response missing {} header",
                response.status_code(),
                label
            )));
        }
    }

    validate_content_length(headers, response.body.len())?;

    if !response.body.is_empty() && !has_header(headers, HeaderName::ContentType) {
        return Err(validation_error(format!(
            "{} response with body missing Content-Type header",
            response.status_code()
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use crate::types::{ContentLength, ReferTo, StatusCode};
    use std::str::FromStr;

    fn valid_request() -> Request {
        SimpleRequestBuilder::register("sip:registrar.example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag-1"))
            .to("Alice", "sip:alice@example.com", None)
            .call_id("reg-call-1")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-test"))
            .max_forwards(70)
            .contact("sip:alice@127.0.0.1:5060", None)
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build()
    }

    #[test]
    fn accepts_valid_empty_request() {
        assert!(validate_wire_request(&valid_request()).is_ok());
    }

    #[test]
    fn authorization_value_boundaries_and_controls_are_enforced_without_echo() {
        let exact = format!(
            "Bearer {}",
            "a".repeat(MAX_AUTHORIZATION_HEADER_VALUE_BYTES - "Bearer ".len())
        );
        validate_authorization_header_value(&exact).expect("exact boundary is valid");

        let oversized = format!("{exact}a");
        let oversized_error = validate_authorization_header_value(&oversized).unwrap_err();
        assert!(oversized_error.to_string().contains("size limit"));
        assert!(!oversized_error.to_string().contains(&oversized));

        for malicious in [
            "Bearer safe\r\nX-Injected: yes",
            "Bearer safe\nX-Injected: yes",
            "Bearer safe\0tail",
            "Bearer safe\ttail",
            "Bearer safe\u{85}tail",
        ] {
            let error = validate_authorization_header_value(malicious).unwrap_err();
            assert!(error.to_string().contains("control character"));
            assert!(!error.to_string().contains(malicious));
        }
        assert!(validate_authorization_header_value("").is_err());
        assert!(validate_authorization_header_value("   ").is_err());
    }

    #[test]
    fn validated_authorization_constructor_rejects_line_smuggling_for_both_names() {
        for name in [
            HeaderName::Authorization,
            HeaderName::ProxyAuthorization,
            HeaderName::Other("AUTHORIZATION".to_string()),
            HeaderName::Other("proxy-Authorization".to_string()),
        ] {
            assert!(validated_authorization_header(
                name.clone(),
                "Digest username=\"alice\"\r\nX-Injected: yes",
            )
            .is_err());

            let header = validated_authorization_header(
                name.clone(),
                "Digest username=\"alice\", response=\"safe\"",
            )
            .expect("valid authorization header");
            assert_eq!(header.name(), name);
        }

        assert!(validated_authorization_header(HeaderName::Via, "Digest safe").is_err());
    }

    #[test]
    fn wire_validation_rejects_raw_authorization_line_smuggling() {
        for name in [
            HeaderName::Authorization,
            HeaderName::ProxyAuthorization,
            HeaderName::Other("aUtHoRiZaTiOn".to_string()),
            HeaderName::Other("PROXY-authorization".to_string()),
        ] {
            let mut request = valid_request();
            request.headers.push(TypedHeader::Other(
                name,
                HeaderValue::Raw(b"Bearer safe\r\nX-Injected: yes".to_vec()),
            ));
            let error = validate_wire_request(&request).unwrap_err();
            assert!(error.to_string().contains("control character"));
            assert!(!error.to_string().contains("X-Injected"));
        }
    }

    #[test]
    fn accepts_register_query_without_contact() {
        let mut request = valid_request();
        request.headers.retain(|h| h.name() != HeaderName::Contact);
        assert!(validate_wire_request(&request).is_ok());
    }

    fn valid_refer_request() -> Request {
        SimpleRequestBuilder::new(Method::Refer, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag-1"))
            .to("Bob", "sip:bob@example.com", Some("tag-2"))
            .call_id("refer-call-1")
            .cseq(2)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-refer"))
            .max_forwards(70)
            .contact("sip:alice@127.0.0.1:5060", None)
            .header(TypedHeader::ReferTo(
                ReferTo::from_str("sip:carol@example.com").unwrap(),
            ))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build()
    }

    #[test]
    fn accepts_valid_refer_request() {
        assert!(validate_wire_request(&valid_refer_request()).is_ok());
    }

    #[test]
    fn rejects_refer_missing_contact() {
        let mut request = valid_refer_request();
        request.headers.retain(|h| h.name() != HeaderName::Contact);
        assert!(validate_wire_request(&request)
            .unwrap_err()
            .to_string()
            .contains("Contact"));
    }

    #[test]
    fn rejects_refer_missing_refer_to() {
        let mut request = valid_refer_request();
        request.headers.retain(|h| h.name() != HeaderName::ReferTo);
        assert!(validate_wire_request(&request)
            .unwrap_err()
            .to_string()
            .contains("Refer-To"));
    }

    #[test]
    fn rejects_missing_call_id() {
        let mut request = valid_request();
        request.headers.retain(|h| h.name() != HeaderName::CallId);
        assert!(validate_wire_request(&request)
            .unwrap_err()
            .to_string()
            .contains("Call-ID"));
    }

    #[test]
    fn rejects_missing_max_forwards() {
        let mut request = valid_request();
        request
            .headers
            .retain(|h| h.name() != HeaderName::MaxForwards);
        assert!(validate_wire_request(&request)
            .unwrap_err()
            .to_string()
            .contains("Max-Forwards"));
    }

    #[test]
    fn rejects_missing_content_length() {
        let mut request = valid_request();
        request
            .headers
            .retain(|h| h.name() != HeaderName::ContentLength);
        assert!(validate_wire_request(&request)
            .unwrap_err()
            .to_string()
            .contains("Content-Length"));
    }

    #[test]
    fn rejects_content_length_mismatch() {
        let mut request = valid_request();
        request.body = "hello".as_bytes().to_vec().into();
        assert!(validate_wire_request(&request)
            .unwrap_err()
            .to_string()
            .contains("mismatch"));
    }

    #[test]
    fn rejects_body_without_content_type() {
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag-1"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("message-call-1")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-test"))
            .max_forwards(70)
            .body("hello")
            .build();

        assert!(validate_wire_request(&request)
            .unwrap_err()
            .to_string()
            .contains("Content-Type"));
    }

    #[test]
    fn accepts_valid_body_request() {
        let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag-1"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("message-call-1")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-test"))
            .max_forwards(70)
            .content_type("text/plain")
            .body("hello")
            .build();

        assert!(validate_wire_request(&request).is_ok());
    }

    #[test]
    fn accepts_valid_response() {
        let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
            .from("Alice", "sip:alice@example.com", Some("tag-1"))
            .to("Bob", "sip:bob@example.com", Some("tag-2"))
            .call_id("call-1")
            .cseq(1, Method::Invite)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-test"))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();

        assert!(validate_wire_response(&response).is_ok());
    }

    #[test]
    fn accepts_raw_content_length_header() {
        let mut request = valid_request();
        request
            .headers
            .retain(|h| h.name() != HeaderName::ContentLength);
        request.headers.push(TypedHeader::Other(
            HeaderName::ContentLength,
            HeaderValue::Raw(b"0".to_vec()),
        ));

        assert!(validate_wire_request(&request).is_ok());
    }

    #[test]
    fn rejects_invalid_raw_content_length_header() {
        let mut request = valid_request();
        request
            .headers
            .retain(|h| h.name() != HeaderName::ContentLength);
        request.headers.push(TypedHeader::Other(
            HeaderName::ContentLength,
            HeaderValue::Raw(b"zero".to_vec()),
        ));

        assert!(validate_wire_request(&request)
            .unwrap_err()
            .to_string()
            .contains("valid integer"));
    }
}
