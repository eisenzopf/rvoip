// Parser for the Accept header (RFC 3261 Section 20.1)
// Accept = "Accept" HCOLON [ accept-value *(COMMA accept-value) ]
// accept-value = media-range [ accept-params ]

use crate::parser::common::{comma_separated_list0};
use crate::parser::token::token; // Use the token parser instead
use crate::parser::common_params::{contact_param_item, semicolon_separated_params0, generic_param};
use crate::parser::separators::{slash, semi, equal};
use crate::parser::ParseResult;
use crate::types::accept::Accept as AcceptHeader; // Specific header type
use crate::types::param::Param;
use crate::types::media_type::MediaType;
use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while_m_n},
    character::complete::{digit1, char},
    combinator::{map, map_res, value, opt, recognize},
    sequence::{pair, preceded, tuple},
    error::{ErrorKind, ParseError}
};
use std::str;
use std::collections::HashMap;
use ordered_float::NotNan;
use serde::{Deserialize, Serialize};

// Define m_type and m_subtype functions since they're not available
fn m_type(input: &[u8]) -> ParseResult<&[u8]> {
    token(input)
}

fn m_subtype(input: &[u8]) -> ParseResult<&[u8]> {
    token(input)
}

// Define structure for Accept header value
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct AcceptValue { // Make struct pub
    pub m_type: String,
    pub m_subtype: String,
    pub q: Option<NotNan<f32>>,
    pub params: HashMap<String, String>, // Generic + media params combined
}

// For direct debug of the parser - remove in production
#[cfg(test)]
fn process_accept_value(t: String, s: String, params: Vec<Param>) -> AcceptValue {
    let mut media_params = HashMap::new();
    let mut q_value = None;
    
    for param in params {
        match param {
            Param::Q(q) => {
                q_value = Some(q);
            },
            Param::Other(name, Some(value)) => {
                media_params.insert(name, value.to_string());
            },
            _ => {} // Ignore other param types
        }
    }
    
    AcceptValue {
        m_type: t.to_lowercase(), // Ensure lowercase for case-insensitive comparison
        m_subtype: s.to_lowercase(), // Ensure lowercase for case-insensitive comparison
        q: q_value,
        params: media_params,
    }
}

// Parse qvalue according to RFC 3261:
// qvalue = ( "0" [ "." 0*3DIGIT ] ) / ( "1" [ "." 0*3("0") ] )
fn qvalue(input: &[u8]) -> ParseResult<NotNan<f32>> {
    // Strictly enforce the ABNF format

    // Check for length after decimal point to ensure we don't exceed 3 digits
    if !input.is_empty() {
        let decimal_point_pos = input.iter().position(|&c| c == b'.');
        if let Some(pos) = decimal_point_pos {
            if input.len() - pos - 1 > 3 {
                return Err(nom::Err::Error(nom::error::Error::new(input, ErrorKind::TooLarge)));
            }
        }
    }

    // Check for "1.1" and other invalid values explicitly
    if input.len() >= 3 && input[0] == b'1' && input[1] == b'.' && 
       (input[2] != b'0' || (input.len() > 3 && input[3] != b'0')) {
        return Err(nom::Err::Error(nom::error::Error::new(input, ErrorKind::Float)));
    }

    // Strict implementation per the ABNF
    alt((
        // "0" [ "." 0*3DIGIT ]
        map_res(
            recognize(tuple((
                char('0'),
                opt(tuple((
                    char('.'),
                    take_while_m_n(0, 3, |c: u8| c.is_ascii_digit())
                )))
            ))),
            |bytes| {
                let s = std::str::from_utf8(bytes)
                    .map_err(|_| nom::Err::Error(nom::error::Error::new(bytes, ErrorKind::Char)))?;
                let val = s.parse::<f32>()
                    .map_err(|_| nom::Err::Error(nom::error::Error::new(bytes, ErrorKind::Float)))?;
                NotNan::new(val)
                    .map_err(|_| nom::Err::Error(nom::error::Error::new(bytes, ErrorKind::Float)))
            }
        ),
        // "1" [ "." 0*3("0") ] - strictly using the ABNF definition
        alt((
            map(tag(b"1"), |_| NotNan::new(1.0).unwrap()),
            map(tag(b"1."), |_| NotNan::new(1.0).unwrap()),
            map(tag(b"1.0"), |_| NotNan::new(1.0).unwrap()),
            map(tag(b"1.00"), |_| NotNan::new(1.0).unwrap()),
            map(tag(b"1.000"), |_| NotNan::new(1.0).unwrap())
        ))
    ))(input)
}

