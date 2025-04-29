// Media types for SDP parsing
//
// Handles parsing of media types (e.g., audio, video, application)

use nom::{
    IResult,
    branch::alt,
    combinator::value,
};
use crate::sdp::media::utils::tag_no_case;

/// Parse a media type - audio, video, application, text, message
pub(crate) fn parse_media_type(input: &str) -> IResult<&str, String> {
    alt((
        value("audio".to_string(), tag_no_case("audio")),
        value("video".to_string(), tag_no_case("video")),
        value("application".to_string(), tag_no_case("application")),
        value("text".to_string(), tag_no_case("text")),
        value("message".to_string(), tag_no_case("message"))
    ))(input)
}

/// Validates if a media type string is valid
pub(crate) fn is_valid_media_type(media_type: &str) -> bool {
    matches!(media_type,
        "audio" | "video" | "application" | "text" | "message"
    )
} 