//! Message bodies (RFC 5621): the structure of a body, the disposition and
//! handling of each body part, whether a UA understands a body, and which part
//! carries the session description.
//!
//! A body is one [`BodyPart`]. A `multipart/*` part holds its parts, split
//! strictly by RFC 2046 §5.1.1 and nested to [`MAX_NESTING`] levels. Content
//! stays as received bytes: binary parts such as `application/isup` are never
//! decoded as text.
//!
//! ```rust
//! use bytes::Bytes;
//! use rvoip_sip_core::body::BodyPart;
//! use rvoip_sip_core::types::content_type::ContentType;
//! use rvoip_sip_core::types::headers::TypedHeader;
//! use std::str::FromStr;
//!
//! // An INVITE body from a carrier: SDP plus an optional ISUP part.
//! let headers = [TypedHeader::ContentType(
//!     ContentType::from_str("multipart/mixed;boundary=b1").unwrap(),
//! )];
//! let body = Bytes::from_static(
//!     b"--b1\r\nContent-Type: application/sdp\r\n\r\nv=0\r\n\
//!       --b1\r\nContent-Type: application/isup\r\n\
//!       Content-Disposition: signal;handling=optional\r\n\r\n\x01\x02\r\n--b1--\r\n",
//! );
//! let body = BodyPart::of_message(&headers, &body).unwrap().unwrap();
//! assert_eq!(body.parts.len(), 2);
//! assert_eq!(body.session_description().as_deref(), Some(&b"v=0"[..]));
//! assert!(body.understood(&|media_type| media_type == "application/sdp"));
//! ```

use bytes::Bytes;
use std::str::FromStr;

use crate::parser::headers::content_type::ContentTypeValue;
use crate::types::content_disposition::{ContentDisposition, DispositionType};
use crate::types::content_type::ContentType;
use crate::types::headers::TypedHeader;
use crate::types::{Request, Response};

/// Deepest `multipart/*` nesting parsed; a deeper body is malformed.
pub const MAX_NESTING: usize = 8;

/// Media types a UA understands structurally (RFC 2046 §5.1, RFC 2387):
/// whether it understands one depends on its parts.
pub const MULTIPART_TYPES: [&str; 3] = [
    "multipart/mixed",
    "multipart/alternative",
    "multipart/related",
];

/// Why a body could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BodyError {
    /// A `multipart/*` body without a `boundary` parameter (RFC 2046 §5.1.1).
    #[error("multipart body has no boundary parameter")]
    MissingBoundary,
    /// A `multipart/*` body that does not follow RFC 2046 §5.1.1.
    #[error("malformed multipart body: {0}")]
    MalformedMultipart(&'static str),
    /// A body part header field that does not parse.
    #[error("malformed body part header field: {0}")]
    MalformedPartHeader(&'static str),
    /// Multipart bodies nested deeper than [`MAX_NESTING`].
    #[error("multipart bodies nested too deeply")]
    TooDeep,
}

/// A message body or one part of a multipart body (RFC 5621 §3).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyPart {
    /// The Content-Type, if the message or part has one.
    pub content_type: Option<ContentTypeValue>,
    /// The Content-Disposition, if the message or part has one.
    pub disposition: Option<ContentDisposition>,
    /// The Content-Transfer-Encoding of a body part, lowercased (RFC 2045
    /// §6). Message bodies have none in SIP (RFC 3261 §7.4).
    pub transfer_encoding: Option<String>,
    /// The content as received (for a multipart part, including its parts).
    pub content: Bytes,
    /// The parts of a `multipart/*` body, in order; empty otherwise.
    pub parts: Vec<BodyPart>,
    /// A body part's MIME header fields as received (unfolded), in order;
    /// empty for a message body, whose header fields are the message's.
    pub header_fields: Vec<(String, String)>,
    content_id: Option<String>,
}

impl BodyPart {
    /// The body of `request`, or `None` when it has none.
    pub fn of_request(request: &Request) -> Result<Option<Self>, BodyError> {
        Self::of_message(&request.headers, &request.body)
    }

    /// The body of `response`, or `None` when it has none.
    pub fn of_response(response: &Response) -> Result<Option<Self>, BodyError> {
        Self::of_message(&response.headers, &response.body)
    }

