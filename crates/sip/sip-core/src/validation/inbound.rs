//! Validation of a parsed inbound request before it is processed.
//!
//! Parsing accepts every request that is syntactically valid; these checks
//! decide whether an element processes it or answers with an error, as RFC
//! 3261 §8.2 and §16.3 and the RFC 4475 torture tests require. Two levels:
//!
//! - [`validate_request`]: what every element (UAS or proxy) enforces before
//!   transaction processing: protocol version, the mandatory header fields,
//!   single-valued header fields, the CSeq method, and the Via branch.
//! - [`recover_request_for_rejection`]: for a request that does not parse,
//!   the header fields needed to answer it with 400.
//! - [`validate_request_for_uas`]: UAS request processing in RFC 3261 §8.2
//!   order: method (405/501), Request-URI scheme (416), Require (420), and the
//!   body (415, RFC 5621), plus the Accept header of an INVITE (406).

use crate::parser::headers::accept::AcceptValue;
use crate::parser::message::ParseMode;
use crate::types::header::HeaderName;
use crate::types::headers::TypedHeader;
use crate::types::uri::Scheme;
use crate::types::{Method, Request, StatusCode};

/// The magic cookie that starts an RFC 3261 branch (RFC 3261 §8.1.1.7).
const MAGIC_COOKIE: &str = "z9hG4bK";

/// Header fields a request must contain (RFC 3261 §8.1.1), and that every
/// response copies from it (RFC 3261 §8.2.6.2). Max-Forwards is also required
/// of RFC 3261 senders, but RFC 2543 requests omit it and are still accepted
/// (RFC 4475 §3.4.1).
const REQUIRED_HEADERS: &[(HeaderName, &str)] = &[
    (HeaderName::Via, "Via"),
    (HeaderName::From, "From"),
    (HeaderName::To, "To"),
    (HeaderName::CallId, "Call-ID"),
    (HeaderName::CSeq, "CSeq"),
];

/// Header fields that may appear at most once (their grammar has no comma
/// separated list; RFC 3261 §7.3.1).
const SINGLE_VALUED_HEADERS: &[(HeaderName, &str)] = &[
    (HeaderName::From, "From"),
    (HeaderName::To, "To"),
    (HeaderName::CallId, "Call-ID"),
    (HeaderName::CSeq, "CSeq"),
    (HeaderName::MaxForwards, "Max-Forwards"),
    (HeaderName::ContentLength, "Content-Length"),
    (HeaderName::ContentType, "Content-Type"),
    (HeaderName::Expires, "Expires"),
];

/// Why a request is not processed, and the response to send instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Rejection {
    /// Status code of the response.
    pub status: StatusCode,
    /// Reason, for diagnostics (not a header field value).
    pub reason: &'static str,
    /// Header fields the response must carry (Allow, Unsupported, Accept).
    pub headers: Vec<TypedHeader>,
}

impl Rejection {
    fn new(status: StatusCode, reason: &'static str) -> Self {
        Self {
            status,
            reason,
            headers: Vec::new(),
        }
    }

    fn with_header(mut self, header: TypedHeader) -> Self {
        self.headers.push(header);
        self
    }
}

/// What a UAS implements, for [`validate_request_for_uas`]. The same lists
/// are advertised in Allow, Supported and Accept (RFC 3261 §11.2), so they
/// must name only what the UAS really processes: an option tag listed here
/// is one the UAS implements, and a request that requires any other gets 420
/// (RFC 3261 §8.2.2.3).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct UasCapabilities {
    /// Methods the UAS processes (the Allow header of a 405 or 501).
    pub methods: Vec<Method>,
    /// Option tags the UAS supports (checked against Require).
    pub supported_extensions: Vec<String>,
    /// Request-URI schemes the UAS accepts.
    pub uri_schemes: Vec<Scheme>,
    /// Media types the UAS processes in INVITE and UPDATE bodies, as
    /// `type/subtype` (the Accept header of a 415).
    pub session_body_types: Vec<String>,
}

impl Default for UasCapabilities {
    fn default() -> Self {
        Self {
            methods: vec![
                Method::Invite,
                Method::Ack,
                Method::Cancel,
                Method::Bye,
                Method::Options,
                Method::Register,
                Method::Info,
                Method::Message,
                Method::Notify,
                Method::Subscribe,
                Method::Refer,
                Method::Update,
                Method::Prack,
            ],
            // RFC 3262, RFC 4028, RFC 3327 (registrar Path echo), RFC 5626.
            // Not `replaces`: RFC 3891 call replacement is not implemented.
            supported_extensions: ["100rel", "timer", "path", "outbound"]
                .map(str::to_string)
                .to_vec(),
            uri_schemes: vec![Scheme::Sip, Scheme::Sips],
            session_body_types: vec!["application/sdp".to_string()],
        }
    }
}

