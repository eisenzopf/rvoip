use std::collections::HashMap;
use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_while, take_while1, is_not, escaped_transform},
    character::complete::{char, digit1, space0, space1},
    combinator::{map, map_res, opt, recognize, peek},
    multi::{fold_many0, many0, many1, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};

use nom::{Err, Needed};

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::types::Method;
use super::utils::{
    crlf, parse_param_name, parse_param_value, parse_token, 
    parse_quoted_string, parse_text_value, parse_semicolon_params, 
    parse_comma_separated_values
};
use crate::types::{Via, CSeq, Address, Param, MediaType, Allow, Accept, ContentDisposition, DispositionType, Warning, ContentLength, Expires, MaxForwards, CallId, ContentType};
use crate::types::route::Route;
use crate::types::record_route::RecordRoute;
use crate::types::reply_to::ReplyTo;
use crate::types::uri_with_params::UriWithParams;
use crate::types::uri_with_params_list::UriWithParamsList;
use crate::types::auth::{AuthenticationInfo, Authorization, ProxyAuthenticate, ProxyAuthorization, WwwAuthenticate, Algorithm, Qop, Scheme as AuthScheme};
use crate::uri::{Uri, Host, Scheme};
use crate::parser::uri::{scheme_parser, userinfo_parser, host_parser, port_parser, parameters_parser};

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
    // Allow whitespace in the header name before colon for tolerance
    let (input, raw_name) = take_while1(|c: char| c != ':')(input)?;
    
    // Trim whitespace from the header name
    let name_trimmed = raw_name.trim();
    
    // Handle header name parsing with better error reporting
    let name = match HeaderName::from_str(name_trimmed) {
        Ok(name) => name,
        Err(_) => {
            // If standard parsing fails, try again after converting to standard case
            // This helps with headers that might be in unusual case formats
            let standard_case = name_trimmed.split('-')
                .map(|s| {
                    let mut c = s.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str().to_lowercase().as_str(),
                    }
                })
                .collect::<Vec<String>>()
                .join("-");
            
            match HeaderName::from_str(&standard_case) {
                Ok(name) => name,
                Err(_) => HeaderName::Other(name_trimmed.to_string()),
            }
        }
    };
    
    // Allow any amount of whitespace around the colon
    let (input, _) = tuple((char(':'), space0))(input)?;
    
    // Get the value up to the end of line
    let (input, value_str) = take_till(|c| c == '\r' || c == '\n')(input)?;
    
    // Handle different line ending formats
    let (input, _) = alt((
        tag("\r\n"),  // standard CRLF
        tag("\n"),    // just LF 
    ))(input)?;
    
    // Initialize with the first line of the value
    let mut value = value_str.trim().to_string();
    let mut remainder = input;
    
    // Handle header value continuation/folding (when a line starts with whitespace)
    let mut folded_lines = Vec::new();
    
    // Try to handle all types of folded header continuations
    while let Ok((new_remainder, continuation)) = alt((
        continuation_line_crlf,  // handle CRLF folded lines
        continuation_line_lf,    // handle LF folded lines
    ))(remainder) {
        folded_lines.push(continuation);
        remainder = new_remainder;
    }
    
    // Append all continuations with proper spacing
    for continuation in folded_lines {
        if !value.is_empty() && !continuation.is_empty() {
            // Add a space between the value and continuation if needed
            if !value.ends_with(' ') && !continuation.starts_with(' ') {
                value.push(' ');
            }
        }
        value.push_str(&continuation);
    }
    
    // Create header value based on name
    let header_value = HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::text(value));
    
    Ok((remainder, Header::new(name, header_value)))
}

// Helper function to parse continuation lines with CRLF
pub fn continuation_line_crlf(input: &str) -> IResult<&str, String> {
    // A continuation line starts with whitespace after CRLF
    let result = tuple((
        tag("\r\n"),   // CRLF line ending
        space1,        // Starting whitespace (required for continuation)
        take_till(|c| c == '\r' || c == '\n') // Rest of the line
    ))(input);
    
    match result {
        Ok((remainder, (_, _, content))) => {
            Ok((remainder, content.trim().to_string()))
        },
        Err(e) => Err(e),
    }
}

