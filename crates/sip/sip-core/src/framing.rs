//! Bounded framing for inbound SIP messages.
//!
//! Stream transports and the typed parser must agree on the exact message
//! boundary. This module is the single authority for `Content-Length`, compact
//! header aliases, header limits, and checked body-length arithmetic.

use std::fmt;

/// Maximum length of one physical SIP start/header line, excluding CRLF.
pub const MAX_SIP_LINE_BYTES: usize = 4 * 1024;
/// Maximum number of logical SIP headers in one message.
pub const MAX_SIP_HEADER_COUNT: usize = 100;
/// Maximum bytes through and including the terminating CRLFCRLF.
pub const MAX_SIP_HEADER_BYTES: usize = 64 * 1024;
/// Maximum SIP message-body size.
pub const MAX_SIP_BODY_BYTES: usize = 16 * 1024 * 1024;
/// Maximum complete SIP message size.
pub const MAX_SIP_MESSAGE_BYTES: usize = MAX_SIP_HEADER_BYTES + MAX_SIP_BODY_BYTES;

/// A complete, validated SIP frame boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SipFrame {
    /// Bytes occupied by the start line, headers, and terminating CRLFCRLF.
    pub header_bytes: usize,
    /// Body bytes declared by the single Content-Length header.
    pub body_bytes: usize,
    /// Total frame bytes (`header_bytes + body_bytes`).
    pub total_bytes: usize,
}

/// Result of incrementally inspecting a SIP receive buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SipFrameStatus {
    /// More bytes are required. `required_total` is known once all headers have
    /// arrived and Content-Length has been validated.
    Incomplete { required_total: Option<usize> },
    /// The buffer contains at least one complete SIP frame.
    Complete(SipFrame),
}

/// Fixed-class framing failures. No rejected header value is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SipFramingError {
    HeaderTooLarge,
    LineTooLong,
    TooManyHeaders,
    InvalidHeaderSyntax,
    InvalidHeaderName,
    MissingContentLength,
    DuplicateContentLength,
    InvalidContentLength,
    ContentLengthOverflow,
    BodyTooLarge,
    MessageTooLarge,
}

impl SipFramingError {
    /// Stable low-cardinality class suitable for logs and metrics.
    pub const fn class(self) -> &'static str {
        match self {
            Self::HeaderTooLarge => "header-too-large",
            Self::LineTooLong => "line-too-long",
            Self::TooManyHeaders => "too-many-headers",
            Self::InvalidHeaderSyntax => "invalid-header-syntax",
            Self::InvalidHeaderName => "invalid-header-name",
            Self::MissingContentLength => "missing-content-length",
            Self::DuplicateContentLength => "duplicate-content-length",
            Self::InvalidContentLength => "invalid-content-length",
            Self::ContentLengthOverflow => "content-length-overflow",
            Self::BodyTooLarge => "body-too-large",
            Self::MessageTooLarge => "message-too-large",
        }
    }
}

impl fmt::Display for SipFramingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "SIP framing rejected input (class={})",
            self.class()
        )
    }
}

impl std::error::Error for SipFramingError {}

/// Inspect the first SIP message in `input` without consuming it.
///
/// Exactly one case-insensitive `Content-Length` or compact `l` header is
/// required. Duplicate values are rejected even when equal so all peers make
/// one unambiguous framing decision. Via, Route, Record-Route, and every other
/// repeatable SIP header are only counted and otherwise remain untouched.
pub fn inspect_sip_frame(input: &[u8]) -> Result<SipFrameStatus, SipFramingError> {
    let Some(header_end) = find_double_crlf(input) else {
        validate_incomplete_header_prefix(input)?;
        return Ok(SipFrameStatus::Incomplete {
            required_total: None,
        });
    };
    let header_bytes = header_end
        .checked_add(4)
        .ok_or(SipFramingError::MessageTooLarge)?;
    if header_bytes > MAX_SIP_HEADER_BYTES {
        return Err(SipFramingError::HeaderTooLarge);
    }

    let body_bytes = parse_headers(&input[..header_end])?;
    if body_bytes > MAX_SIP_BODY_BYTES {
        return Err(SipFramingError::BodyTooLarge);
    }
    let total_bytes = header_bytes
        .checked_add(body_bytes)
        .ok_or(SipFramingError::MessageTooLarge)?;
    if total_bytes > MAX_SIP_MESSAGE_BYTES {
        return Err(SipFramingError::MessageTooLarge);
    }

    let frame = SipFrame {
        header_bytes,
        body_bytes,
        total_bytes,
    };
    if input.len() < total_bytes {
        Ok(SipFrameStatus::Incomplete {
            required_total: Some(total_bytes),
        })
    } else {
        Ok(SipFrameStatus::Complete(frame))
    }
}

fn find_double_crlf(input: &[u8]) -> Option<usize> {
    input.windows(4).position(|window| window == b"\r\n\r\n")
}