    /// The body of a message with header fields `headers` and body `body`.
    pub fn of_message(headers: &[TypedHeader], body: &Bytes) -> Result<Option<Self>, BodyError> {
        if body.is_empty() {
            return Ok(None);
        }
        let content_type = headers.iter().find_map(|header| match header {
            TypedHeader::ContentType(content_type) => Some(content_type.0.clone()),
            _ => None,
        });
        let disposition = headers.iter().find_map(|header| match header {
            TypedHeader::ContentDisposition(disposition) => Some(disposition.clone()),
            _ => None,
        });
        let part = Self {
            content_type,
            disposition,
            transfer_encoding: None,
            content: body.clone(),
            parts: Vec::new(),
            header_fields: Vec::new(),
            content_id: None,
        };
        part.with_parts(0).map(Some)
    }

    /// Parses the parts of a `multipart/*` body at nesting level `depth`.
    fn with_parts(mut self, depth: usize) -> Result<Self, BodyError> {
        if !self.is_multipart() || !self.identity_encoded() {
            return Ok(self);
        }
        if depth >= MAX_NESTING {
            return Err(BodyError::TooDeep);
        }
        let boundary = self
            .content_type
            .as_ref()
            .and_then(|content_type| content_type.parameters.get("boundary"))
            .ok_or(BodyError::MissingBoundary)?
            .clone();
        let content = self.content.clone();
        for raw in split_multipart(&content, &boundary)? {
            self.parts.push(parse_part(raw, depth + 1)?);
        }
        Ok(self)
    }

    /// The media type as `type/subtype`, lowercased. A body part without a
    /// Content-Type is `text/plain` (RFC 2045 §5.2); a message body without
    /// one has no media type.
    pub fn media_type(&self) -> Option<String> {
        self.content_type
            .as_ref()
            .map(|content_type| format!("{}/{}", content_type.m_type, content_type.m_subtype))
    }

    /// Whether this is a `multipart/*` body.
    pub fn is_multipart(&self) -> bool {
        self.content_type
            .as_ref()
            .is_some_and(|content_type| content_type.m_type == "multipart")
    }

    /// The disposition type: the Content-Disposition's, or the default of RFC
    /// 3261 §20.11: `session` for `application/sdp`, `render` otherwise.
    pub fn disposition_type(&self) -> DispositionType {
        match &self.disposition {
            Some(disposition) => disposition.disposition_type.clone(),
            None if self.media_type().as_deref() == Some("application/sdp") => {
                DispositionType::Session
            }
            None => DispositionType::Render,
        }
    }

    /// Whether the UA must understand this part to process the message: the
    /// `handling` parameter, `required` when absent (RFC 3261 §20.11, RFC
    /// 5621 §3.2).
    pub fn handling_required(&self) -> bool {
        !self.disposition.as_ref().is_some_and(|disposition| {
            disposition
                .params
                .get("handling")
                .is_some_and(|handling| handling.eq_ignore_ascii_case("optional"))
        })
    }

    /// Whether the content is the octets themselves: no Content-Transfer-
    /// Encoding, or `7bit`, `8bit` or `binary` (RFC 2045 §6.2). A UA that does
    /// not decode another encoding does not understand the part.
    pub fn identity_encoded(&self) -> bool {
        self.transfer_encoding
            .as_deref()
            .is_none_or(|encoding| matches!(encoding, "7bit" | "8bit" | "binary"))
    }

    /// Whether a UA that understands the media types for which
    /// `understands_media_type` returns true understands this body (RFC 5621
    /// §3.1):
    /// - `multipart/mixed`: every part whose handling is required;
    /// - `multipart/alternative`: at least one alternative (RFC 2046 §5.1.4);
    /// - `multipart/related`: the root part (RFC 2387 §3.2);
    /// - any other type: the media type itself, in an identity encoding.
    pub fn understood(&self, understands_media_type: &dyn Fn(&str) -> bool) -> bool {
        if !self.identity_encoded() {
            return false;
        }
        let Some(media_type) = self.media_type() else {
            return false;
        };
        match media_type.as_str() {
            "multipart/mixed" => self
                .parts
                .iter()
                .all(|part| !part.handling_required() || part.understood(understands_media_type)),
            "multipart/alternative" => self
                .parts
                .iter()
                .any(|part| part.understood(understands_media_type)),
            "multipart/related" => self
                .related_root()
                .is_some_and(|root| root.understood(understands_media_type)),
            other => understands_media_type(other),
        }
    }

