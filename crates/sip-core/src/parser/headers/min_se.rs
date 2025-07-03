// ABNF from RFC 4028, Section 3:
// Min-SE        =  "Min-SE" HCOLON delta-seconds *(SEMI generic-param)
// delta-seconds = 1*DIGIT
//
// Note: This parser is strict and only accepts delta-seconds, followed by EOF.
// Parameters are not parsed or allowed, aligning with the current MinSE struct.

use crate::types::MinSE;
use nom::character::complete::u32 as parse_u32_nom; // ADDED import
use nom::{
    IResult,
    sequence::terminated,
    character::complete::multispace0,
    combinator::eof,
    error::ParseError,
    // Removed unused imports: tuple, tag, char, digit1, map_res, map, opt, many0, alt, common_params
};

// delta_seconds helper is no longer needed if parse_u32 is used directly.

/// Parses the value part of a Min-SE header (the part after "Min-SE:").
/// It expects only delta-seconds and consumes trailing OWS and EOF.
///
/// Example: "90"
/// Example: "1800 "
pub fn parse_min_se_value<'a, E>(input: &'a [u8]) -> IResult<&'a [u8], MinSE, E>
where
    E: ParseError<&'a [u8]> + nom::error::FromExternalError<&'a [u8], std::num::ParseIntError>,
{
    // Use parse_u32_nom for delta-seconds directly
    let (input, val) = parse_u32_nom(input)?; // CHANGED to parse_u32_nom
    // Ensure that after delta_seconds, there's only optional whitespace and EOF.
    // Any parameters would cause `eof` to fail.
    let (input, _) = terminated(multispace0, eof)(input)?;
    Ok((input, MinSE { delta_seconds: val }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::error::{VerboseError, Error as NomError};

    #[test]
    fn test_parse_min_se_value_valid() {
        assert_eq!(parse_min_se_value::<VerboseError<&[u8]>>(b"90"), Ok((&b""[..], MinSE::new(90))));
        assert_eq!(parse_min_se_value::<VerboseError<&[u8]>>(b"1800"), Ok((&b""[..], MinSE::new(1800))));
    }

    #[test]
    fn test_parse_min_se_value_with_ows() {
        // OWS before EOF is consumed
        assert_eq!(parse_min_se_value::<VerboseError<&[u8]>>(b"120  "), Ok((&b""[..], MinSE::new(120))));
    }

    #[test]
    fn test_parse_min_se_value_empty() {
        assert!(parse_min_se_value::<VerboseError<&[u8]>>(b"").is_err());
    }

    #[test]
    fn test_parse_min_se_value_leading_whitespace_fails_if_not_handled_before_call() {
        assert!(parse_min_se_value::<VerboseError<&[u8]>>(b" 90").is_err());
    }

    #[test]
    fn test_parse_min_se_value_trailing_characters_fail() {
        assert!(parse_min_se_value::<VerboseError<&[u8]>>(b"90extra").is_err());
        assert!(parse_min_se_value::<VerboseError<&[u8]>>(b"90;param=val").is_err());
    }

    #[test]
    fn test_parse_min_se_value_max() {
        let max_val_str = u32::MAX.to_string();
        assert_eq!(parse_min_se_value::<VerboseError<&[u8]>>(max_val_str.as_bytes()), Ok((&b""[..], MinSE::new(u32::MAX))));
    }

    #[test]
    fn test_parse_min_se_value_overflow_u32() {
        let overflow_val_str = (u64::from(u32::MAX) + 1).to_string();
        // parse_u32 should handle this gracefully and return an error.
        assert!(parse_min_se_value::<VerboseError<&[u8]>>(overflow_val_str.as_bytes()).is_err());
    }
} 