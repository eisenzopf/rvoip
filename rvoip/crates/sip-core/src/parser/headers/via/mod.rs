// Parser for the Via header (RFC 3261 Section 20.42)
// Via = ( "Via" / "v" ) HCOLON via-parm *(COMMA via-parm)
// via-parm = sent-protocol LWS sent-by *( SEMI via-params )

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    character::complete::{digit1, space1},
    combinator::{map, map_res, opt},
    sequence::{pair, preceded, separated_pair, tuple},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::separators::{hcolon, comma, slash};
use crate::parser::token::token;
use crate::parser::whitespace::lws;
use crate::parser::uri::host::hostport;
use crate::parser::common::comma_separated_list1;
use crate::parser::common_params::semicolon_separated_params0;
use crate::parser::ParseResult;

// Import local submodules
mod params;
use params::via_param_item; // Use the parser for a single via param item

// Import types
use crate::types::via::{ViaHeader, SentProtocol};
use crate::uri::Host;

// sent-protocol = protocol-name SLASH protocol-version SLASH transport
// protocol-name = "SIP" / token
// protocol-version = token
// transport = "UDP" / "TCP" / "TLS" / "SCTP" / other-transport
fn sent_protocol(input: &[u8]) -> ParseResult<SentProtocol> {
    map_res(
        tuple((
            alt((tag_no_case(b"SIP"), token)), // name
            preceded(slash, token), // version
            preceded(slash, token), // transport
        )),
        |(name_b, ver_b, tran_b)| {
            let name = str::from_utf8(name_b)?.to_string();
            let version = str::from_utf8(ver_b)?.to_string();
            let transport = str::from_utf8(tran_b)?.to_string();
            Ok(SentProtocol { name, version, transport })
        }
    )(input)
}

// sent-by = host [ COLON port ]
// Uses hostport parser from uri module
fn sent_by(input: &[u8]) -> ParseResult<(Host, Option<u16>)> {
    hostport(input)
}

// via-parm = sent-protocol LWS sent-by *( SEMI via-params )
fn via_param_parser(input: &[u8]) -> ParseResult<ViaHeader> {
    map(
        tuple((
            sent_protocol,
            preceded(lws, sent_by),
            semicolon_separated_params0(via_param_item) // Use list helper
        )),
        |(protocol, (host, port), params)| ViaHeader {
            sent_protocol: protocol,
            sent_by_host: host,
            sent_by_port: port,
            params,
        }
    )(input)
}

// Via = ( "Via" / "v" ) HCOLON via-parm *(COMMA via-parm)
pub(crate) fn parse_via(input: &[u8]) -> ParseResult<Vec<ViaHeader>> {
    preceded(
        pair(alt((tag_no_case(b"Via"), tag_no_case(b"v"))), hcolon),
        comma_separated_list1(via_param_parser) // Use the parser for a full via-parm
    )(input)
}