impl UasCapabilities {
    /// `Allow` listing [`Self::methods`].
    pub fn allow_header(&self) -> TypedHeader {
        TypedHeader::Allow(crate::types::Allow(self.methods.clone()))
    }

    /// `Supported` listing [`Self::supported_extensions`].
    pub fn supported_header(&self) -> TypedHeader {
        TypedHeader::Supported(crate::types::Supported::new(
            self.supported_extensions.clone(),
        ))
    }

    /// `Accept` listing [`Self::session_body_types`].
    pub fn accept_header(&self) -> TypedHeader {
        TypedHeader::Accept(crate::types::Accept::from_media_types(
            self.session_body_types
                .iter()
                .filter_map(|media_type| accept_value(media_type)),
        ))
    }
}

/// Whether a response can be sent to `request`: it has the Via, From, To,
/// Call-ID and CSeq header fields every response copies from the request (RFC
/// 3261 §8.2.6.2, Table 2). An element discards a request without them, as
/// no valid response to it exists; it must not break (RFC 4475 §3.3.1).
pub fn is_answerable(request: &Request) -> bool {
    let headers = request.all_headers();
    REQUIRED_HEADERS
        .iter()
        .all(|(name, _)| headers.iter().any(|h| h.name().wire_eq(name)))
}

/// Checks every element applies before processing a request that
/// [`is_answerable`].
///
/// - SIP-Version other than 2.0: 505 (RFC 3261 §8.2.6, RFC 4475 §3.1.2.16).
/// - More than one From, To, Call-ID, CSeq, Max-Forwards, ... : 400 (RFC
///   4475 §3.3.8).
/// - CSeq method different from the request method: 400, or 501 when the
///   method is an extension method (RFC 3261 §8.1.1.5, RFC 4475 §3.1.2.17,
///   §3.1.2.18).
/// - A branch that is only the magic cookie: 400 (RFC 4475 §3.2.1). A branch
///   without the cookie, or no branch at all, is an RFC 2543 request and is
///   matched with the RFC 2543 rules (RFC 3261 §17.2.3).
pub fn validate_request(request: &Request) -> Result<(), Rejection> {
    if (request.version.major, request.version.minor) != (2, 0) {
        return Err(Rejection::new(
            StatusCode::VersionNotSupported,
            "SIP-Version is not SIP/2.0",
        ));
    }

    let headers = request.all_headers();
    let count = |name: &HeaderName| headers.iter().filter(|h| h.name().wire_eq(name)).count();
    for (name, _) in SINGLE_VALUED_HEADERS {
        if count(name) > 1 {
            return Err(Rejection::new(
                StatusCode::BadRequest,
                "a single-valued header field appears more than once",
            ));
        }
    }

    let cseq_method = headers.iter().find_map(|h| match h {
        TypedHeader::CSeq(cseq) => Some(cseq.method().clone()),
        _ => None,
    });
    if let Some(cseq_method) = cseq_method {
        let method = request.method();
        if cseq_method != method {
            return Err(match method {
                Method::Extension(_) => Rejection::new(
                    StatusCode::NotImplemented,
                    "unknown method with a mismatched CSeq method",
                ),
                _ => Rejection::new(
                    StatusCode::BadRequest,
                    "CSeq method does not match the request method",
                ),
            });
        }
    }

    if request
        .first_via()
        .and_then(|via| via.branch().map(str::to_owned))
        .as_deref()
        == Some(MAGIC_COOKIE)
    {
        return Err(Rejection::new(
            StatusCode::BadRequest,
            "the Via branch has no transaction identifier",
        ));
    }
    Ok(())
}