fn find_crlf(input: &[u8]) -> Option<usize> {
    input.windows(2).position(|window| window == b"\r\n")
}

fn validate_incomplete_header_prefix(input: &[u8]) -> Result<(), SipFramingError> {
    if input.len() > MAX_SIP_HEADER_BYTES {
        return Err(SipFramingError::HeaderTooLarge);
    }

    let mut cursor = 0;
    let mut line_index = 0;
    let mut header_count = 0;
    while let Some(relative_end) = find_crlf(&input[cursor..]) {
        let line_end = cursor + relative_end;
        let line = &input[cursor..line_end];
        validate_line_len(line)?;
        if line_index > 0 && !is_continuation(line) {
            header_count += 1;
            if header_count > MAX_SIP_HEADER_COUNT {
                return Err(SipFramingError::TooManyHeaders);
            }
        }
        line_index += 1;
        cursor = line_end + 2;
    }
    validate_line_len(&input[cursor..])
}

fn parse_headers(header_block: &[u8]) -> Result<usize, SipFramingError> {
    let mut cursor = 0;
    let mut line_index = 0;
    let mut header_count = 0;
    let mut content_length = None;
    let mut current_content_length: Option<ContentLengthParser> = None;

    loop {
        let (line, next_cursor) = match find_crlf(&header_block[cursor..]) {
            Some(relative_end) => {
                let line_end = cursor + relative_end;
                (&header_block[cursor..line_end], Some(line_end + 2))
            }
            None => (&header_block[cursor..], None),
        };
        validate_line_len(line)?;

        if line_index == 0 {
            if line.is_empty() {
                return Err(SipFramingError::InvalidHeaderSyntax);
            }
        } else if is_continuation(line) {
            let Some(parser) = current_content_length.as_mut() else {
                if header_count == 0 {
                    return Err(SipFramingError::InvalidHeaderSyntax);
                }
                line_index += 1;
                if let Some(next) = next_cursor {
                    cursor = next;
                    continue;
                }
                break;
            };
            parser.feed(line)?;
        } else {
            finish_content_length(&mut current_content_length, &mut content_length)?;
            header_count += 1;
            if header_count > MAX_SIP_HEADER_COUNT {
                return Err(SipFramingError::TooManyHeaders);
            }
            let Some(colon) = line.iter().position(|byte| *byte == b':') else {
                return Err(SipFramingError::InvalidHeaderSyntax);
            };
            let name = &line[..colon];
            if !is_header_name(name) {
                return Err(SipFramingError::InvalidHeaderName);
            }
            if is_content_length_name(name) {
                if content_length.is_some() || current_content_length.is_some() {
                    return Err(SipFramingError::DuplicateContentLength);
                }
                let mut parser = ContentLengthParser::default();
                parser.feed(&line[colon + 1..])?;
                current_content_length = Some(parser);
            }
        }

        line_index += 1;
        let Some(next) = next_cursor else {
            break;
        };
        cursor = next;
    }

    finish_content_length(&mut current_content_length, &mut content_length)?;
    content_length.ok_or(SipFramingError::MissingContentLength)
}

fn validate_line_len(line: &[u8]) -> Result<(), SipFramingError> {
    if line.len() > MAX_SIP_LINE_BYTES {
        Err(SipFramingError::LineTooLong)
    } else {
        Ok(())
    }
}

fn is_continuation(line: &[u8]) -> bool {
    matches!(line.first(), Some(b' ' | b'\t'))
}

fn is_header_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.iter().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'-' | b'.' | b'!' | b'%' | b'*' | b'_' | b'+' | b'`' | b'\'' | b'~'
                )
        })
}

fn is_content_length_name(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"content-length") || name.eq_ignore_ascii_case(b"l")
}

fn finish_content_length(
    parser: &mut Option<ContentLengthParser>,
    content_length: &mut Option<usize>,
) -> Result<(), SipFramingError> {
    if let Some(parser) = parser.take() {
        if content_length.is_some() {
            return Err(SipFramingError::DuplicateContentLength);
        }
        *content_length = Some(parser.finish()?);
    }
    Ok(())
}

#[derive(Default)]
struct ContentLengthParser {
    value: usize,
    saw_digit: bool,
    saw_trailing_whitespace: bool,
}

