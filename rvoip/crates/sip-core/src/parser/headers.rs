use std::collections::HashMap;
use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_while, take_while1},
    character::complete::{char, digit1, space0, space1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, many1, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::method::Method;
use super::utils::{
    crlf, parse_param_name, parse_param_value, parse_token, 
    parse_quoted_string, parse_text_value, parse_semicolon_params, 
    parse_comma_separated_values
};

/// Parse a single header
pub fn parse_header(input: &str) -> Result<Header> {
    match header_parser(input) {
        Ok((_, header)) => Ok(header),
        Err(e) => Err(Error::Parser(format!("Failed to parse header: {:?}", e))),
    }
}

/// Parse headers from a string
pub fn parse_headers(input: &str) -> Result<Vec<Header>> {
    match headers_parser(input) {
        Ok((_, headers)) => Ok(headers),
        Err(e) => Err(Error::Parser(format!("Failed to parse headers: {:?}", e))),
    }
}

/// Parser for a single header line
pub fn header_parser(input: &str) -> IResult<&str, Header> {
    let (input, name) = map_res(
        take_while1(|c: char| c != ':'),
        |s: &str| HeaderName::from_str(s.trim())
    )(input)?;
    
    let (input, _) = tuple((char(':'), space0))(input)?;
    
    let (input, value_str) = take_till(|c| c == '\r' || c == '\n')(input)?;
    let (input, _) = crlf(input)?;
    
    // Check for header continuation (folded lines)
    let mut remainder = input;
    let mut value = value_str.trim().to_string();
    
    while let Ok((new_remainder, (_, _, continuation))) = tuple((
        crlf,
        space1,
        take_till(|c| c == '\r' || c == '\n')
    ))(remainder) {
        // Add the continuation to the value
        value.push(' ');
        value.push_str(continuation.trim());
        remainder = new_remainder;
    }
    
    // Create header value based on name
    let header_value = HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::text(value));
    
    Ok((remainder, Header::new(name, header_value)))
}

/// Parser for multiple headers
fn headers_parser(input: &str) -> IResult<&str, Vec<Header>> {
    terminated(
        many0(header_parser),
        crlf
    )(input)
}

/// Parse authentication parameters (WWW-Authenticate, Authorization)
pub fn parse_auth_params(input: &str) -> Result<HashMap<String, String>> {
    match auth_params_parser(input) {
        Ok((_, params)) => Ok(params),
        Err(e) => Err(Error::Parser(format!("Failed to parse auth parameters: {:?}", e))),
    }
}

/// Parser for authentication parameters
fn auth_params_parser(input: &str) -> IResult<&str, HashMap<String, String>> {
    // Extract auth scheme
    let (input, scheme) = parse_token(input)?;
    let (input, _) = space1(input)?;
    
    // Parse parameters
    let (input, params) = separated_list0(
        pair(char(','), space0),
        auth_param_parser
    )(input)?;
    
    // Create result map
    let mut result = HashMap::new();
    result.insert("scheme".to_string(), scheme.to_string());
    
    // Add all parameters
    for (name, value) in params {
        result.insert(name, value);
    }
    
    Ok((input, result))
}

/// Parser for a single auth parameter
fn auth_param_parser(input: &str) -> IResult<&str, (String, String)> {
    separated_pair(
        map(parse_param_name, |s| s.to_string()),
        tuple((space0, char('='), space0)),
        map(
            alt((
                parse_quoted_string,
                parse_token
            )),
            |s| s.trim_matches('"').to_string()
        )
    )(input)
}

/// Parse a Contact header value
pub fn parse_contact(input: &str) -> Result<Vec<HashMap<String, String>>> {
    match contact_parser(input) {
        Ok((_, contacts)) => Ok(contacts),
        Err(e) => Err(Error::Parser(format!("Failed to parse Contact header: {:?}", e))),
    }
}

/// Parser for one or more Contact values
fn contact_parser(input: &str) -> IResult<&str, Vec<HashMap<String, String>>> {
    separated_list1(
        pair(char(','), space0),
        single_contact_parser
    )(input)
}

/// Parser for a single Contact value
fn single_contact_parser(input: &str) -> IResult<&str, HashMap<String, String>> {
    let (mut input, contact_parts) = alt((
        // <sip:alice@example.com> format (with optional display-name)
        tuple((
            opt(terminated(
                alt((
                    map(parse_quoted_string, |s| s.to_string()),
                    map(take_till(|c| c == '<'), |s: &str| s.trim().to_string())
                )),
                space0
            )),
            delimited(
                char('<'),
                map(take_till(|c| c == '>'), |s: &str| s.to_string()),
                char('>')
            )
        )),
        // Plain URI format
        map(
            take_till(|c| c == ';' || c == ',' || c == '\r' || c == '\n'),
            |s: &str| (None, s.trim().to_string())
        )
    ))(input)?;
    
    let (display_name, uri) = contact_parts;
    
    // Create a map to store the contact information
    let mut result = HashMap::new();
    
    // Add display name if present
    if let Some(name) = display_name {
        let trimmed = name.trim().trim_matches('"');
        if !trimmed.is_empty() {
            result.insert("display_name".to_string(), trimmed.to_string());
        }
    }
    
    // Add URI
    result.insert("uri".to_string(), uri);
    
    // Parse parameters if present
    if let Ok((new_input, params)) = parse_semicolon_params(input) {
        input = new_input;
        for (name, value) in params {
            result.insert(name, value);
        }
    }
    
    Ok((input, result))
}

/// Parse an address header (From, To)
pub fn parse_address(input: &str) -> Result<HashMap<String, String>> {
    match address_parser(input) {
        Ok((_, address)) => Ok(address),
        Err(e) => Err(Error::Parser(format!("Failed to parse address header: {:?}", e))),
    }
}

