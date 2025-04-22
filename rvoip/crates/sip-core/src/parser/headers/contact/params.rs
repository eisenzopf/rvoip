// Parser for Contact header parameters

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    combinator::{map, map_res, opt},
    multi::{many0},
    sequence::{pair, preceded},
    IResult,
};
use std::str;
use ordered_float::NotNan;

// Import from base modules
use crate::parser::common_params::generic_param;
use crate::parser::separators::{equal, semi};
use crate::parser::values::{qvalue, delta_seconds};
use crate::parser::ParseResult;

// Import shared types
use crate::types::param::Param;
use crate::types::contact::ContactParams;


// c-p-q = "q" EQUAL qvalue
fn contact_q(input: &[u8]) -> ParseResult<Param> {
    map(preceded(pair(tag_no_case(b"q"), equal), qvalue), Param::Q)(input)
}

// c-p-expires = "expires" EQUAL delta-seconds
fn contact_expires(input: &[u8]) -> ParseResult<Param> {
    map(preceded(pair(tag_no_case(b"expires"), equal), delta_seconds), Param::Expires)(input)
}

// contact-params = c-p-q / c-p-expires / contact-extension
// contact-extension = generic-param
pub(crate) fn contact_param_item(input: &[u8]) -> ParseResult<Param> {
    alt((
        contact_q,
        contact_expires,
        generic_param,
    ))(input)
}

/// Parses *( SEMI contact-params )
/// Aggregates results into a ContactParams struct.
pub(crate) fn parse_contact_params(input: &[u8]) -> ParseResult<ContactParams> {
    map(
        many0(preceded(semi, contact_param_item)),
        |params_vec| {
            let mut params = ContactParams::new();
            for p in params_vec {
                match p {
                    Param::Q(q) => params.q = Some(q),
                    Param::Expires(e) => params.expires = Some(e),
                    Param::Other(k, v) => { params.others.insert(k, v); },
                    _ => {} // Ignore other Param types
                }
            }
            params
        }
    )(input)
} 