// Helper function to parse continuation lines with LF
pub fn continuation_line_lf(input: &str) -> IResult<&str, String> {
    // A continuation line starts with whitespace after LF
    let result = tuple((
        tag("\n"),     // LF line ending
        space1,        // Starting whitespace (required for continuation)
        take_till(|c| c == '\r' || c == '\n') // Rest of the line
    ))(input);
    
    match result {
        Ok((remainder, (_, _, content))) => {
            Ok((remainder, content.trim().to_string()))
        },
        Err(e) => Err(e),
    }
}

/// Parser for multiple headers
pub fn headers_parser(input: &str) -> IResult<&str, Vec<Header>> {
    terminated(
        many0(header_parser),
        alt((
            tag("\r\n"),  // CRLF empty line
            tag("\n")     // LF empty line
        ))
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
pub fn auth_params_parser(input: &str) -> IResult<&str, HashMap<String, String>> {
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
pub fn auth_param_parser(input: &str) -> IResult<&str, (String, String)> {
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

/// Parse a Contact header value into a Vec of Addresses
pub fn parse_contact(input: &str) -> Result<Vec<Address>> {
    match contact_parser(input) {
        Ok((_, contacts)) => Ok(contacts),
        Err(e) => Err(Error::Parser(format!("Failed to parse Contact header: {:?}", e))),
    }
}

/// Parser for one or more Contact values
fn contact_parser(input: &str) -> IResult<&str, Vec<Address>> {
    separated_list1(
        pair(char(','), space0),
        single_address_parser
    )(input)
}

/// Parse an address header (From, To) into an Address
pub fn parse_address(input: &str) -> Result<Address> {
    let trimmed_input = input.trim();
     if trimmed_input.is_empty() {
        return Err(Error::InvalidHeader("Empty address header value".to_string()));
    }
    match single_address_parser(trimmed_input) {
        Ok((rest, address)) if rest.is_empty() => Ok(address),
        Ok((rest, _)) => Err(Error::InvalidHeader(format!(
            "Trailing characters after address value: {}", rest
        ))),
        Err(e) => Err(Error::Parser(format!("Failed to parse address header: {:?}", e))),
    }
}

/// Parser for a quoted string that handles escaped quotes (\")
fn parse_quoted_string_with_escapes(input: &str) -> IResult<&str, String> {
    delimited(
        char('"'),
        // Use escaped_transform to handle \" and other potential escapes if needed
        escaped_transform(
            // Match normal characters that are not \ or "
            is_not("\"\\\\\""),
            // The control character is \\
            '\\',
            // Transform escaped characters: \" becomes ", \\\\ becomes \\\ etc.
            alt((
                map(tag("\""), |_| "\""), // Map \" to "
                map(tag("\\"), |_| "\\")  // Map \\ to \
                // Add other escapes if necessary
            )),
        ),
        char('"')
    )(input)
}

/// Parser for a single Address (used for Contact, From, To, etc.)
fn single_address_parser(input: &str) -> IResult<&str, Address> {
    // Try parsing name-addr format first ("Display Name" <URI>)
    let name_addr_parser = map(
        tuple((
            terminated(
                alt((
                    parse_quoted_string_with_escapes, 
                    map(take_till(|c| c == '<'), |s: &str| s.trim().to_string())
                )),
                space0 // Allow zero or more spaces before <
            ), 
            delimited(
                char('<'),
                map_res(take_till(|c| c == '>'), |s: &str| Uri::from_str(s)),
                char('>')
            )
        )),
        |(disp_name, uri)| (Some(disp_name), uri) 
    );

    // Try parsing addr-spec format (URI without < >)
    let addr_spec_parser = map(
        // Use map_res to attempt parsing, stop if it fails before params
        map_res(take_till(|c| c == ';' || c == ',' || c == '\r' || c == '\n'), |s: &str| Uri::from_str(s.trim())),
        |uri| (None, uri) // No display name for addr-spec
    );
    
    // Try parsing LAQUOT addr-spec RAQUOT format (<URI>)
    let bracketed_addr_spec_parser = map(
        delimited(
            char('<'),
            map_res(take_till(|c| c == '>'), |s: &str| Uri::from_str(s)),
            char('>')
        ),
         |uri| (None, uri) // No display name
    );

    // Try name-addr, then <addr-spec>, then addr-spec
    let (mut remaining_input, contact_parts) = alt((
        name_addr_parser, 
        bracketed_addr_spec_parser, 
        addr_spec_parser
        ))(input)?;
    
    let (display_name, uri) = contact_parts;
    
    // Parse parameters if present
    let (final_input, params) = match parameters_parser(remaining_input) {
        Ok((final_input, params_vec)) => (final_input, params_vec),
        Err(_) => (remaining_input, Vec::new()),
    };

    // Construct the Address struct
    let address = Address {
        display_name: display_name.filter(|s| !s.is_empty()),
        uri,
        params,
    };

    Ok((final_input, address))
}

/// Parse a CSeq header value into the strong type
pub fn parse_cseq(input: &str) -> Result<CSeq> {
    match cseq_parser(input) {
        Ok((_, cseq)) => Ok(cseq),
        // Provide a more specific error message
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => Err(Error::Parser(format!(
            "Failed to parse CSeq header '{}': {:?}",
            input,
            e.code
        ))),
        Err(nom::Err::Incomplete(_)) => Err(Error::Parser(format!(
            "Incomplete input while parsing CSeq header: {}",
            input
        ))),
    }
}

/// nom parser for a CSeq header value
pub fn cseq_parser(input: &str) -> IResult<&str, CSeq> {
    let (input, seq_str) = map_res(
        take_while1(|c: char| c.is_ascii_digit()),
        |s: &str| s.parse::<u32>() // Parse directly to u32
    )(input)?;
    
    let (input, _) = space1(input)?;
    
    // Only map if Method::from_str succeeds AND is not an Extension
    let (input, method) = map_res(parse_token, |m_str| {
         Method::from_str(m_str).and_then(|method| {
             if matches!(method, Method::Extension(_)) {
                 // Treat unrecognized methods as parse errors for CSeq header
                 Err(Error::InvalidMethod)
             } else {
                 Ok(method)
             }
         })
    })(input)?;
    
    // Return the strongly typed CSeq struct
    Ok((input, CSeq { seq: seq_str, method }))
}

/// Parse a Content-Type header value into the MediaType struct
pub fn parse_content_type(input: &str) -> Result<MediaType> {
    let trimmed_input = input.trim(); // Trim input
    if trimmed_input.is_empty() {
        return Err(Error::InvalidHeader("Empty Content-Type value".to_string()));
    }
    match content_type_parser(trimmed_input) {
        // Check that the entire trimmed input was consumed
        Ok((rest, content_type)) if rest.is_empty() => Ok(content_type),
        Ok((rest, _)) => Err(Error::InvalidHeader(format!(
            "Trailing characters after Content-Type value: {}", rest
        ))),
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => Err(Error::Parser(format!(
            "Failed to parse Content-Type header '{}': {:?}",
            trimmed_input,
            e.code
        ))),
        Err(nom::Err::Incomplete(_)) => Err(Error::Parser(format!(
            "Incomplete input while parsing Content-Type header: {}",
            trimmed_input
        ))),
    }
}

/// nom parser for a Content-Type header value
pub fn content_type_parser(input: &str) -> IResult<&str, MediaType> {
    // Parse type/subtype
    let (input, (type_, subtype)) = separated_pair(
        map(parse_token, |s: &str| s.to_string()),
        char('/'),
        map(parse_token, |s: &str| s.to_string())
    )(input)?;

    // Parse optional parameters directly (parse_semicolon_params handles leading ';')
    let (input, params_map) = match parse_semicolon_params(input) {
        Ok((rest, params)) => (rest, params),
        Err(_) => (input, HashMap::new()), // No parameters found or parsing failed
    };

    let media_type = MediaType {
        type_,
        subtype,
        params: params_map,
    };

    Ok((input, media_type))
}

/// Parse a Via header using nom
pub fn parse_via(input: &str) -> Result<Via> {
    match via_parser(input) {
        Ok((_, via)) => Ok(via),
        Err(e) => Err(Error::Parser(format!("Failed to parse Via header: {:?}", e))),
    }
}

/// Parse multiple Via headers separated by commas
pub fn parse_multiple_vias(input: &str) -> Result<Vec<Via>> {
    match multiple_vias_parser(input) {
        Ok((_, vias)) => Ok(vias),
        Err(e) => Err(Error::Parser(format!("Failed to parse multiple Via headers: {:?}", e))),
    }
}

/// Parser for a Via header's protocol part (SIP/2.0/UDP)
pub fn protocol_parser(input: &str) -> IResult<&str, (String, String, String)> {
    tuple((
        // Protocol name (SIP)
        map(
            take_while1(|c: char| c.is_alphabetic()),
            |s: &str| s.to_string()
        ),
        tag("/"),
        // Version (2.0)
        map(
            take_while1(|c: char| c.is_ascii_digit() || c == '.'),
            |s: &str| s.to_string()
        ),
        tag("/"),
        // Transport (UDP, TCP, etc)
        map(
            take_while1(|c: char| c.is_alphabetic()),
            |s: &str| s.to_string()
        )
    ))(input).map(|(next, (protocol, _, version, _, transport))| {
        (next, (protocol, version, transport))
    })
}

/// Parser for host:port
pub fn host_port_parser(input: &str) -> IResult<&str, (String, Option<u16>)> {
    let (input, host_port) = take_till(|c| c == ';' || c == ',' || c == '\r' || c == '\n')(input)?;

    let host_port_parts: Vec<&str> = host_port.trim().split(':').collect();
    let host = host_port_parts[0].to_string();
    let port = if host_port_parts.len() > 1 {
        host_port_parts[1].parse::<u16>().ok()
    } else {
        None
    };

    Ok((input, (host, port)))
}

/// Parser for a complete Via header
pub fn via_parser(input: &str) -> IResult<&str, Via> {
    let (input, (protocol, version, transport)) = protocol_parser(input)?;
    let (input, _) = space1(input)?;
    let (input, (host, port)) = host_port_parser(input)?;

    // Create a basic Via object using the type from crate::types::via
    let mut via = Via::new(protocol, version, transport, host, port);

    // Parse parameters using the refactored parameters_parser from uri module
    let (input, params) = parameters_parser(input)?;

    // Assign the parsed Vec<Param>
    via.params = params;

    Ok((input, via))
}

/// Parser for multiple Via headers
pub fn multiple_vias_parser(input: &str) -> IResult<&str, Vec<Via>> {
    separated_list1(
        pair(char(','), space0),
        via_parser
    )(input)
}

/// Parse a WWW-Authenticate header value into the WwwAuthenticate struct
pub fn parse_www_authenticate(input: &str) -> Result<WwwAuthenticate> {
    // We expect only one challenge here, but the parser might handle more complex scenarios later
    match www_authenticate_parser(input) {
        Ok((_, auth)) => Ok(auth),
        Err(e) => Err(Error::Parser(format!("Failed to parse WWW-Authenticate header: {:?}", e))),
    }
}

/// nom parser for a WWW-Authenticate header value
pub fn www_authenticate_parser(input: &str) -> IResult<&str, WwwAuthenticate> {
    // Parse the scheme (e.g., "Digest")
    let (input, scheme_str) = parse_token(input)?;
    let scheme = AuthScheme::from_str(scheme_str).map_err(|_| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::MapRes)))?;
    let (input, _) = space1(input)?;

    // Parse the comma-separated parameters
    let (input, params_list) = separated_list1(
        pair(char(','), space0),
        auth_param_parser // Use the existing parameter parser
    )(input)?;

    // Convert Vec<(String, String)> to HashMap<String, String> for easier lookup
    let params: HashMap<String, String> = params_list.into_iter().collect();

    // --- Construct WwwAuthenticate struct --- 
    let realm = params.get("realm")
        .ok_or_else(|| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))? // Realm is mandatory
        .to_string();
        
    let nonce = params.get("nonce")
        .ok_or_else(|| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))? // Nonce is mandatory
        .to_string();
        
    let stale = params.get("stale")
        .and_then(|s| match s.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None, // Invalid stale value
        });

    let algorithm = params.get("algorithm")
        .map(|s| Algorithm::from_str(s).unwrap_or_else(|_| Algorithm::Other(s.to_string()))); // Allow custom algorithms

    // Qop can be a comma-separated list in the header value itself (e.g., qop="auth,auth-int")
    let qop_values = params.get("qop")
        .map(|s| s.split(',').map(|q| q.trim()).filter(|q| !q.is_empty()).collect::<Vec<&str>>())
        .unwrap_or_else(Vec::new);
        
    let qop = qop_values.into_iter()
        .map(|q| Qop::from_str(q).unwrap_or_else(|_| Qop::Other(q.to_string())))
        .collect();

    let www_auth = WwwAuthenticate {
        scheme,
        realm,
        nonce,
        stale,
        algorithm,
        qop,
        domain: params.get("domain").map(|s| s.to_string()),
        opaque: params.get("opaque").map(|s| s.to_string()),
        // Initialize other fields as needed (e.g., charset, userhash)
    };

    Ok((input, www_auth))
}

