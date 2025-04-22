use nom::{
    branch::alt,
    bytes::complete::{tag, take, take_while1, take_while_m_n},
    character::complete::{alpha1, alphanumeric1, digit1, hex_digit1},
    combinator::{map_res, recognize},
    sequence::tuple,
    IResult,
};
use std::str;

// Type alias for parser result
pub(crate) type ParseResult<'a, O> = IResult<&'a [u8], O>;

// Core Rules (RFC 2234) & Basic Rules (RFC 3261) Character Sets

pub(crate) fn alpha(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(nom::character::is_alphabetic)(input)
}

pub(crate) fn digit(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(nom::character::is_digit)(input)
}

pub(crate) fn alphanum(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(nom::character::is_alphanumeric)(input)
}

pub(crate) fn hex_digit(input: &[u8]) -> ParseResult<&[u8]> {
    hex_digit1(input)
}

pub(crate) fn lhex(input: &[u8]) -> ParseResult<&[u8]> {
    // LHEX = DIGIT / %x61-66 ;lowercase a-f
    take_while1(|c: u8| c.is_ascii_digit() || (c >= b'a' && c <= b'f'))(input)
}

pub(crate) fn mark(input: &[u8]) -> ParseResult<&[u8]> {
    // mark = "-" / "_" / "." / "!" / "~" / "*" / "'" / "(" / ")"
    recognize(alt((
        tag(b"-"), tag(b"_"), tag(b"."), tag(b"!"), tag(b"~"), 
        tag(b"*"), tag(b"'"), tag(b"("), tag(b")")
    )))(input)
}

pub(crate) fn unreserved(input: &[u8]) -> ParseResult<&[u8]> {
    // unreserved = alphanum / mark
    alt((alphanum, mark))(input)
}

pub(crate) fn reserved(input: &[u8]) -> ParseResult<&[u8]> {
    // reserved = ";" / "/" / "?" / ":" / "@" / "&" / "=" / "+" / "$" / ","
    recognize(alt((
        tag(b";"), tag(b"/"), tag(b"?"), tag(b":"), tag(b"@"), 
        tag(b"&"), tag(b"="), tag(b"+"), tag(b"$"), tag(b",")
    )))(input)
}

fn is_hex_digit_byte(c: u8) -> bool {
    (c >= b'0' && c <= b'9') || (c >= b'A' && c <= b'F') || (c >= b'a' && c <= b'f')
}

pub(crate) fn escaped(input: &[u8]) -> ParseResult<&[u8]> {
    // escaped = "%" HEXDIG HEXDIG
    recognize(tuple((tag(b"%"), take_while_m_n(2, 2, is_hex_digit_byte))))(input)
}

pub(crate) fn lalpha(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(|c: u8| c.is_ascii_lowercase())(input)
}

pub(crate) fn ualpha(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(|c: u8| c.is_ascii_uppercase())(input)
} 