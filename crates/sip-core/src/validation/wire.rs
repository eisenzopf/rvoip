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

fn validation_error(message: impl Into<String>) -> Error {
    Error::ValidationError(message.into())
}

fn has_header(headers: &[TypedHeader], name: HeaderName) -> bool {
    headers.iter().any(|h| h.name() == name)
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

    if request.method == Method::Register && !has_header(headers, HeaderName::Contact) {
        return Err(validation_error("REGISTER request missing Contact header"));
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
    use crate::types::{ContentLength, StatusCode};

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