/// UAS request processing (RFC 3261 §8.2), after [`validate_request`].
///
/// 1. Method (§8.2.1): a method the UAS recognizes but does not support gets
///    405, and one it does not recognize gets 501 (§21.5.2); both carry an
///    Allow header field listing the supported methods.
/// 2. Request-URI (§8.2.2.1): an unsupported scheme gets 416.
/// 3. Require (§8.2.2.3): unsupported option tags get 420 with an Unsupported
///    header field listing them. Not applied to ACK or CANCEL.
/// 4. Content (§8.2.3, RFC 5621): an INVITE or UPDATE body of a type the UAS
///    does not process, whose disposition handling is "required" (the
///    default, RFC 3261 erratum 4567), gets 415 with an Accept header field.
/// 5. An INVITE whose Accept header field excludes every session body type
///    gets 406 (RFC 4475 §3.3.15).
pub fn validate_request_for_uas(
    request: &Request,
    capabilities: &UasCapabilities,
) -> Result<(), Rejection> {
    let method = request.method();
    let headers = request.all_headers();

    if !capabilities.methods.contains(&method) {
        let rejection = match method {
            Method::Extension(_) => {
                Rejection::new(StatusCode::NotImplemented, "method not recognized")
            }
            _ => Rejection::new(StatusCode::MethodNotAllowed, "method not supported"),
        };
        return Err(rejection.with_header(capabilities.allow_header()));
    }

    if !capabilities.uri_schemes.contains(&request.uri().scheme) {
        return Err(Rejection::new(
            StatusCode::UnsupportedUriScheme,
            "Request-URI scheme not supported",
        ));
    }

    if !matches!(method, Method::Ack | Method::Cancel) {
        let unsupported: Vec<String> = headers
            .iter()
            .filter_map(|h| match h {
                TypedHeader::Require(require) => Some(require.option_tags.clone()),
                _ => None,
            })
            .flatten()
            .filter(|tag| {
                !capabilities
                    .supported_extensions
                    .iter()
                    .any(|supported| supported.eq_ignore_ascii_case(tag))
            })
            .collect();
        if !unsupported.is_empty() {
            return Err(Rejection::new(
                StatusCode::BadExtension,
                "required extension not supported",
            )
            .with_header(TypedHeader::Unsupported(
                crate::types::Unsupported::with_tags(unsupported),
            )));
        }
    }

    if matches!(method, Method::Invite | Method::Update) && !request.body().is_empty() {
        let required = !headers.iter().any(|h| match h {
            TypedHeader::ContentDisposition(disposition) => disposition
                .to_string()
                .to_ascii_lowercase()
                .replace(' ', "")
                .contains("handling=optional"),
            _ => false,
        });
        let content_type = headers.iter().find_map(|h| match h {
            TypedHeader::ContentType(content_type) => Some(format!(
                "{}/{}",
                content_type.0.m_type, content_type.0.m_subtype
            )),
            _ => None,
        });
        let understood = content_type.as_deref().is_some_and(|media_type| {
            media_type.eq_ignore_ascii_case("multipart/mixed")
                || media_type.eq_ignore_ascii_case("multipart/alternative")
                || capabilities
                    .session_body_types
                    .iter()
                    .any(|accepted| accepted.eq_ignore_ascii_case(media_type))
        });
        if required && !understood {
            return Err(Rejection::new(
                StatusCode::UnsupportedMediaType,
                "message body type not supported",
            )
            .with_header(capabilities.accept_header()));
        }
    }

    if method == Method::Invite {
        let accepts: Vec<&AcceptValue> = headers
            .iter()
            .filter_map(|h| match h {
                TypedHeader::Accept(accept) => Some(accept.0.iter()),
                _ => None,
            })
            .flatten()
            .collect();
        let has_accept = headers.iter().any(|h| h.name() == HeaderName::Accept);
        let acceptable = accepts.iter().any(|value| {
            capabilities.session_body_types.iter().any(|offered| {
                let (typ, subtype) = offered.split_once('/').unwrap_or((offered, ""));
                (value.m_type == "*" || value.m_type.eq_ignore_ascii_case(typ))
                    && (value.m_subtype == "*" || value.m_subtype.eq_ignore_ascii_case(subtype))
            })
        });
        // An empty Accept header field means no body is acceptable; a missing
        // one means application/sdp (RFC 3261 §20.1).
        if has_accept && !acceptable {
            return Err(Rejection::new(
                StatusCode::NotAcceptable,
                "Accept excludes every session description type",
            ));
        }
    }
    Ok(())
}

fn accept_value(media_type: &str) -> Option<AcceptValue> {
    let (m_type, m_subtype) = media_type.split_once('/')?;
    Some(AcceptValue {
        m_type: m_type.to_string(),
        m_subtype: m_subtype.to_string(),
        q: None,
        params: Default::default(),
    })
}