/// Parse an Allow header value into the Allow struct
pub fn parse_allow(input: &str) -> Result<Allow> {
    let trimmed_input = input.trim();
    if trimmed_input.is_empty() {
        return Err(Error::InvalidHeader("Empty Allow header value".to_string()));
    }
    match allow_parser(trimmed_input) {
        Ok((rest, allow)) if rest.is_empty() => {
            if allow.0.is_empty() {
                 Err(Error::InvalidHeader("Invalid Allow header value (empty list)".to_string()))
            } else {
                Ok(allow)
            }
        },
         Ok((rest, _)) => Err(Error::InvalidHeader(format!(
            "Trailing characters after Allow value: {}", rest
        ))),
        Err(e) => Err(Error::Parser(format!("Failed to parse Allow header: {:?}", e))),
    }
}

/// nom parser for an Allow header value
pub fn allow_parser(input: &str) -> IResult<&str, Allow> {
    map(
        separated_list1(
            // Separator: Allow optional spaces around the comma
            delimited(space0, char(','), space0), 
            map_res(parse_token, |m_str| Method::from_str(m_str))
        ),
        |methods| Allow(methods)
    )(input)
}

/// Parse an Accept header value into the Accept struct
pub fn parse_accept(input: &str) -> Result<Accept> {
    let trimmed_input = input.trim(); // Trim input first
    if trimmed_input.is_empty() {
        return Err(Error::InvalidHeader("Empty Accept header value".to_string()));
    }
    match accept_parser(trimmed_input) {
        // Ensure the entire trimmed input was consumed
        Ok((rest, accept)) if rest.is_empty() => Ok(accept),
        Ok((rest, _)) => Err(Error::InvalidHeader(format!(
            "Trailing characters after Accept value: {}", rest
        ))),
        Err(e) => Err(Error::Parser(format!("Failed to parse Accept header: {:?}", e))),
    }
}

