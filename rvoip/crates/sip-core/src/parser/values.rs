use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1, take_while_m_n},
    character::complete::digit1,
    combinator::{map_res, opt, recognize},
    error::{error_position, ErrorKind},
    multi::{many0, many1},
    sequence::{pair, preceded, tuple},
    IResult,
};
use ordered_float::NotNan;
use std::str;

use super::utf8::text_utf8_char;
use super::whitespace::lws;

// Type alias for parser result
pub(crate) type ParseResult<'a, O> = IResult<&'a [u8], O>;

// delta-seconds = 1*DIGIT
pub(crate) fn delta_seconds(input: &[u8]) -> ParseResult<u32> {
    map_res(digit1, |s: &[u8]| {
        str::from_utf8(s).ok().and_then(|s_str| s_str.parse::<u32>().ok())
    })(input)
}

// qvalue = ( "0" [ "." 0*3DIGIT ] ) / ( "1" [ "." 0*3("0") ] )
pub(crate) fn qvalue(input: &[u8]) -> ParseResult<NotNan<f32>> {
    map_res(
        recognize(alt((
            // 1.000 or 1
            recognize(tuple((
                tag(b"1"),
                opt(pair(tag(b"."), take_while_m_n(0, 3, |c: u8| c == b'0'))),
            ))),
            // 0.xxx or 0
            recognize(tuple((
                tag(b"0"),
                opt(pair(
                    tag(b"."),
                    take_while_m_n(0, 3, |c: u8| c.is_ascii_digit()),
                )),
            ))),
        ))),
        |q_bytes| {
            str::from_utf8(q_bytes)
                .map_err(|_| nom::Err::Failure(error_position!(input, ErrorKind::Char)))
                .and_then(|q_str| {
                    q_str
                        .parse::<f32>()
                        .map_err(|_| nom::Err::Failure(error_position!(input, ErrorKind::Float)))
                        .and_then(|q_f32| {
                            if q_f32 >= 0.0 && q_f32 <= 1.0 {
                                NotNan::try_from(q_f32)
                                    .map_err(|_| nom::Err::Failure(error_position!(input, ErrorKind::Verify))) // Error for NaN
                            } else {
                                Err(nom::Err::Failure(error_position!(input, ErrorKind::Verify)))
                            }
                        })
                })
        },
    )(input)
}

// TEXT-UTF8-TRIM = 1*TEXT-UTF8char *(*LWS TEXT-UTF8char)
// Parses the structure but does not perform trimming or LWS replacement.
// Those should happen after parsing.
// Uses text_utf8_char from the utf8 module.
pub(crate) fn text_utf8_trim(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(pair(
        text_utf8_char,
        many0(preceded(lws, text_utf8_char)),
    ))(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::error::ErrorKind;

    #[test]
    fn test_text_utf8_trim() {
        // Basic ASCII
        assert_eq!(text_utf8_trim(b"Subject Text"), Ok((&[][..], b"Subject Text")));
        assert_eq!(text_utf8_trim(b"OneChar"), Ok((&[][..], b"OneChar")));
        assert_eq!(text_utf8_trim(b"One\t Two"), Ok((&[][..], b"One\t Two"))); // Internal LWS
        assert_eq!(text_utf8_trim(b"Leading LWS"), Ok((&[][..], b"Leading LWS"))); // LWS before second char
        
        // With UTF-8
        assert_eq!(text_utf8_trim(&[b'H', b'e', b'l', b'l', b'o', b' ', 0xC3, 0xA7]), Ok((&[][..], &[b'H', b'e', b'l', b'l', b'o', b' ', 0xC3, 0xA7][..]))); // "Hello ç"
        assert_eq!(text_utf8_trim(&[0xC3, 0xA7, b' ', b'W', b'o', b'r', b'l', b'd']), Ok((&[][..], &[0xC3, 0xA7, b' ', b'W', b'o', b'r', b'l', b'd'][..]))); // "ç World"
        assert_eq!(text_utf8_trim(&[0xC3, 0xA7, b'\t', 0xE2, 0x82, 0xAC]), Ok((&[][..], &[0xC3, 0xA7, b'\t', 0xE2, 0x82, 0xAC][..]))); // "ç\t€"

        // Edge cases
        assert_eq!(text_utf8_trim(b"!"), Ok((&[][..], b"!")));
        assert!(text_utf8_trim(b"\r\n").is_err()); // Should not consume CRLF
        assert!(text_utf8_trim(b" Text").is_err()); // Starts with LWS, not TEXT-UTF8char
        assert!(text_utf8_trim(b"").is_err()); // Empty input

        // Check remaining input
        assert_eq!(text_utf8_trim(b"Value\r\nNext"), Ok((&b"\r\nNext"[..], b"Value")));
    }
} 