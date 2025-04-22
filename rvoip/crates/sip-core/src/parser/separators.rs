use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    combinator::recognize,
    multi::many0,
    sequence::{delimited, preceded, terminated, tuple},
    IResult,
};

use super::whitespace::sws;

// Type alias for parser result
pub type ParseResult<'a, O> = IResult<&'a [u8], O>;


fn is_separator_char(c: u8) -> bool {
    c == b'(' || c == b')' || c == b'<' || c == b'>' || c == b'@' ||
    c == b',' || c == b';' || c == b':' || c == b'\\' || c == b'"' ||
    c == b'/' || c == b'[' || c == b']' || c == b'?' || c == b'=' ||
    c == b'{' || c == b'}' || c == b' ' || c == b'\t'
}

pub fn separators(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(is_separator_char)(input)
}

pub fn hcolon(input: &[u8]) -> ParseResult<&[u8]> {
    // HCOLON = *( SP / HTAB ) ":" SWS
    recognize(tuple((many0(alt((tag(b" "), tag(b"\t")))), tag(b":"), sws)))(input)
}

pub fn dquote(input: &[u8]) -> ParseResult<&[u8]> {
    tag(b"\"")(input)
}

// Separator wrappers with SWS
pub fn star(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(sws, tag(b"*"), sws)(input)
}

pub fn slash(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(sws, tag(b"/"), sws)(input)
}

pub fn equal(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(sws, tag(b"="), sws)(input)
}

pub fn lparen(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(sws, tag(b"("), sws)(input)
}

pub fn rparen(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(sws, tag(b")"), sws)(input)
}

pub fn raquot(input: &[u8]) -> ParseResult<&[u8]> {
    // RAQUOT = ">" SWS
    terminated(tag(b">"), sws)(input)
}

pub fn laquot(input: &[u8]) -> ParseResult<&[u8]> {
    // LAQUOT = SWS "<"
    preceded(sws, tag(b"<"))(input)
}

pub fn comma(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(sws, tag(b","), sws)(input)
}

pub fn semi(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(sws, tag(b";"), sws)(input)
}

pub fn colon(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(sws, tag(b":"), sws)(input)
}

pub fn ldquot(input: &[u8]) -> ParseResult<&[u8]> {
    // LDQUOT = SWS DQUOTE
    preceded(sws, dquote)(input)
}

pub fn rdquot(input: &[u8]) -> ParseResult<&[u8]> {
    // RDQUOT = DQUOTE SWS
    terminated(dquote, sws)(input)
} 