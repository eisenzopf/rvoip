use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1, take_while_m_n},
    character::complete::{digit1},
    combinator::{map, map_res, opt, recognize},
    multi::{many0, many1},
    sequence::{pair, preceded, separated_pair},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;

// Use new specific modules
use crate::parser::common_chars::{escaped, unreserved};
use crate::parser::token::token;
use crate::parser::separators::{semi, equal};
use crate::parser::uri::host::host; // For maddr_param
use crate::parser::utils::unescape_uri_component; // Import unescape helper
use crate::parser::ParseResult;
use crate::types::param::{Param, GenericValue}; // Using GenericValue now
use crate::types::uri::Host as UriHost; // Avoid conflict if Host enum imported directly
use crate::error::Error;

// param-unreserved = "[" / "]" / "/" / ":" / "&" / "+" / "$"
fn is_param_unreserved(c: u8) -> bool {
    matches!(c, b'[' | b']' | b'/' | b':' | b'&' | b'+' | b'$')
}

// paramchar = param-unreserved / unreserved / escaped
// Returns raw bytes
pub(crate) fn paramchar(input: &[u8]) -> ParseResult<&[u8]> {
    alt((take_while1(is_param_unreserved), unreserved, escaped))(input)
}

// pname = 1*paramchar
// Returns raw bytes, unescaping happens in other_param
pub(crate) fn pname(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many1(paramchar))(input)
}

// pvalue = 1*paramchar
// Returns raw bytes, unescaping happens in other_param
pub(crate) fn pvalue(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many1(paramchar))(input)
}

// other-param = pname [ "=" pvalue ]
// Updated to unescape name and value
fn other_param(input: &[u8]) -> ParseResult<Param> {
    map_res(
        pair(pname, opt(preceded(equal, pvalue))),
        |(name_bytes, value_opt_bytes)| {
            let name = unescape_uri_component(name_bytes)?;
            let value_opt = value_opt_bytes
                .map(|v_bytes| unescape_uri_component(v_bytes))
                .transpose()?;
            // Construct Param::Other, but now the value is just Option<String>
            // This loses the Host/Token/Quoted distinction from generic_param
            // TODO: Revisit if URI params need GenericValue like header params
            Ok::<Param, nom::error::Error<&[u8]>>(Param::Other(name, value_opt.map(|v| GenericValue::Token(v)))) // Temp: Stuff into Token
        }
    )(input)
}

// transport-param = "transport=" ( "udp" / "tcp" / "sctp" / "tls" / other-transport)
// other-transport = token
fn transport_param(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(tag("transport="), token),
        |t_bytes| str::from_utf8(t_bytes).map(|s| Param::Transport(s.to_string()))
    )(input)
}

// user-param = "user=" ( "phone" / "ip" / other-user)
// other-user = token
fn user_param(input: &[u8]) -> ParseResult<Param> {
     map_res(
        preceded(tag("user="), token),
        |u_bytes| str::from_utf8(u_bytes).map(|s| Param::User(s.to_string()))
    )(input)
}

// method-param = "method=" Method (Method from request line)
// For URI context, Method is just a token.
fn method_param(input: &[u8]) -> ParseResult<Param> {
     map_res(
        preceded(tag("method="), token),
        |m_bytes| str::from_utf8(m_bytes).map(|s| Param::Method(s.to_string()))
    )(input)
}

// ttl-param = "ttl=" ttl (1*3 DIGIT)
fn ttl_param(input: &[u8]) -> ParseResult<Param> {
    map_res(
        preceded(tag("ttl="), take_while_m_n(1, 3, |c: u8| c.is_ascii_digit())),
        |ttl_bytes| {
            let s = str::from_utf8(ttl_bytes)
                .map_err(|_| nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))?;
            s.parse::<u8>()
                .map_err(|_| nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Digit)))
                .map(Param::Ttl)
        }
    )(input)
}

// maddr-param = "maddr=" host
fn maddr_param(input: &[u8]) -> ParseResult<Param> {
    map(
        preceded(tag("maddr="), host), // Use the actual host parser
        |host_val: UriHost| Param::Maddr(host_val.to_string()) // Add .to_string()
    )(input)
}

// lr-param = "lr"
fn lr_param(input: &[u8]) -> ParseResult<Param> {
    map(tag("lr"), |_| Param::Lr)(input)
}

// uri-parameter = transport-param / user-param / method-param / ttl-param / maddr-param / lr-param / other-param
fn uri_parameter(input: &[u8]) -> ParseResult<Param> {
    // Order matters: check specific params before generic 'other_param'
    alt((
        transport_param,
        user_param,
        method_param,
        ttl_param,
        maddr_param,
        lr_param,
        other_param, // Must be last
    ))(input)
}

// uri-parameters = *( ";" uri-parameter)
pub fn uri_parameters(input: &[u8]) -> ParseResult<Vec<Param>> {
    many0(preceded(semi, uri_parameter))(input)
}

#[cfg(test)]
mod tests {
     use super::*;
     use crate::types::uri::Host;
     // ... other imports ...

    #[test]
    fn test_other_param_unescaped() {
        let (rem, param) = other_param(b"name%20with%20space=val%2fslash").unwrap();
        assert!(rem.is_empty());
        // Check unescaped name and value
        if let Param::Other(name, Some(GenericValue::Token(value))) = param {
            assert_eq!(name, "name with space");
            assert_eq!(value, "val/slash");
        } else {
            panic!("Param structure mismatch");
        }
    }
    
    #[test]
    fn test_uri_parameters_unescaped() {
        let input = b";transport=tcp;p%20name=p%20val;lr";
        let (rem, params) = uri_parameters(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 3);
        assert!(params.contains(&Param::Transport("tcp".to_string())));
        assert!(params.contains(&Param::Lr));
        assert!(params.iter().any(|p| matches!(p, Param::Other(n, Some(GenericValue::Token(v))) if n == "p name" && v == "p val")));
    }
} 