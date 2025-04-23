use nom::{
    branch::alt,
    bytes::complete::{tag, take_while_m_n},
    combinator::{opt, recognize},
    multi::{many0, many1},
    sequence::{pair, preceded},
    IResult,
};

// Type alias for parser result
pub type ParseResult<'a, O> = IResult<&'a [u8], O>;

/// Parses a single whitespace character (SP or HTAB)
pub fn wsp(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(alt((tag(b" "), tag(b"\t"))))(input)
}

/// Parses optional whitespace (0 or more SP or HTAB)
fn owsp(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many0(wsp))(input)
}

/// Parses CRLF (accepts \r\n or just \n)
/// This is more lenient than strict RFC 3261 but common in practice.
pub fn crlf(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(alt((tag(b"\r\n"), tag(b"\n"))))(input)
}

/// Parses Linear White Space (LWS) according to RFC 3261 Section 25.1
/// LWS = [*WSP CRLF] 1*WSP ; linear whitespace
/// This includes handling line folding, where CRLF followed by whitespace
/// is treated as a continuation of the same line.
pub fn lws(input: &[u8]) -> ParseResult<&[u8]> {
    alt((
        // Case 1: Folded line - *WSP CRLF 1*WSP
        recognize(pair(
            pair(owsp, crlf),
            many1(wsp)
        )),
        // Case 2: Simple whitespace - 1*WSP (without folding)
        recognize(many1(wsp))
    ))(input)
}

/// Parses optional whitespace (SWS) according to RFC 3261
/// SWS = [LWS] ; optional linear whitespace
pub fn sws(input: &[u8]) -> ParseResult<&[u8]> {
    opt(lws)(input).map(|(rem, opt_val)| (rem, opt_val.unwrap_or(&[])))
} 