// accept-param = ( "q" EQUAL qvalue ) / generic-param
fn accept_param(input: &[u8]) -> ParseResult<Param> {
    alt((
        // "q" EQUAL qvalue
        map(
            tuple((tag_no_case(b"q"), equal, qvalue)),
            |(_, _, q)| Param::Q(q)
        ),
        // generic-param
        generic_param
    ))(input)
}

// accept-range = media-range [ accept-params ]
fn accept_range(input: &[u8]) -> ParseResult<(String, String, Vec<Param>)> {
    map(
        pair(
            media_range, // (type, subtype)
            semicolon_separated_params0(accept_param) // *accept-params
        ),
        |((m_type, m_subtype), params)| {
            // Convert byte slices to strings using std::str::from_utf8 first
            let type_str = std::str::from_utf8(m_type).unwrap_or_default().to_string();
            let subtype_str = std::str::from_utf8(m_subtype).unwrap_or_default().to_string();
            (type_str, subtype_str, params)
        }
    )(input)
}

// media-range = ( "*/*" / ( m-type SLASH "*" ) / ( m-type SLASH m-subtype ) )
fn media_range(input: &[u8]) -> ParseResult<(&[u8], &[u8])> {
    alt((
        value((b"*" as &[u8], b"*" as &[u8]), tag(b"*/*")),
        pair(m_type, preceded(slash, tag(b"*"))),
        pair(m_type, preceded(slash, m_subtype)),
    ))(input)
}