    /// The session description (RFC 3264) this body carries: an
    /// `application/sdp` part whose disposition is `session`. In
    /// `multipart/mixed` the first one; in `multipart/alternative` the last
    /// alternative that has one (RFC 2046 §5.1.4: alternatives are in
    /// increasing order of preference); in `multipart/related` the root's.
    pub fn session_description(&self) -> Option<Bytes> {
        if !self.identity_encoded() {
            return None;
        }
        match self.media_type().as_deref() {
            Some("multipart/mixed") => self.parts.iter().find_map(Self::session_description),
            Some("multipart/alternative") => {
                self.parts.iter().rev().find_map(Self::session_description)
            }
            Some("multipart/related") => self.related_root()?.session_description(),
            Some("application/sdp")
                if matches!(self.disposition_type(), DispositionType::Session) =>
            {
                Some(self.content.clone())
            }
            _ => None,
        }
    }

    /// The root of a `multipart/related` body: the part whose Content-ID the
    /// `start` parameter names, or the first part (RFC 2387 §3.2).
    fn related_root(&self) -> Option<&BodyPart> {
        let start = self
            .content_type
            .as_ref()
            .and_then(|content_type| content_type.parameters.get("start"));
        match start {
            Some(start) => self
                .parts
                .iter()
                .find(|part| part.content_id.as_deref() == Some(start.as_str())),
            None => self.parts.first(),
        }
    }
}

/// The session description of a message: from its body (see
/// [`BodyPart::session_description`]), or, for a body without the
/// Content-Type RFC 3261 §20.15 requires, the whole body when it is an SDP
/// session description (it starts with `v=`, RFC 8866 §5).
pub fn session_description(headers: &[TypedHeader], body: &Bytes) -> Option<Bytes> {
    let part = BodyPart::of_message(headers, body).ok()??;
    if part.content_type.is_none() {
        return body.starts_with(b"v=").then(|| body.clone());
    }
    part.session_description()
}

/// [`session_description`] as text, for the SDP parsers.
pub fn session_description_text(headers: &[TypedHeader], body: &Bytes) -> Option<String> {
    session_description(headers, body).and_then(|sdp| String::from_utf8(sdp.to_vec()).ok())
}

/// Splits a multipart body into its body parts (headers and content) by the
/// delimiters of RFC 2046 §5.1.1. The preamble and epilogue are discarded.
/// The close delimiter is required.
fn split_multipart<'a>(body: &'a [u8], boundary: &str) -> Result<Vec<&'a [u8]>, BodyError> {
    if boundary.is_empty() || boundary.len() > 70 {
        return Err(BodyError::MalformedMultipart(
            "boundary is not 1 to 70 characters",
        ));
    }
    let dash_boundary = [b"--", boundary.as_bytes()].concat();
    let mut parts = Vec::new();
    // The first delimiter may start the body; every later one follows CRLF,
    // which belongs to the delimiter.
    let mut search = 0;
    let mut part_start: Option<usize> = None;
    loop {
        let Some(found) = find_delimiter(body, &dash_boundary, search) else {
            return Err(BodyError::MalformedMultipart("no close delimiter"));
        };
        let line_start = found;
        let after = found + dash_boundary.len();
        let is_close = body[after..].starts_with(b"--");
        let rest = if is_close { after + 2 } else { after };
        // transport-padding (LWSP) then CRLF; the close delimiter may also
        // end the body.
        let padding = body[rest..]
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let line_end = rest + padding;
        let ends_line = body[line_end..].starts_with(b"\r\n");
        if !ends_line && !(is_close && line_end == body.len()) {
            // Text that only starts with the boundary is not a delimiter.
            search = after;
            continue;
        }
        if let Some(start) = part_start {
            // The CRLF before the delimiter belongs to the delimiter.
            let end = line_start.saturating_sub(2).max(start);
            parts.push(&body[start..end]);
        }
        if is_close {
            break;
        }
        part_start = Some(line_end + 2);
        search = line_end + 2;
    }
    if parts.is_empty() {
        return Err(BodyError::MalformedMultipart("no body parts"));
    }
    Ok(parts)
}

