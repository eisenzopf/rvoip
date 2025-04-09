use std::str::FromStr;

use bytes::Bytes;
use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_until, take_while1},
    character::complete::{char, digit1, line_ending, space0, space1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, many1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::message::{Message, Request, Response, StatusCode};
use crate::method::Method;
use crate::uri::Uri;
use crate::version::Version;

/// Parse a SIP message from raw bytes
pub fn parse_message(data: &Bytes) -> Result<Message> {
    // Convert bytes to string for parsing
    let data_str = std::str::from_utf8(data).map_err(|_| {
        Error::InvalidFormat("Message contains invalid UTF-8".to_string())
    })?;
    
    // Use nom to parse the full message
    match sip_message(data_str) {
        Ok((_, message)) => Ok(message),
        Err(_) => Err(Error::InvalidFormat("Failed to parse SIP message".to_string())),
    }
}

// Parser for a complete SIP message
fn sip_message(input: &str) -> IResult<&str, Message> {
    alt((
        map(sip_request, Message::Request),
        map(sip_response, Message::Response),
    ))(input)
}

// Parser for a SIP request
fn sip_request(input: &str) -> IResult<&str, Request> {
    // Parse method
    let (input, method) = method_parser(input)?;
    // Parse space
    let (input, _) = space1(input)?;
    // Parse URI
    let (input, uri) = uri_parser(input)?;
    // Parse space
    let (input, _) = space1(input)?;
    // Parse version
    let (input, version) = version_parser(input)?;
    
    let (input, _) = crlf(input)?;
    let (input, headers) = headers_parser(input)?;
    let (input, _) = crlf(input)?;
    let (input, body) = body_parser(input)?;
    
    Ok((
        input,
        Request {
            method,
            uri,
            version,
            headers,
            body: Bytes::from(body),
        },
    ))
}

// Parser for a SIP response
fn sip_response(input: &str) -> IResult<&str, Response> {
    // Parse version
    let (input, version) = version_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, status) = status_code_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, reason) = reason_phrase_parser(input)?;
    
    let (input, _) = crlf(input)?;
    let (input, headers) = headers_parser(input)?;
    let (input, _) = crlf(input)?;
    let (input, body) = body_parser(input)?;
    
    Ok((
        input,
        Response {
            version,
            status,
            reason: if reason.is_empty() { None } else { Some(reason.to_string()) },
            headers,
            body: Bytes::from(body),
        },
    ))
}

// Parser for SIP method
fn method_parser(input: &str) -> IResult<&str, Method> {
    map_res(
        take_while1(|c: char| c.is_ascii_alphabetic()),
        |s: &str| Method::from_str(s)
    )(input)
}

// Parser for SIP URI
fn uri_parser(input: &str) -> IResult<&str, Uri> {
    // For now, we'll just capture the URI as a string and parse it with FromStr
    map_res(
        take_while1(|c: char| !c.is_whitespace()),
        |s: &str| Uri::from_str(s)
    )(input)
}

// Parser for SIP version
fn version_parser(input: &str) -> IResult<&str, Version> {
    map_res(
        recognize(tuple((
            tag("SIP/"),
            digit1,
            char('.'),
            digit1,
        ))),
        |s: &str| Version::from_str(s)
    )(input)
}

// Parser for status code
fn status_code_parser(input: &str) -> IResult<&str, StatusCode> {
    map_res(
        digit1,
        |s: &str| {
            let code = s.parse::<u16>().unwrap_or(0);
            StatusCode::from_u16(code)
        }
    )(input)
}

// Parser for reason phrase
fn reason_phrase_parser(input: &str) -> IResult<&str, &str> {
    take_till(|c| c == '\r' || c == '\n')(input)
}

// Parser for headers
fn headers_parser(input: &str) -> IResult<&str, Vec<Header>> {
    many0(terminated(header_parser, crlf))(input)
}

// Parser for a single header
fn header_parser(input: &str) -> IResult<&str, Header> {
    let (input, (name, value)) = separated_pair(
        map_res(
            take_till(|c| c == ':'),
            |s: &str| HeaderName::from_str(s.trim())
        ),
        tuple((char(':'), space0)),
        map_res(
            take_till(|c| c == '\r' || c == '\n'),
            |s: &str| Ok::<_, Error>(HeaderValue::from_str(s.trim())?)
        )
    )(input)?;
    
    Ok((input, Header::new(name, value)))
}

// Parse the body of the message
fn body_parser(input: &str) -> IResult<&str, String> {
    Ok((input, input.to_string()))  // Convert to owned String to avoid lifetime issues
}

// Parser for CRLF
fn crlf(input: &str) -> IResult<&str, &str> {
    alt((tag("\r\n"), tag("\n")))(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_request() {
        let message = "INVITE sip:bob@example.com SIP/2.0\r\n\
                      Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                      Max-Forwards: 70\r\n\
                      To: Bob <sip:bob@example.com>\r\n\
                      From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                      Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                      CSeq: 314159 INVITE\r\n\
                      Contact: <sip:alice@pc33.example.com>\r\n\
                      Content-Type: application/sdp\r\n\
                      Content-Length: 0\r\n\
                      \r\n";
        
        let message_bytes = Bytes::from(message);
        let parsed = parse_message(&message_bytes).unwrap();
        
        assert!(parsed.is_request());
        if let Message::Request(req) = parsed {
            assert_eq!(req.method, Method::Invite);
            assert_eq!(req.uri.to_string(), "sip:bob@example.com");
            assert_eq!(req.version, Version::sip_2_0());
            assert_eq!(req.headers.len(), 9);
            
            // Check a few headers
            let call_id = req.header(&HeaderName::CallId).unwrap();
            assert_eq!(call_id.value.as_text().unwrap(), "a84b4c76e66710@pc33.example.com");
            
            let from = req.header(&HeaderName::From).unwrap();
            assert_eq!(from.value.as_text().unwrap(), "Alice <sip:alice@example.com>;tag=1928301774");
        } else {
            panic!("Expected request, got response");
        }
    }
    
    #[test]
    fn test_parse_response() {
        let message = "SIP/2.0 200 OK\r\n\
                      Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                      To: Bob <sip:bob@example.com>;tag=a6c85cf\r\n\
                      From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                      Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                      CSeq: 314159 INVITE\r\n\
                      Contact: <sip:bob@192.168.0.2>\r\n\
                      Content-Length: 0\r\n\
                      \r\n";
        
        let message_bytes = Bytes::from(message);
        let parsed = parse_message(&message_bytes).unwrap();
        
        assert!(parsed.is_response());
        if let Message::Response(resp) = parsed {
            assert_eq!(resp.status, StatusCode::Ok);
            assert_eq!(resp.version, Version::sip_2_0());
            assert_eq!(resp.reason_phrase(), "OK");
            assert_eq!(resp.headers.len(), 7);
            
            // Check a few headers
            let to = resp.header(&HeaderName::To).unwrap();
            assert_eq!(to.value.as_text().unwrap(), "Bob <sip:bob@example.com>;tag=a6c85cf");
            
            let cseq = resp.header(&HeaderName::CSeq).unwrap();
            assert_eq!(cseq.value.as_text().unwrap(), "314159 INVITE");
        } else {
            panic!("Expected response, got request");
        }
    }
} 