/// nom parser for an Accept header value
pub fn accept_parser(input: &str) -> IResult<&str, Accept> {
    map(
        // Parse comma-separated list of media types
        separated_list1(
            pair(char(','), space0),
            content_type_parser // Use the existing parser for a single MediaType
        ),
        |media_types| Accept(media_types)
    )(input)
}

/// Parse a Content-Disposition header value into the ContentDisposition struct
pub fn parse_content_disposition(input: &str) -> Result<ContentDisposition> {
    match content_disposition_parser(input) {
        Ok((_, disp)) => Ok(disp),
        Err(e) => Err(Error::Parser(format!("Failed to parse Content-Disposition header: {:?}", e))),
    }
}

/// nom parser for a Content-Disposition header value
pub fn content_disposition_parser(input: &str) -> IResult<&str, ContentDisposition> {
    // Parse the disposition type (e.g., session, render)
    let (input, type_str) = parse_token(input)?;
    let disposition_type = match type_str.to_ascii_lowercase().as_str() {
        "session" => DispositionType::Session,
        "render" => DispositionType::Render,
        "icon" => DispositionType::Icon,
        "alert" => DispositionType::Alert,
        _ => DispositionType::Other(type_str.to_string()),
    };

    // Parse parameters using a parser that handles quoted values
    let (input, params_list) = many0(
         preceded(
             // Parameter starts with ; and optional spaces
             pair(char(';'), space0), 
             // Use auth_param_parser which handles quotes and removes them
             auth_param_parser 
         )
     )(input)?;
    let params_map: HashMap<String, String> = params_list.into_iter().collect();

    Ok((input, ContentDisposition { disposition_type, params: params_map }))
}