/// The next delimiter at or after `from`: `dash_boundary` at the start of the
/// body or right after CRLF.
fn find_delimiter(body: &[u8], dash_boundary: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    while at + dash_boundary.len() <= body.len() {
        let offset = body[at..]
            .windows(dash_boundary.len())
            .position(|window| window == dash_boundary)?;
        let found = at + offset;
        if found == 0 || (found >= 2 && &body[found - 2..found] == b"\r\n") {
            return Some(found);
        }
        at = found + 1;
    }
    None
}

/// One body part: MIME header fields, an empty line, the content (RFC 2046
/// §5.1.1 `body-part`). A part without header fields starts with the empty
/// line.
fn parse_part(raw: &[u8], depth: usize) -> Result<BodyPart, BodyError> {
    let (header_block, content) = if raw.is_empty() {
        (raw, raw)
    } else if raw.starts_with(b"\r\n") {
        (&raw[..0], &raw[2..])
    } else {
        match raw.windows(4).position(|window| window == b"\r\n\r\n") {
            Some(end) => (&raw[..end], &raw[end + 4..]),
            None => {
                return Err(BodyError::MalformedMultipart(
                    "body part without an empty line",
                ))
            }
        }
    };
    let header_text = std::str::from_utf8(header_block)
        .map_err(|_| BodyError::MalformedPartHeader("not UTF-8"))?;
    // Unfold continuation lines (RFC 5322 §2.2.3).
    let mut fields: Vec<String> = Vec::new();
    for line in header_text.split("\r\n") {
        if line.starts_with([' ', '\t']) {
            let last = fields.last_mut().ok_or(BodyError::MalformedPartHeader(
                "continuation without a field",
            ))?;
            last.push(' ');
            last.push_str(line.trim());
        } else if !line.is_empty() {
            fields.push(line.to_string());
        }
    }
    let mut content_type = None;
    let mut disposition = None;
    let mut transfer_encoding = None;
    let mut content_id = None;
    let mut header_fields = Vec::with_capacity(fields.len());
    for field in &fields {
        let (name, value) = field
            .split_once(':')
            .ok_or(BodyError::MalformedPartHeader("field without a colon"))?;
        let (name, value) = (name.trim(), value.trim());
        header_fields.push((name.to_string(), value.to_string()));
        match name.to_ascii_lowercase().as_str() {
            "content-type" => {
                content_type = Some(
                    ContentType::from_str(value)
                        .map_err(|_| BodyError::MalformedPartHeader("Content-Type"))?
                        .0,
                );
            }
            "content-disposition" => {
                disposition = Some(
                    ContentDisposition::from_str(value)
                        .map_err(|_| BodyError::MalformedPartHeader("Content-Disposition"))?,
                );
            }
            "content-transfer-encoding" => transfer_encoding = Some(value.to_ascii_lowercase()),
            "content-id" => content_id = Some(value.to_string()),
            _ => {}
        }
    }
    // A body part without a Content-Type is text/plain (RFC 2045 §5.2).
    let content_type = content_type.or_else(|| {
        Some(ContentTypeValue {
            m_type: "text".to_string(),
            m_subtype: "plain".to_string(),
            parameters: Default::default(),
        })
    });
    BodyPart {
        content_type,
        disposition,
        transfer_encoding,
        content: Bytes::copy_from_slice(content),
        parts: Vec::new(),
        header_fields,
        content_id,
    }
    .with_parts(depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(content_type: &str, disposition: Option<&str>, content: &[u8]) -> BodyPart {
        let mut headers = vec![TypedHeader::ContentType(
            ContentType::from_str(content_type).unwrap(),
        )];
        if let Some(disposition) = disposition {
            headers.push(TypedHeader::ContentDisposition(
                ContentDisposition::from_str(disposition).unwrap(),
            ));
        }
        BodyPart::of_message(&headers, &Bytes::copy_from_slice(content))
            .unwrap()
            .unwrap()
    }

    fn sdp_only(media_type: &str) -> bool {
        media_type == "application/sdp"
    }

    const SDP: &[u8] = b"v=0\r\no=- 1 1 IN IP4 192.0.2.1\r\ns=-\r\nc=IN IP4 192.0.2.1\r\nt=0 0\r\nm=audio 4000 RTP/AVP 0\r\n";

    fn mixed(isup_disposition: &str) -> Vec<u8> {
        [
            &b"--unique-boundary-1\r\nContent-Type: application/sdp\r\n\r\n"[..],
            SDP,
            b"\r\n--unique-boundary-1\r\nContent-Type: application/isup;version=itu-t92+\r\n",
            isup_disposition.as_bytes(),
            b"\r\n\x01\x00\x0a\xff\xfe\r\n\r\n--unique-boundary-1--\r\n",
        ]
        .concat()
    }

    #[test]
    fn mixed_sdp_and_isup_keeps_binary_content() {
        let body = body(
            "multipart/mixed;boundary=unique-boundary-1",
            None,
            &mixed("Content-Disposition: signal;handling=optional\r\n"),
        );
        assert_eq!(body.parts.len(), 2);
        assert_eq!(body.parts[0].content, SDP);
        // Binary content survives, including a trailing CRLF that belongs to it.
        assert_eq!(&body.parts[1].content[..], b"\x01\x00\x0a\xff\xfe\r\n");
        assert_eq!(body.session_description().as_deref(), Some(SDP));
        assert!(body.understood(&sdp_only));
    }

    /// RFC 5621 §3.2: a part is required unless `handling=optional`, so an
    /// ISUP part without it is one the UA must understand.
    #[test]
    fn required_part_that_is_not_understood_makes_the_body_not_understood() {
        let required = body(
            "multipart/mixed;boundary=unique-boundary-1",
            None,
            &mixed("Content-Disposition: signal\r\n"),
        );
        assert!(required.parts[1].handling_required());
        assert!(!required.understood(&sdp_only));
        let explicit = body(
            "multipart/mixed;boundary=unique-boundary-1",
            None,
            &mixed("Content-Disposition: signal;handling=required\r\n"),
        );
        assert!(!explicit.understood(&sdp_only));
    }

    #[test]
    fn alternative_is_understood_with_one_understood_alternative() {
        let content = [
            &b"--alt\r\nContent-Type: application/x-unknown-session\r\n\r\nxyz\r\n"[..],
            b"--alt\r\nContent-Type: application/sdp\r\n\r\n",
            SDP,
            b"\r\n--alt--",
        ]
        .concat();
        let body = body("multipart/alternative;boundary=alt", None, &content);
        assert!(body.understood(&sdp_only));
        assert_eq!(body.session_description().as_deref(), Some(SDP));
        assert!(!body.understood(&|_| false));
    }

    #[test]
    fn nested_multipart_and_default_dispositions() {
        let content = [
            &b"--outer\r\nContent-Type: multipart/alternative;boundary=\"inner\"\r\n\r\n"[..],
            b"--inner\r\nContent-Type: application/sdp\r\n\r\n",
            SDP,
            b"\r\n--inner--\r\n--outer\r\n\r\nplain text without a Content-Type\r\n--outer--\r\n",
        ]
        .concat();
        let body = body("multipart/mixed;boundary=outer", None, &content);
        assert_eq!(body.parts.len(), 2);
        assert_eq!(body.parts[0].parts.len(), 1);
        assert_eq!(body.session_description().as_deref(), Some(SDP));
        // RFC 2045 §5.2 default type, RFC 3261 §20.11 default disposition.
        assert_eq!(body.parts[1].media_type().as_deref(), Some("text/plain"));
        assert!(matches!(
            body.parts[1].disposition_type(),
            DispositionType::Render
        ));
        assert!(matches!(
            body.parts[0].parts[0].disposition_type(),
            DispositionType::Session
        ));
        // The text part is required and not understood.
        assert!(!body.understood(&sdp_only));
        assert!(body.understood(&|t| t == "application/sdp" || t == "text/plain"));
    }

    #[test]
    fn sdp_part_with_another_disposition_is_not_the_session_description() {
        let content = [
            &b"--b\r\nContent-Type: application/sdp\r\nContent-Disposition: render\r\n\r\n"[..],
            SDP,
            b"\r\n--b--",
        ]
        .concat();
        let body = body("multipart/mixed;boundary=b", None, &content);
        assert_eq!(body.session_description(), None);
    }

    #[test]
    fn related_root_is_the_start_part() {
        let content = [
            &b"--r\r\nContent-Type: text/plain\r\nContent-ID: <second>\r\n\r\nx\r\n"[..],
            b"--r\r\nContent-Type: application/sdp\r\nContent-ID: <root>\r\n\r\n",
            SDP,
            b"\r\n--r--",
        ]
        .concat();
        let body = body(
            "multipart/related;boundary=r;start=\"<root>\"",
            None,
            &content,
        );
        assert_eq!(body.session_description().as_deref(), Some(SDP));
        assert!(body.understood(&sdp_only));
    }

    #[test]
    fn transfer_encoding_other_than_identity_is_not_understood() {
        let content = b"--b\r\nContent-Type: application/sdp\r\nContent-Transfer-Encoding: base64\r\n\r\ndj0w\r\n--b--";
        let body = body("multipart/mixed;boundary=b", None, content);
        assert!(!body.understood(&sdp_only));
        assert_eq!(body.session_description(), None);
    }

    #[test]
    fn malformed_multipart_bodies_are_errors() {
        let parse = |content_type: &str, content: &[u8]| {
            BodyPart::of_message(
                &[TypedHeader::ContentType(
                    ContentType::from_str(content_type).unwrap(),
                )],
                &Bytes::copy_from_slice(content),
            )
        };
        assert_eq!(
            parse("multipart/mixed", b"--b\r\n\r\nx\r\n--b--"),
            Err(BodyError::MissingBoundary)
        );
        assert!(parse("multipart/mixed;boundary=b", b"--b\r\n\r\nx\r\n").is_err());
        // `--bb` is not the delimiter `--b`, so there is no close delimiter.
        assert!(parse("multipart/mixed;boundary=b", b"--b\r\n\r\nx\r\n--bb--").is_err());
        let deep = (0..=MAX_NESTING)
            .rev()
            .fold(b"\r\nx".to_vec(), |inner, level| {
                [
                    format!(
                        "--l{level}\r\nContent-Type: multipart/mixed;boundary=l{}\r\n\r\n",
                        level + 1
                    )
                    .as_bytes(),
                    &inner,
                    format!("\r\n--l{level}--").as_bytes(),
                ]
                .concat()
            });
        assert_eq!(
            parse("multipart/mixed;boundary=l0", &deep),
            Err(BodyError::TooDeep)
        );
    }

    #[test]
    fn preamble_epilogue_and_padding_are_skipped() {
        let content = [
            &b"This is the preamble.\r\n--b  \r\nContent-Type: application/sdp\r\n\r\n"[..],
            SDP,
            b"\r\n--b-- \r\nepilogue",
        ]
        .concat();
        let body = body("multipart/mixed;boundary=b", None, &content);
        assert_eq!(body.session_description().as_deref(), Some(SDP));
    }

    #[test]
    fn message_session_description() {
        let sdp = Bytes::from_static(SDP);
        let with_type = [TypedHeader::ContentType(
            ContentType::from_str("application/sdp").unwrap(),
        )];
        assert_eq!(session_description(&with_type, &sdp), Some(sdp.clone()));
        // No Content-Type: the body is used only when it is SDP.
        assert_eq!(session_description(&[], &sdp), Some(sdp.clone()));
        assert_eq!(
            session_description(&[], &Bytes::from_static(b"hello")),
            None
        );
        let text = [TypedHeader::ContentType(
            ContentType::from_str("text/plain").unwrap(),
        )];
        assert_eq!(session_description(&text, &sdp), None);
        assert_eq!(session_description(&with_type, &Bytes::new()), None);
    }
}