/// For a request that does not parse: the request reduced to what a 400 (Bad
/// Request) response is built from (RFC 3261 §8.2.6: Via, From, To, Call-ID
/// and CSeq), so the element can answer it as RFC 4475 §3.1.2 asks.
///
/// `None` when `input` is a response or has no request start line, or when
/// one of those header fields is missing, repeated, or itself malformed: no
/// valid response can be formed then, and the message is discarded (RFC 3261
/// §18.3).
pub fn recover_request_for_rejection(input: &[u8]) -> Option<Request> {
    let text = String::from_utf8_lossy(input);
    let header_end = ["\r\n\r\n", "\n\n"]
        .iter()
        .filter_map(|terminator| text.find(terminator))
        .min()
        .unwrap_or(text.len());
    let mut lines = text[..header_end].lines();
    let start_line = lines.next()?.trim();
    let method = start_line.split(' ').next()?;
    let is_token = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-.!%*_+`'~".contains(&b))
    };
    if start_line.starts_with("SIP/") || !is_token(method) {
        return None;
    }

    // Unfold continuation lines, then keep the header fields a response
    // copies (RFC 3261 §7.3.1 compact forms included).
    let mut fields: Vec<String> = Vec::new();
    for line in lines {
        let line = line.trim_end_matches('\r');
        if line.starts_with([' ', '\t']) {
            if let Some(last) = fields.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            }
            continue;
        }
        fields.push(line.to_string());
    }
    let wanted = |name: &str| {
        matches!(
            name.trim().to_ascii_lowercase().as_str(),
            "via" | "v" | "from" | "f" | "to" | "t" | "call-id" | "i" | "cseq"
        )
    };
    let mut reduced = format!("{method} sip:invalid.invalid SIP/2.0\r\n");
    for field in fields
        .iter()
        .filter(|field| field.split_once(':').is_some_and(|(name, _)| wanted(name)))
    {
        reduced.push_str(field);
        reduced.push_str("\r\n");
    }
    reduced.push_str("Content-Length: 0\r\n\r\n");

    let Ok(crate::types::Message::Request(request)) =
        crate::parse_message_with_mode(reduced.as_bytes(), ParseMode::Lenient)
    else {
        return None;
    };
    let headers = request.all_headers();
    let count = |name: &HeaderName| headers.iter().filter(|h| h.name().wire_eq(name)).count();
    let complete = count(&HeaderName::Via) >= 1
        && [
            HeaderName::From,
            HeaderName::To,
            HeaderName::CallId,
            HeaderName::CSeq,
        ]
        .iter()
        .all(|name| count(name) == 1);
    complete.then_some(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_message_with_mode;
    use crate::types::Message;

    fn options(extra: &str) -> Request {
        let text = format!(
            "OPTIONS sip:uas@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP client.example.com;branch=z9hG4bK-caps\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:uac@example.com>;tag=1\r\n\
             To: <sip:uas@example.com>\r\n\
             Call-ID: caps@example.com\r\n\
             CSeq: 1 OPTIONS\r\n\
             {extra}\
             Content-Length: 0\r\n\r\n"
        );
        match parse_message_with_mode(text.as_bytes(), ParseMode::Strict).unwrap() {
            Message::Request(request) => request,
            _ => unreachable!(),
        }
    }

    #[test]
    fn capabilities_round_trip_through_serde_and_default_missing_fields() {
        let mut capabilities = UasCapabilities::default();
        capabilities.supported_extensions.push("replaces".into());
        let json = serde_json::to_string(&capabilities).unwrap();
        assert_eq!(
            serde_json::from_str::<UasCapabilities>(&json).unwrap(),
            capabilities
        );
        // A configuration naming only some fields keeps the defaults for the rest.
        let partial: UasCapabilities =
            serde_json::from_str(r#"{"supported_extensions":["timer"]}"#).unwrap();
        assert_eq!(partial.supported_extensions, ["timer"]);
        assert_eq!(partial.methods, UasCapabilities::default().methods);
    }

    #[test]
    fn advertised_headers_list_the_capabilities() {
        let capabilities = UasCapabilities::default();
        let allow = capabilities.allow_header().to_string();
        assert!(
            allow.starts_with("Allow:") && allow.contains("PRACK"),
            "{allow}"
        );
        let supported = capabilities.supported_header().to_string();
        assert!(supported.starts_with("Supported:"), "{supported}");
        assert!(supported.contains("100rel") && !supported.contains("replaces"));
        let accept = capabilities.accept_header().to_string();
        assert!(accept.starts_with("Accept:") && accept.contains("application/sdp"));
    }

    /// RFC 3891 call replacement is not implemented, so a request that
    /// requires it gets 420 (RFC 3261 §8.2.2.3) unless an application adds
    /// the option tag.
    #[test]
    fn replaces_is_supported_only_when_configured() {
        let request = options("Require: replaces\r\n");
        let rejection =
            validate_request_for_uas(&request, &UasCapabilities::default()).unwrap_err();
        assert_eq!(rejection.status, StatusCode::BadExtension);

        let mut capabilities = UasCapabilities::default();
        capabilities.supported_extensions.push("replaces".into());
        assert!(validate_request_for_uas(&request, &capabilities).is_ok());
    }
}
