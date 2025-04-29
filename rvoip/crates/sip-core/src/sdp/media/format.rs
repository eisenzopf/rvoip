// Media format parsing for SDP
//
// Handles parsing of media formats in m= lines

use nom::{
    IResult,
    character::complete::{space1},
    multi::separated_list1,
    bytes::complete::take_while1,
    combinator::map,
};

/// Parse media formats (space separated list of identifiers)
pub(crate) fn parse_formats(input: &str) -> IResult<&str, Vec<String>> {
    separated_list1(
        space1,
        map(
            take_while1(|c: char| c.is_ascii_digit() || c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_'),
            |s: &str| s.to_string()
        )
    )(input)
}

/// Parse port and optional port count
pub(crate) fn parse_port_and_count(input: &str) -> IResult<&str, (u16, Option<u16>)> {
    use nom::{
        branch::alt,
        character::complete::{char, digit1},
        combinator::{map, map_res},
        sequence::tuple,
    };

    alt((
        // Port with count: "port/count"
        map(
            tuple((
                map_res(digit1, |s: &str| s.parse::<u16>()),
                char('/'),
                map_res(digit1, |s: &str| s.parse::<u16>())
            )),
            |(port, _, count)| (port, Some(count))
        ),
        // Just port
        map(
            map_res(digit1, |s: &str| s.parse::<u16>()),
            |port| (port, None)
        )
    ))(input)
}

/// Check if a format ID is a valid RTP payload type (0-127)
pub(crate) fn is_valid_payload_type(format: &str) -> bool {
    if let Ok(pt) = format.parse::<u8>() {
        return pt <= 127;
    }
    false
} 