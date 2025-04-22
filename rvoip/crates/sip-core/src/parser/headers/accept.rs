// Parser for the Accept header (RFC 3261 Section 20.1)
// Accept = "Accept" HCOLON [ accept-value *(COMMA accept-value) ]
// accept-value = media-range [ accept-params ]

use crate::parser::common::{comma_separated_list0};
use crate::parser::token::token; // Use the token parser instead
use crate::parser::common_params::{contact_param_item, semicolon_separated_params0, generic_param};
use crate::parser::separators::{slash, semi};
use crate::parser::ParseResult;
use crate::types::accept::Accept as AcceptHeader; // Specific header type
use crate::types::param::Param;
use crate::types::media_type::MediaType;
use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    combinator::{map, map_res, value},
    sequence::{pair, preceded}
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
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct AcceptValue { // Make struct pub
    pub m_type: String,
    pub m_subtype: String,
    pub q: Option<NotNan<f32>>,
    pub params: HashMap<String, String>, // Generic + media params combined
}

// accept-param = ( "q" EQUAL qvalue ) / generic-param
// qvalue = ( "0" [ "." 0*3DIGIT ] ) / ( "1" [ "." 0*3("0") ] )
// Simplified: Use generic param parser, q value validation done at type level?
fn accept_param(input: &[u8]) -> ParseResult<Param> {
    generic_param(input) // Use generic_param for now
    // TODO: Implement specific qvalue parser if needed
}

// accept-range = media-range [ accept-params ]
fn accept_range(input: &[u8]) -> ParseResult<(String, String, Vec<Param>)> {
    map(
        pair(
            media_range, // (type, subtype)
            semicolon_separated_params0(contact_param_item) // *accept-params
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
            let accept_values = values.into_iter()
                .map(|(t, s, p)| {
                    // Convert parameters to HashMap
                    let mut params = HashMap::new();
                    let mut q_value = None;
                    
                    for param in p {
                        if let crate::types::param::Param::Other(name, Some(value)) = param {
                            // Check for q parameter
                            if name.to_lowercase() == "q" {
                                // Parse q value as float
                                if let Ok(q) = value.to_string().parse::<f32>() {
                                    if let Ok(q_not_nan) = NotNan::new(q) {
                                        q_value = Some(q_not_nan);
                                    }
                                }
                            } else {
                                // Store other parameters
                                params.insert(name, value.to_string());
                            }
                        }
                    }
                    
                    // Create AcceptValue directly
                    AcceptValue {
                        m_type: t,
                        m_subtype: s,
                        q: q_value,
                        params,
                    }
                })
                .collect();
            
            AcceptHeader(accept_values)
        }
    )(input)
}

// #[cfg(test)]
// mod tests { ... } 