impl ContentLengthParser {
    fn feed(&mut self, input: &[u8]) -> Result<(), SipFramingError> {
        for byte in input {
            match byte {
                b'0'..=b'9' if !self.saw_trailing_whitespace => {
                    self.saw_digit = true;
                    self.value = self
                        .value
                        .checked_mul(10)
                        .and_then(|value| value.checked_add(usize::from(byte - b'0')))
                        .ok_or(SipFramingError::ContentLengthOverflow)?;
                }
                b' ' | b'\t' => {
                    if self.saw_digit {
                        self.saw_trailing_whitespace = true;
                    }
                }
                _ => return Err(SipFramingError::InvalidContentLength),
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<usize, SipFramingError> {
        if self.saw_digit {
            Ok(self.value)
        } else {
            Err(SipFramingError::InvalidContentLength)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(content_length: &[u8], body: &[u8]) -> Vec<u8> {
        let mut message = b"OPTIONS sip:service.example SIP/2.0\r\nVia: SIP/2.0/UDP edge.example;branch=z9hG4bK.one\r\n".to_vec();
        message.extend_from_slice(content_length);
        message.extend_from_slice(b"\r\n\r\n");
        message.extend_from_slice(body);
        message
    }

    #[test]
    fn accepts_canonical_and_compact_content_length() {
        for header in [b"Content-Length: 4".as_slice(), b"L:\t4".as_slice()] {
            let message = request(header, b"body");
            let SipFrameStatus::Complete(frame) = inspect_sip_frame(&message).unwrap() else {
                panic!("complete frame expected");
            };
            assert_eq!(frame.body_bytes, 4);
            assert_eq!(frame.total_bytes, message.len());
        }
    }

    #[test]
    fn rejects_duplicate_content_length_in_all_alias_orders() {
        for headers in [
            b"Content-Length: 0\r\nContent-Length: 0".as_slice(),
            b"Content-Length: 0\r\nl: 0".as_slice(),
            b"l: 0\r\nCONTENT-LENGTH: 0".as_slice(),
            b"l: 0\r\nL: 1".as_slice(),
        ] {
            assert_eq!(
                inspect_sip_frame(&request(headers, b"")),
                Err(SipFramingError::DuplicateContentLength)
            );
        }
    }

    #[test]
    fn rejects_missing_invalid_non_utf8_and_overflow_content_length() {
        let missing = b"OPTIONS sip:service.example SIP/2.0\r\nVia: x\r\n\r\n";
        assert_eq!(
            inspect_sip_frame(missing),
            Err(SipFramingError::MissingContentLength)
        );
        for header in [
            b"Content-Length: nope".as_slice(),
            b"Content-Length: \xff".as_slice(),
            b"Content-Length: 184467440737095516160".as_slice(),
        ] {
            let expected = if header.ends_with(b"160") {
                SipFramingError::ContentLengthOverflow
            } else {
                SipFramingError::InvalidContentLength
            };
            assert_eq!(inspect_sip_frame(&request(header, b"")), Err(expected));
        }
    }

    #[test]
    fn reports_incomplete_header_and_body_without_consuming_a_boundary() {
        assert_eq!(
            inspect_sip_frame(b"OPTIONS sip:service.example SIP/2.0\r\n"),
            Ok(SipFrameStatus::Incomplete {
                required_total: None
            })
        );
        let message = request(b"Content-Length: 4", b"bo");
        let header_bytes = message.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert_eq!(
            inspect_sip_frame(&message),
            Ok(SipFrameStatus::Incomplete {
                required_total: Some(header_bytes + 4)
            })
        );
    }

    #[test]
    fn enforces_header_line_count_body_and_total_bounds() {
        let long_line = vec![b'a'; MAX_SIP_LINE_BYTES + 1];
        assert_eq!(
            inspect_sip_frame(&long_line),
            Err(SipFramingError::LineTooLong)
        );

        let mut many = b"OPTIONS sip:service.example SIP/2.0\r\n".to_vec();
        for _ in 0..=MAX_SIP_HEADER_COUNT {
            many.extend_from_slice(b"X-Test: 1\r\n");
        }
        assert_eq!(
            inspect_sip_frame(&many),
            Err(SipFramingError::TooManyHeaders)
        );

        let huge = request(
            format!("Content-Length: {}", MAX_SIP_BODY_BYTES + 1).as_bytes(),
            b"",
        );
        assert_eq!(inspect_sip_frame(&huge), Err(SipFramingError::BodyTooLarge));
    }

    #[test]
    fn repeatable_routing_headers_do_not_affect_framing() {
        let message = b"OPTIONS sip:service.example SIP/2.0\r\nVia: first\r\nVia: second\r\nRoute: one\r\nRoute: two\r\nRecord-Route: one\r\nRecord-Route: two\r\nContent-Length: 0\r\n\r\n";
        assert!(matches!(
            inspect_sip_frame(message),
            Ok(SipFrameStatus::Complete(_))
        ));
    }

    #[test]
    fn public_parser_uses_the_same_strict_singleton_decision() {
        for (headers, class) in [
            (
                b"Content-Length: 0\r\nl: 0".as_slice(),
                "duplicate-content-length",
            ),
            (
                b"Content-Length: invalid".as_slice(),
                "invalid-content-length",
            ),
        ] {
            let error = crate::parse_message(&request(headers, b""))
                .expect_err("public parser must enforce framing singleton policy");
            assert!(error.to_string().contains(class), "{error}");
        }
    }
}
