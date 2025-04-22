// RFC 3261 Section 25.1: Basic Rules
// UTF-8 Character definitions

use nom::{
    branch::alt,
    bytes::complete::{tag, take, take_while_m_n},
    combinator::{recognize, map_res},
    sequence::tuple,
    IResult,
    error::{ErrorKind, Error as NomError},
};

// Type alias for parser result
pub type ParseResult<'a, O> = IResult<&'a [u8], O>;

// UTF8-CONT = %x80-BF
fn utf8_cont(input: &[u8]) -> ParseResult<&[u8]> {
    map_res(take(1usize), |byte: &[u8]| {
        if byte.is_empty() || !(byte[0] >= 0x80 && byte[0] <= 0xBF) {
            Err(nom::Err::Failure(NomError::new(input, ErrorKind::Verify)))
        } else {
            Ok(byte)
        }
    })(input)
}

// UTF8-NONASCII = %xC2-DF 1UTF8-CONT
//               / %xE0-EF 2UTF8-CONT
//               / %xF0-F7 3UTF8-CONT
// Adjusted ranges based on RFC 3629 (excluding overlong sequences C0, C1, etc.)
// Checks first byte to determine length, then takes required bytes and validates.
pub fn utf8_nonascii(input: &[u8]) -> ParseResult<&[u8]> {
    if input.is_empty() {
        return Err(nom::Err::Error(NomError::new(input, ErrorKind::Eof)));
    }

    let first_byte = input[0];
    let len = match first_byte {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return Err(nom::Err::Error(NomError::new(input, ErrorKind::Tag)))
    };

    if input.len() < len {
        return Err(nom::Err::Incomplete(nom::Needed::new(len - input.len())));
    }

    // Validate the sequence according to RFC 3629 rules
    // (Prevents overlong sequences and surrogates)
    let valid = match (first_byte, len) {
        (0xE0, 3) => input[1] >= 0xA0 && input[1] <= 0xBF && input[2] >= 0x80 && input[2] <= 0xBF,
        (0xED, 3) => input[1] >= 0x80 && input[1] <= 0x9F && input[2] >= 0x80 && input[2] <= 0xBF,
        (0xF0, 4) => input[1] >= 0x90 && input[1] <= 0xBF && input[2] >= 0x80 && input[2] <= 0xBF && input[3] >= 0x80 && input[3] <= 0xBF,
        (0xF4, 4) => input[1] >= 0x80 && input[1] <= 0x8F && input[2] >= 0x80 && input[2] <= 0xBF && input[3] >= 0x80 && input[3] <= 0xBF,
        (_, 2)    => input[1] >= 0x80 && input[1] <= 0xBF,
        (_, 3)    => input[1] >= 0x80 && input[1] <= 0xBF && input[2] >= 0x80 && input[2] <= 0xBF,
        (_, 4)    => input[1] >= 0x80 && input[1] <= 0xBF && input[2] >= 0x80 && input[2] <= 0xBF && input[3] >= 0x80 && input[3] <= 0xBF,
        _         => false, // Should be unreachable due to first match
    };

    if valid {
        Ok((&input[len..], &input[..len]))
    } else {
        Err(nom::Err::Error(NomError::new(input, ErrorKind::Verify)))
    }
}

// TEXT-UTF8char = %x21-7E / UTF8-NONASCII
pub fn text_utf8_char(input: &[u8]) -> ParseResult<&[u8]> {
    alt((
        // %x21-7E (Printable US-ASCII chars excluding space)
        map_res(take(1usize), |byte: &[u8]| {
            if byte.is_empty() || !(byte[0] >= 0x21 && byte[0] <= 0x7E) {
               Err(nom::Err::Failure(NomError::new(input, ErrorKind::Verify)))
            } else {
                Ok(byte)
            }
        }),
        utf8_nonascii,
    ))(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::error::ErrorKind;

    #[test]
    fn test_utf8_cont() {
        assert_eq!(utf8_cont(&[0x80]), Ok((&[][..], &[0x80][..])));
        assert_eq!(utf8_cont(&[0xBF]), Ok((&[][..], &[0xBF][..])));
        assert!(utf8_cont(&[0x7F]).is_err());
        assert!(utf8_cont(&[0xC0]).is_err());
        assert!(utf8_cont(&[]).is_err());
    }

    #[test]
    fn test_utf8_nonascii() {
        // 2-byte sequences (e.g., ç)
        assert_eq!(utf8_nonascii(&[0xC3, 0xA7]), Ok((&[][..], &[0xC3, 0xA7][..]))); // ç
        assert!(utf8_nonascii(&[0xC1, 0x80]).is_err()); // Invalid start C1
        assert!(utf8_nonascii(&[0xC3]).is_err()); // Incomplete
        assert!(utf8_nonascii(&[0xC3, 0x20]).is_err()); // Invalid cont byte

        // 3-byte sequences (e.g., €)
        assert_eq!(utf8_nonascii(&[0xE2, 0x82, 0xAC]), Ok((&[][..], &[0xE2, 0x82, 0xAC][..]))); // €
        assert!(utf8_nonascii(&[0xE2, 0x82]).is_err()); // Incomplete
        assert!(utf8_nonascii(&[0xE2, 0x82, 0x20]).is_err()); // Invalid cont byte

        // 4-byte sequences (e.g., 𝄞)
        assert_eq!(utf8_nonascii(&[0xF0, 0x9D, 0x84, 0x9E]), Ok((&[][..], &[0xF0, 0x9D, 0x84, 0x9E][..]))); // 𝄞
        assert!(utf8_nonascii(&[0xF0, 0x9D, 0x84]).is_err()); // Incomplete
        assert!(utf8_nonascii(&[0xF0, 0x9D, 0x84, 0x20]).is_err()); // Invalid cont byte
    }
     #[test]
    fn test_text_utf8_char() {
        assert_eq!(text_utf8_char(b"!"), Ok((&[][..], &b"!"[..])));
        assert_eq!(text_utf8_char(b"A"), Ok((&[][..], &b"A"[..])));
        assert_eq!(text_utf8_char(b"~"), Ok((&[][..], &b"~"[..])));
        assert_eq!(text_utf8_char(&[0xC3, 0xA7, b' ']), Ok((&b" "[..], &[0xC3, 0xA7][..]))); // ç
        assert!(text_utf8_char(b" ").is_err()); // Space is not included
        assert!(text_utf8_char(&[0x0A]).is_err()); // LF is not included
        assert!(text_utf8_char(&[]).is_err());
    }
} 