/// Parse a Warning header value into the Warning struct
pub fn parse_warning(input: &str) -> Result<Warning> {
    match warning_parser(input) {
        Ok((_, warn)) => Ok(warn),
        Err(e) => Err(Error::Parser(format!("Failed to parse Warning header: {:?}", e))),
    }
}

/// nom parser for a Warning header value (e.g., 307 isi.edu "Session parameter 'foo' not understood")
pub fn warning_parser(input: &str) -> IResult<&str, Warning> {
    let (input, code) = map_res(digit1, |s: &str| s.parse::<u16>())(input)?;
    let (input, _) = space1(input)?;
    
    // Agent can be a host or pseudo-host
    let (input, agent_str) = parse_token(input)?;
    // Attempt to parse as URI, fallback to Host::Domain if it fails
    // A simple hostname is valid here according to RFC 3261, treat as domain for Uri struct
    let agent_uri = Uri::from_str(agent_str)
        .or_else(|_| Uri::from_str(&format!("sip:{}", agent_str))) // Try adding sip: scheme
        .map_err(|_| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::MapRes)))?;

    let (input, _) = space1(input)?;
    
    // Text is quoted
    let (input, text) = map(parse_quoted_string, |s: &str| s.to_string())(input)?;

    Ok((input, Warning { code, agent: agent_uri, text }))
}