/// Parser for an address header (From, To)
fn address_parser(input: &str) -> IResult<&str, HashMap<String, String>> {
    // The same as single_contact_parser
    single_contact_parser(input)
}

/// Parse a CSeq header value
pub fn parse_cseq(input: &str) -> Result<HashMap<String, String>> {
    match cseq_parser(input) {
        Ok((_, cseq)) => Ok(cseq),
        Err(e) => Err(Error::Parser(format!("Failed to parse CSeq header: {:?}", e))),
    }
}

/// Parser for a CSeq header value
fn cseq_parser(input: &str) -> IResult<&str, HashMap<String, String>> {
    let (input, sequence) = map_res(
        take_while1(|c: char| c.is_ascii_digit()),
        |s: &str| s.parse::<u32>()
    )(input)?;
    
    let (input, _) = space1(input)?;
    
    let (input, method) = map_res(
        take_while1(|c: char| c.is_alphanumeric() || c == '_' || c == '-'),
        |s: &str| Method::from_str(s)
    )(input)?;
    
    let mut result = HashMap::new();
    result.insert("sequence".to_string(), sequence.to_string());
    result.insert("method".to_string(), method.to_string());
    
    Ok((input, result))
}

/// Parse a Content-Type header value
pub fn parse_content_type(input: &str) -> Result<HashMap<String, String>> {
    match content_type_parser(input) {
        Ok((_, content_type)) => Ok(content_type),
        Err(e) => Err(Error::Parser(format!("Failed to parse Content-Type header: {:?}", e))),
    }
}

/// Parser for a Content-Type header value
fn content_type_parser(input: &str) -> IResult<&str, HashMap<String, String>> {
    let (input, media_type) = separated_pair(
        map(parse_token, |s| s.to_string()),
        char('/'),
        map(parse_token, |s| s.to_string())
    )(input)?;
    
    let mut result = HashMap::new();
    result.insert("media_type".to_string(), media_type.0);
    result.insert("media_subtype".to_string(), media_type.1);
    
    // Parse parameters if present
    if let Ok((new_input, params)) = parse_semicolon_params(input) {
        for (name, value) in params {
            result.insert(name, value);
        }
    }
    
    Ok((input, result))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_header_parser() {
        let input = "Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n";
        let (_, header) = header_parser(input).unwrap();
        
        assert_eq!(header.name, HeaderName::Via);
        assert_eq!(
            header.value.as_text().unwrap(), 
            "SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds"
        );
    }
    
    #[test]
    fn test_headers_parser() {
        let input = "Via: SIP/2.0/UDP pc33.example.com:5060;branch=z9hG4bK776asdhds\r\n\
                   From: Alice <sip:alice@example.com>;tag=1928301774\r\n\
                   To: Bob <sip:bob@example.com>\r\n\
                   Call-ID: a84b4c76e66710@pc33.example.com\r\n\
                   \r\n";
        
        let (_, headers) = headers_parser(input).unwrap();
        
        assert_eq!(headers.len(), 4);
        assert_eq!(headers[0].name, HeaderName::Via);
        assert_eq!(headers[1].name, HeaderName::From);
        assert_eq!(headers[2].name, HeaderName::To);
        assert_eq!(headers[3].name, HeaderName::CallId);
    }
    
    #[test]
    fn test_auth_params_parser() {
        let input = "Digest realm=\"example.com\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", algorithm=MD5";
        let (_, params) = auth_params_parser(input).unwrap();
        
        assert_eq!(params.get("scheme").unwrap(), "Digest");
        assert_eq!(params.get("realm").unwrap(), "example.com");
        assert_eq!(params.get("nonce").unwrap(), "dcd98b7102dd2f0e8b11d0f600bfb0c093");
        assert_eq!(params.get("algorithm").unwrap(), "MD5");
    }
    
    #[test]
    fn test_contact_parser() {
        let input = "\"Alice\" <sip:alice@example.com>;expires=3600, <sip:bob@example.com>;q=0.8";
        let (_, contacts) = contact_parser(input).unwrap();
        
        assert_eq!(contacts.len(), 2);
        
        assert_eq!(contacts[0].get("display_name").unwrap(), "Alice");
        assert_eq!(contacts[0].get("uri").unwrap(), "sip:alice@example.com");
        assert_eq!(contacts[0].get("expires").unwrap(), "3600");
        
        assert_eq!(contacts[1].get("uri").unwrap(), "sip:bob@example.com");
        assert_eq!(contacts[1].get("q").unwrap(), "0.8");
    }
    
    #[test]
    fn test_cseq_parser() {
        let input = "314159 INVITE";
        let (_, cseq) = cseq_parser(input).unwrap();
        
        assert_eq!(cseq.get("sequence").unwrap(), "314159");
        assert_eq!(cseq.get("method").unwrap(), "INVITE");
    }
    
    #[test]
    fn test_content_type_parser() {
        let input = "application/sdp";
        let (_, content_type) = content_type_parser(input).unwrap();
        
        assert_eq!(content_type.get("media_type").unwrap(), "application");
        assert_eq!(content_type.get("media_subtype").unwrap(), "sdp");
        
        // With parameters
        let input = "multipart/mixed; boundary=boundary1";
        let (_, content_type) = content_type_parser(input).unwrap();
        
        assert_eq!(content_type.get("media_type").unwrap(), "multipart");
        assert_eq!(content_type.get("media_subtype").unwrap(), "mixed");
        assert_eq!(content_type.get("boundary").unwrap(), "boundary1");
    }
} 