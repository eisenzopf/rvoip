use nom::{
    branch::alt,
    bytes::complete::{tag, take, take_while1},
    combinator::{map_res, recognize},
    error::{ErrorKind},
    multi::many0,
    sequence::{delimited, pair, preceded},
    IResult,
};

use super::separators::{dquote, lparen, rparen};
use super::whitespace::{sws, lws, wsp};
use super::utf8::utf8_nonascii;

// Type alias for parser result
pub type ParseResult<'a, O> = IResult<&'a [u8], O>;


// quoted-pair = "\" (%x00-09 / %x0B-0C / %x0E-7F)
pub fn quoted_pair(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        tag(b"\\"),
        map_res(take(1usize), |c: &[u8]| {
            // Check if byte is empty or CR/LF (invalid escapes in SIP)
            if c.is_empty() || c[0] == b'\r' || c[0] == b'\n' {
                Err(nom::Err::Failure(nom::error::Error::new(input, ErrorKind::Verify)))
            } else {
                Ok(c)
            }
        }),
    ))(input)
}

// qdtext = LWS / %x21 / %x23-5B / %x5D-7E / UTF8-NONASCII
pub fn qdtext(input: &[u8]) -> ParseResult<&[u8]> {
    alt((
        lws,
        recognize(map_res(take(1usize), |c: &[u8]| {
            if c.is_empty() || !(c[0] == 0x21 || (c[0] >= 0x23 && c[0] <= 0x5B) || (c[0] >= 0x5D && c[0] <= 0x7E)) {
                Err("Not qdtext ASCII")
            } else {
                Ok(c)
            }
        })),
        utf8_nonascii
    ))(input)
}

// quoted-string = SWS DQUOTE *(qdtext / quoted-pair ) DQUOTE
// Returns the raw content within the quotes, including escape sequences.
pub fn quoted_string(input: &[u8]) -> ParseResult<&[u8]> {
    preceded(
        sws,
        delimited(
            dquote,
            recognize(many0(alt((qdtext, quoted_pair)))),
            dquote,
        ),
    )(input)
}

// ctext = %x21-27 / %x2A-5B / %x5D-7E / UTF8-NONASCII / LWS
pub fn ctext(input: &[u8]) -> ParseResult<&[u8]> {
    alt((
        lws,
        recognize(map_res(take(1usize), |c: &[u8]| {
            if c.is_empty() || !((c[0] >= 0x21 && c[0] <= 0x27) || (c[0] >= 0x2A && c[0] <= 0x5B) || (c[0] >= 0x5D && c[0] <= 0x7E)) {
                Err("Not ctext ASCII")
            } else {
                Ok(c)
            }
        })),
        utf8_nonascii
    ))(input)
}

// comment = LPAREN *(ctext / quoted-pair / comment) RPAREN
// Recursive parser. We return the content inside the parens.
pub fn comment(input: &[u8]) -> ParseResult<&[u8]> {
    delimited(
        lparen, // Consumes LPAREN and surrounding SWS
        recognize(many0(alt((ctext, quoted_pair, comment)))), // Recursive call
        rparen, // Consumes RPAREN and surrounding SWS
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qdtext_utf8() {
        let input = &[0xE2, 0x82, 0xAC]; // Euro sign (€)
        let result = qdtext(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, input);
    }

    #[test]
    fn test_ctext_utf8() {
        let input = &[0xC3, 0xA7]; // Cedilla (ç)
        let result = ctext(input);
        assert!(result.is_ok());
        let (rem, val) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(val, input);
    }
    
    // TODO: Add tests for quoted_string/comment with embedded UTF8
} 