/// Parse an Authorization header value into the Authorization struct
pub fn parse_authorization(input: &str) -> Result<Authorization> {
    match authorization_parser(input) {
        Ok((_, auth)) => Ok(auth),
        Err(e) => Err(Error::Parser(format!("Failed to parse Authorization header: {:?}", e))),
    }
}

/// nom parser for an Authorization header value
pub fn authorization_parser(input: &str) -> IResult<&str, Authorization> {
    // Parse the scheme (e.g., "Digest")
    let (input, scheme_str) = parse_token(input)?;
    let scheme = AuthScheme::from_str(scheme_str).map_err(|_| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::MapRes)))?;
    let (input, _) = space1(input)?;

    // Parse the comma-separated parameters
    let (input, params_list) = separated_list1(
        pair(char(','), space0),
        auth_param_parser // Use the existing parameter parser
    )(input)?;

    // Convert Vec<(String, String)> to HashMap<String, String> for easier lookup
    let params: HashMap<String, String> = params_list.into_iter().collect();

    // --- Construct Authorization struct --- 
    // Mandatory fields for Digest
    let username = params.get("username")
        .ok_or_else(|| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))?
        .to_string();
    let realm = params.get("realm")
        .ok_or_else(|| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))?
        .to_string();
    let nonce = params.get("nonce")
        .ok_or_else(|| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))?
        .to_string();
    let uri_str = params.get("uri")
        .ok_or_else(|| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))?;
    let uri = Uri::from_str(uri_str).map_err(|_| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::MapRes)))?;
    let response = params.get("response")
        .ok_or_else(|| nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))?
        .to_string();

    // Optional fields
    let algorithm = params.get("algorithm")
        .map(|s| Algorithm::from_str(s).unwrap_or_else(|_| Algorithm::Other(s.to_string())));
        
    let message_qop = params.get("qop") // Name is "qop" in the header
        .map(|s| Qop::from_str(s).unwrap_or_else(|_| Qop::Other(s.to_string())));
        
    let nonce_count = params.get("nc")
        .and_then(|s| u32::from_str_radix(s, 16).ok()); // nc is hex

    let auth = Authorization {
        scheme,
        username,
        realm,
        nonce,
        uri,
        response,
        algorithm,
        message_qop,
        nonce_count,
        cnonce: params.get("cnonce").map(|s| s.to_string()),
        opaque: params.get("opaque").map(|s| s.to_string()),
        // Initialize other fields as needed
    };

    Ok((input, auth))
}

/// Parse a Proxy-Authenticate header value into the ProxyAuthenticate struct
pub fn parse_proxy_authenticate(input: &str) -> Result<ProxyAuthenticate> {
    match www_authenticate_parser(input) { // Reuse WWW-Authenticate parser
        Ok((_, www_auth)) => Ok(ProxyAuthenticate(www_auth)),
        Err(e) => Err(Error::Parser(format!("Failed to parse Proxy-Authenticate header: {:?}", e))),
    }
}