// Accept = "Accept" HCOLON [ accept-value *(COMMA accept-value) ]
// Note: HCOLON handled elsewhere.
pub fn parse_accept(input: &[u8]) -> ParseResult<AcceptHeader> {
    // Use comma_separated_list0 as the list can be empty
    map(
        comma_separated_list0(accept_range),
        |values| {
            let mut accept_values = values.into_iter()
                .map(|(t, s, p)| {
                    // Convert parameters to HashMap
                    let mut params = HashMap::new();
                    let mut q_value = None;
                    
                    for param in p {
                        match param {
                            Param::Q(q) => {
                                q_value = Some(q);
                            },
                            Param::Other(name, Some(value)) => {
                                params.insert(name, value.to_string());
                            },
                            _ => {} // Ignore other param types
                        }
                    }
                    
                    // Create AcceptValue directly
                    AcceptValue {
                        m_type: t.to_lowercase(), // Ensure lowercase for case-insensitive comparison
                        m_subtype: s.to_lowercase(), // Ensure lowercase for case-insensitive comparison
                        q: q_value,
                        params,
                    }
                })
                .collect::<Vec<AcceptValue>>();
            
            // Sort by q-value (higher values first)
            accept_values.sort_by(|a, b| {
                let a_q = a.q.unwrap_or_else(|| NotNan::new(1.0).unwrap());
                let b_q = b.q.unwrap_or_else(|| NotNan::new(1.0).unwrap());
                // Sort in descending order (higher q values first)
                b_q.cmp(&a_q)
            });
            
            AcceptHeader(accept_values)
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::Param;
    use crate::types::param::GenericValue;
    use std::str::FromStr;

    // Helper function to test q-value parsing directly
    fn test_q_param(input: &[u8]) -> Option<NotNan<f32>> {
        match accept_param(input) {
            Ok((_, Param::Q(q))) => Some(q),
            _ => None,
        }
    }

    #[test]
    fn test_qvalue_parser() {
        // Test valid q-values
        assert!(qvalue(b"0").is_ok());
        assert!(qvalue(b"1").is_ok());
        assert!(qvalue(b"0.5").is_ok());
        assert!(qvalue(b"0.123").is_ok());
        assert!(qvalue(b"1.0").is_ok());
        assert!(qvalue(b"1.00").is_ok());
        assert!(qvalue(b"1.000").is_ok());
        assert!(qvalue(b"0.000").is_ok());
        
        // Parse values and check correctness
        let (_, q1) = qvalue(b"0.5").unwrap();
        assert_eq!(q1.into_inner(), 0.5);
        
        let (_, q2) = qvalue(b"1").unwrap();
        assert_eq!(q2.into_inner(), 1.0);
        
        let (_, q3) = qvalue(b"0.001").unwrap();
        assert_eq!(q3.into_inner(), 0.001);
        
        // Test invalid q-values
        assert!(qvalue(b"1.1").is_err(), "1.1 should be rejected");
        assert!(qvalue(b"1.0000").is_err()); // Too many digits
        assert!(qvalue(b"0.1234").is_err()); // Too many digits
        assert!(qvalue(b"-0.5").is_err()); // Negative
        assert!(qvalue(b"2").is_err()); // > 1.0
        assert!(qvalue(b"abc").is_err()); // Not a number
        
        // Test q param parsing directly
        assert_eq!(test_q_param(b"q=0.5").unwrap().into_inner(), 0.5);
        assert_eq!(test_q_param(b"q=0.8").unwrap().into_inner(), 0.8);
        assert_eq!(test_q_param(b"q=1").unwrap().into_inner(), 1.0);
    }
    
    #[test]
    fn test_media_range_parser() {
        // Test "*/*" format
        let (rem, (t, s)) = media_range(b"*/*").unwrap();
        assert!(rem.is_empty());
        assert_eq!(t, b"*");
        assert_eq!(s, b"*");
        
        // Test "type/*" format
        let (rem, (t, s)) = media_range(b"application/*").unwrap();
        assert!(rem.is_empty());
        assert_eq!(std::str::from_utf8(t).unwrap(), "application");
        assert_eq!(s, b"*");
        
        // Test "type/subtype" format
        let (rem, (t, s)) = media_range(b"text/html").unwrap();
        assert!(rem.is_empty());
        assert_eq!(std::str::from_utf8(t).unwrap(), "text");
        assert_eq!(std::str::from_utf8(s).unwrap(), "html");
        
        // Test invalid inputs
        assert!(media_range(b"*").is_err()); // Missing subtype
        assert!(media_range(b"/html").is_err()); // Missing type
        assert!(media_range(b"application/").is_err()); // Missing subtype
    }
    
    #[test]
    fn test_accept_param_parser() {
        // Test q parameter
        let (rem, param) = accept_param(b"q=0.8").unwrap();
        assert!(rem.is_empty());
        match param {
            Param::Q(q) => assert_eq!(q.into_inner(), 0.8),
            _ => panic!("Expected Q parameter"),
        }
        
        // Test generic parameter
        let (rem, param) = accept_param(b"level=1").unwrap();
        assert!(rem.is_empty());
        match param {
            Param::Other(name, Some(value)) => {
                assert_eq!(name, "level");
                assert_eq!(value.to_string(), "1");
            },
            _ => panic!("Expected generic parameter"),
        }
    }
    
    #[test]
    fn test_accept_range_parser() {
        // Test simple media range
        let (rem, (t, s, params)) = accept_range(b"text/html").unwrap();
        assert!(rem.is_empty());
        assert_eq!(t, "text");
        assert_eq!(s, "html");
        assert!(params.is_empty());
        
        // Create a separate test with q parameter
        let (rem, (t, s, params)) = accept_range(b"application/json;q=0.8").unwrap();
        assert!(rem.is_empty());
        assert_eq!(t, "application");
        assert_eq!(s, "json");
        assert_eq!(params.len(), 1);
        
        // Check for the q parameter - should find exactly one
        let q_params = params.iter().filter(|&p| matches!(p, Param::Q(_))).count();
        assert_eq!(q_params, 1, "Should have exactly one q parameter");
        
        // Check q value directly
        let q_value = params.iter().find_map(|p| {
            if let Param::Q(q) = p {
                Some(q.into_inner())
            } else {
                None
            }
        });
        assert_eq!(q_value, Some(0.8), "q value should be 0.8");
                
        // Test with multiple parameters
        let (rem, (t, s, params)) = accept_range(b"application/json;level=1").unwrap();
        assert!(rem.is_empty());
        assert_eq!(t, "application");
        assert_eq!(s, "json");
        assert_eq!(params.len(), 1);
        
        // Check for the level parameter
        let level_params = params.iter().filter(|&p| {
            if let Param::Other(name, _) = p {
                name == "level"
            } else {
                false
            }
        }).count();
        assert_eq!(level_params, 1, "Should have exactly one level parameter");
    }
    
    #[test]
    fn test_parse_accept() {
        // Test parsing using debug output
        let (rem, parsed) = comma_separated_list0(accept_range)(b"application/json, text/html;q=0.8, */*;q=0.1").unwrap();
        assert!(rem.is_empty());
        
        let values: Vec<AcceptValue> = parsed.into_iter()
            .map(|(t, s, p)| process_accept_value(t, s, p))
            .collect();
        
        // Verify values are parsed correctly
        assert_eq!(values.len(), 3);
        assert_eq!(values[0].m_type, "application");
        assert_eq!(values[0].m_subtype, "json");
        
        assert_eq!(values[1].m_type, "text");
        assert_eq!(values[1].m_subtype, "html");
        
        assert_eq!(values[2].m_type, "*");
        assert_eq!(values[2].m_subtype, "*");
        
        // Test with multiple media types, sorted by q-value
        let (rem, accept) = parse_accept(b"application/json, text/html;q=0.8, */*;q=0.1").unwrap();
        assert!(rem.is_empty());
        assert_eq!(accept.0.len(), 3);
        
        // Values should be sorted by q-value (highest first)
        assert_eq!(accept.0[0].m_type, "application", "application/json should be first with implicit q=1.0");
        assert_eq!(accept.0[0].m_subtype, "json");
        
        assert_eq!(accept.0[1].m_type, "text", "text/html should be second with q=0.8");
        assert_eq!(accept.0[1].m_subtype, "html");
        
        assert_eq!(accept.0[2].m_type, "*", "*/* should be last with q=0.1");
        assert_eq!(accept.0[2].m_subtype, "*");
        
        // Test empty input
        let (rem, accept) = parse_accept(b"").unwrap();
        assert!(rem.is_empty());
        assert!(accept.0.is_empty());
        
        // Test with type/* format
        let (rem, accept) = parse_accept(b"image/*;q=0.9").unwrap();
        assert!(rem.is_empty());
        assert_eq!(accept.0.len(), 1);
        assert_eq!(accept.0[0].m_type, "image");
        assert_eq!(accept.0[0].m_subtype, "*");
        
        // Test with various whitespace
        let (rem, accept) = parse_accept(b"application/xml, text/html;q=0.8").unwrap();
        assert!(rem.is_empty());
        assert_eq!(accept.0.len(), 2);
        assert_eq!(accept.0[0].m_type, "application");
        assert_eq!(accept.0[0].m_subtype, "xml");
        assert_eq!(accept.0[1].m_type, "text");
        assert_eq!(accept.0[1].m_subtype, "html");
        
        // Test case-insensitivity
        let (rem, accept) = parse_accept(b"Text/HTML, APPLICATION/json").unwrap();
        assert!(rem.is_empty());
        assert_eq!(accept.0.len(), 2);
        assert_eq!(accept.0[0].m_type, "text");  // should be lowercased
        assert_eq!(accept.0[0].m_subtype, "html");  // should be lowercased
        assert_eq!(accept.0[1].m_type, "application");  // should be lowercased
        assert_eq!(accept.0[1].m_subtype, "json");  // should be lowercased
    }
    
    #[test]
    fn test_sort_by_qvalue() {
        // Parse input with explicit decreasing q-values
        let input = b"g/h;q=1.0, c/d;q=0.8, e/f;q=0.5, a/b;q=0.2";
        let (_, accept) = parse_accept(input).unwrap();
        
        // Just verify the order is correct (without asserting specific q-values)
        assert_eq!(accept.0.len(), 4);
        assert_eq!(accept.0[0].m_type, "g", "First item should be g/h with q=1.0");
        assert_eq!(accept.0[1].m_type, "c", "Second item should be c/d with q=0.8");
        assert_eq!(accept.0[2].m_type, "e", "Third item should be e/f with q=0.5");
        assert_eq!(accept.0[3].m_type, "a", "Fourth item should be a/b with q=0.2");
        
        // Test with same q-values (order should be preserved for equal q-values)
        let input_same_q = b"e/f;q=1.0, g/h;q=1.0, a/b;q=0.5, c/d;q=0.5";
        let (_, accept_same_q) = parse_accept(input_same_q).unwrap();
        
        // e/f and g/h should be first (both q=1.0), preserving original order
        assert_eq!(accept_same_q.0[0].m_type, "e");
        assert_eq!(accept_same_q.0[1].m_type, "g");
        
        // a/b and c/d should be next (both q=0.5), preserving original order
        assert_eq!(accept_same_q.0[2].m_type, "a");
        assert_eq!(accept_same_q.0[3].m_type, "c");
    }
} 