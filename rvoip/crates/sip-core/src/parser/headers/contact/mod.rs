// Parser for the Contact header (RFC 3261 Section 20.10)
// Contact = ("Contact" / "m" ) HCOLON ( STAR / (contact-param *(COMMA contact-param)))

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    combinator::{map, opt, value},
    sequence::{pair, preceded},
    IResult,
};

// Import from new modules
use crate::parser::separators::{hcolon, comma, star};
use crate::parser::address::name_addr_or_addr_spec;
use crate::parser::common::comma_separated_list0;
use crate::parser::ParseResult;

// Import local submodules
mod params;
use params::parse_contact_params; // *(SEMI contact-params)

// Import types
use crate::types::contact::{ContactHeader, ContactValue, ContactParams};
use crate::uri::Uri;


// contact-param = (name-addr / addr-spec) *(SEMI contact-params)
fn contact_value(input: &[u8]) -> ParseResult<ContactValue> {
    map(
        pair(
            name_addr_or_addr_spec, // Returns (Option<&[u8]>, Uri)
            parse_contact_params // Returns ContactParams
        ),
        |((display_name_opt, uri), params)| ContactValue {
            // TODO: Handle display name unescaping/conversion
            display_name: display_name_opt.map(|b| String::from_utf8_lossy(b).to_string()),
            uri,
            params,
        }
    )(input)
}


// Contact = ("Contact" / "m" ) HCOLON ( STAR / (contact-param *(COMMA contact-param)))
pub(crate) fn parse_contact(input: &[u8]) -> ParseResult<ContactHeader> {
     preceded(
        pair(alt((tag_no_case(b"Contact"), tag_no_case(b"m"))), hcolon),
        alt((
            map(star, |_| ContactHeader::Star), // Handle Contact: *
            map(comma_separated_list0(contact_value), ContactHeader::Contacts) // Handle list
        ))
    )(input)
}