/// Parse a Proxy-Authorization header value into the ProxyAuthorization struct
pub fn parse_proxy_authorization(input: &str) -> Result<ProxyAuthorization> {
    match authorization_parser(input) { // Reuse Authorization parser
        Ok((_, auth)) => Ok(ProxyAuthorization(auth)),
        Err(e) => Err(Error::Parser(format!("Failed to parse Proxy-Authorization header: {:?}", e))),
    }
}

/// Parse an Authentication-Info header value into the AuthenticationInfo struct
pub fn parse_authentication_info(input: &str) -> Result<AuthenticationInfo> {
    match authentication_info_parser(input) {
        Ok((_, auth_info)) => Ok(auth_info),
        Err(e) => Err(Error::Parser(format!("Failed to parse Authentication-Info header: {:?}", e))),
    }
}

/// nom parser for an Authentication-Info header value
pub fn authentication_info_parser(input: &str) -> IResult<&str, AuthenticationInfo> {
    // Parse comma-separated parameters directly
    let (input, params_list) = separated_list1(
        pair(char(','), space0),
        auth_param_parser // Reuse the auth parameter parser
    )(input)?;

    // Convert Vec<(String, String)> to HashMap<String, String> for easier lookup
    let params: HashMap<String, String> = params_list.into_iter().collect();

    // --- Construct AuthenticationInfo struct --- 
    let nextnonce = params.get("nextnonce").map(|s| s.to_string());
    let qop = params.get("qop")
        .map(|s| Qop::from_str(s).unwrap_or_else(|_| Qop::Other(s.to_string())));
    let rspauth = params.get("rspauth").map(|s| s.to_string());
    let cnonce = params.get("cnonce").map(|s| s.to_string());
    // Make nc parsing stricter - must be valid hex
    let nc = match params.get("nc") {
        Some(s) => {
            // Ensure it contains only hex digits and has the correct length (8)
            if s.len() == 8 && s.chars().all(|c| c.is_ascii_hexdigit()) {
                u32::from_str_radix(s, 16).ok()
            } else {
                // Return error if format is wrong, instead of None
                 return Err(nom::Err::Failure(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))
            }
        }
        None => None,
    };

    let auth_info = AuthenticationInfo {
        nextnonce,
        qop,
        rspauth,
        cnonce,
        nc,
    };

    Ok((input, auth_info))
}

/// Parser for a single URI with parameters (e.g., <sip:host;lr>)
pub fn uri_with_params_parser(input: &str) -> IResult<&str, UriWithParams> {
    // Revert to parsing the URI part first, which includes its own parameters
    let (remaining_input, uri_part) = alt((
        // <sip:alice@example.com;param=val> format
        delimited(
            char('<'),
            map_res(take_till(|c| c == '>'), |s: &str| Uri::from_str(s)),
            char('>')
        ),
        // sip:alice@example.com;param=val format (no angle brackets)
        // Stop parsing at the next comma if present (for list context)
        map_res(take_till(|c| c == ',' || c == '\r' || c == '\n'), |s: &str| Uri::from_str(s.trim()))
    ))(input)?;

    // Header parameters are generally NOT associated with Route/Record-Route URIs this way.
    // Parameters specific to the routing hop belong inside the URI's parameters field.
    // We assume no *additional* parameters after the URI part for Route/Record-Route.
    let header_params = Vec::new(); // Initialize as empty
    let final_input = remaining_input; // No further parsing here

    Ok((final_input, UriWithParams { uri: uri_part, params: header_params }))
}

/// Parse a Route header value into the Route struct
pub fn parse_route(input: &str) -> Result<Route> {
    let trimmed_input = input.trim();
    if trimmed_input.is_empty() {
        return Err(Error::InvalidHeader("Empty Route header value".to_string()));
    }
    match route_parser(trimmed_input) {
        Ok((rest, route)) if rest.is_empty() => {
            if route.0.uris.is_empty() {
                Err(Error::InvalidHeader("Invalid Route header value (empty list)".to_string()))
            } else {
                Ok(route)
            }
        },
        Ok((rest, _)) => Err(Error::InvalidHeader(format!(
            "Trailing characters after Route value: {}", rest
        ))),
        Err(e) => Err(Error::Parser(format!("Failed to parse Route header: {:?}", e))),
    }
}

