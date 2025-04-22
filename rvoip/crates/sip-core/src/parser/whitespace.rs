use nom::{
    branch::alt,
    bytes::complete::tag,
    combinator::recognize,
    multi::{many0, many1},
    IResult,
};

// Type alias for parser result
pub(crate) type ParseResult<'a, O> = IResult<&'a [u8], O>;

pub(crate) fn wsp(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(alt((tag(b" "), tag(b"\t"))))(input)
}

pub(crate) fn lws(input: &[u8]) -> ParseResult<&[u8]> {
    // LWS = [*WSP CRLF] 1*WSP (RFC 3261 definition, but often simplified)
    // Simplified: 1 or more SP or HTAB
    // A more complete implementation would handle folding.
    recognize(many1(alt((tag(b" "), tag(b"\t")))))(input)
}

pub(crate) fn sws(input: &[u8]) -> ParseResult<&[u8]> {
    // SWS = [LWS]
    // Simplified: 0 or more SP or HTAB
    recognize(many0(alt((tag(b" "), tag(b"\t")))))(input)
}

/// Parses CRLF (accepts \r\n or just \n)
/// This is more lenient than strict RFC 3261 but common in practice.
pub(crate) fn crlf(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(alt((tag(b"\r\n"), tag(b"\n"))))(input)
} 