/// nom parser for a Route header value (comma-separated URIs with params)
pub fn route_parser(input: &str) -> IResult<&str, Route> {
    map(
        separated_list1(
            pair(char(','), space0),
            uri_with_params_parser
        ),
        |uris| Route(UriWithParamsList { uris })
    )(input)
}

/// Parse a Record-Route header value into the RecordRoute struct
pub fn parse_record_route(input: &str) -> Result<RecordRoute> {
    let trimmed_input = input.trim();
    if trimmed_input.is_empty() {
        return Err(Error::InvalidHeader("Empty Record-Route header value".to_string()));
    }
    match record_route_parser(trimmed_input) {
        Ok((rest, route)) if rest.is_empty() => {
             if route.0.uris.is_empty() {
                Err(Error::InvalidHeader("Invalid Record-Route header value (empty list)".to_string()))
            } else {
                Ok(route)
            }
        },
        Ok((rest, _)) => Err(Error::InvalidHeader(format!(
            "Trailing characters after Record-Route value: {}", rest
        ))),
        Err(e) => Err(Error::Parser(format!("Failed to parse Record-Route header: {:?}", e))),
    }
}

/// nom parser for a Record-Route header value (identical structure to Route)
pub fn record_route_parser(input: &str) -> IResult<&str, RecordRoute> {
     map(
        separated_list1(
            pair(char(','), space0),
            uri_with_params_parser
        ),
        |uris| RecordRoute(UriWithParamsList { uris })
    )(input)
}

/// Parse a Reply-To header value into the ReplyTo struct
pub fn parse_reply_to(input: &str) -> Result<ReplyTo> {
    // Reply-To has the same structure as From/To/Contact (Name-Addr)
    match single_address_parser(input) {
        Ok((_, address)) => Ok(ReplyTo(address)), 
        Err(e) => Err(Error::Parser(format!("Failed to parse Reply-To header: {:?}", e))),
    }
}

/// Parse a Content-Length header value
pub fn parse_content_length(input: &str) -> Result<ContentLength> {
    input.trim()
         .parse::<usize>()
         .map(ContentLength)
         .map_err(|e| Error::Parser(format!("Invalid Content-Length: {}", e)))
}

/// Parse an Expires header value
pub fn parse_expires(input: &str) -> Result<Expires> {
    input.trim()
         .parse::<u32>()
         .map(Expires)
         .map_err(|e| Error::Parser(format!("Invalid Expires: {}", e)))
}

/// Parse a Max-Forwards header value
pub fn parse_max_forwards(input: &str) -> Result<MaxForwards> {
    input.trim()
         .parse::<u8>()
         .map(MaxForwards)
         .map_err(|e| Error::Parser(format!("Invalid Max-Forwards: {}", e)))
}

/// Parse a Call-ID header value
pub fn parse_call_id(input: &str) -> Result<CallId> {
    // Call-ID is just text, no real parsing needed beyond what HeaderValue does
    Ok(CallId(input.trim().to_string()))
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
    fn test_cseq_parser_typed() {
        let input = "314159 INVITE";
        let result = parse_cseq(input);
        assert!(result.is_ok());
        let cseq = result.unwrap();
        assert_eq!(cseq.seq, 314159); // Compare u32 with u32 literal
        assert_eq!(cseq.method, Method::Invite);
    }
    
    #[test]
    fn test_content_type_parser_typed() {
        let input = "application/sdp";
        let (_, content_type) = content_type_parser(input).unwrap();
        
        assert_eq!(content_type.type_, "application");
        assert_eq!(content_type.subtype, "sdp");
        assert!(content_type.params.is_empty());
        
        // With parameters
        let input = "multipart/mixed; boundary=boundary1; charset=utf-8";
        let (_, content_type) = content_type_parser(input).unwrap();
        
        assert_eq!(content_type.type_, "multipart");
        assert_eq!(content_type.subtype, "mixed");
        assert_eq!(content_type.params.get("boundary").unwrap(), "boundary1");
        assert_eq!(content_type.params.get("charset").unwrap(), "utf-8");
        assert_eq!(content_type.params.len(), 2);
    }

    // ... other tests ...
} 