// Parser for the Accept header (RFC 3261 Section 20.1)
// Accept = "Accept" HCOLON [ accept-range *(COMMA accept-range) ]
// accept-range = media-range *(SEMI accept-param)
// media-range = ( "*/*" / ( m-type SLASH "*" ) / ( m-type SLASH m-subtype ) ) *( SEMI m-parameter )
// accept-param = ("q" EQUAL qvalue) / generic-param
// m-parameter = m-attribute EQUAL m-value (token / quoted-string)

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    combinator::{map, map_res, opt, recognize, value},
    multi::{many0, separated_list0},
    sequence::{pair, preceded, tuple},
    IResult,
};
use std::str;
use std::collections::HashMap;
use ordered_float::NotNan;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, comma};
use crate::parser::common_params::accept_param; // Uses generic_param and qvalue indirectly
use crate::parser::ParseResult;

// Import from sibling header modules
use super::media_type::{parse_media_range, media_params_to_hashmap, m_parameter, m_subtype, m_type}; // Use the specific media_range parser and reuse media_type components

use crate::types::param::Param;
use crate::types::accept::{AcceptRange, MediaRange}; // Assuming these exist

// Reusing m-parameter logic from content_type.rs (could be shared)
fn m_value(input: &[u8]) -> ParseResult<&[u8]> {
    alt((token, quoted_string))(input)
}
fn m_parameter(input: &[u8]) -> ParseResult<(&[u8], &[u8])> {
    separated_pair(token, equal, m_value)(input)
}

// media-range = ( "*/*" / ( m-type SLASH "*" ) / ( m-type SLASH m-subtype ) )
//               *( SEMI m-parameter )
// Returns MediaRange { type: String, subtype: String, params: HashMap<String, String> }
fn media_range(input: &[u8]) -> ParseResult<MediaRange> {
    map_res(
        pair(
            // Parser for type/subtype with wildcards
            alt((
                value((b"*", b"*"), tag(b"*/*".as_slice())),
                pair(m_type, preceded(slash, tag(b"*".as_slice()))),
                pair(m_type, preceded(slash, m_subtype)),
            )),
            // Parameters
            many0(preceded(semi, m_parameter))
        ),
        |((type_bytes, subtype_bytes), params_vec)| {
            let type_str = str::from_utf8(type_bytes)?.to_string();
            let subtype_str = str::from_utf8(subtype_bytes)?.to_string();
            let params_map = params_vec.into_iter().collect::<HashMap<_, _>>();
            Ok(MediaRange { m_type: type_str, m_subtype: subtype_str, parameters: params_map })
        }
    )(input)
}

// accept-range = media-range *(SEMI accept-param)
// Returns AcceptRange { media_range: MediaRange, accept_params: Vec<Param> }
fn accept_range(input: &[u8]) -> ParseResult<AcceptRange> {
    map(
        pair(
            media_range,
            many0(preceded(semi, accept_param))
        ),
        |(range, params)| AcceptRange { media_range: range, accept_params: params }
    )(input)
}

// Define structure for Accept header value (simplified)
#[derive(Debug, PartialEq, Clone)]
pub struct AcceptValue {
    pub m_type: String,
    pub m_subtype: String,
    pub q: Option<NotNan<f32>>,
    pub params: HashMap<String, String>, // Generic + media params combined
}

// Accept = "Accept" HCOLON [ accept-range *(COMMA accept-range) ]
pub(crate) fn parse_accept(input: &[u8]) -> ParseResult<Vec<AcceptRange>> {
    comma_separated_list0(accept_range)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{Param, GenericValue};
    use ordered_float::NotNan;

    #[test]
    fn test_media_range() {
        let (rem, mr) = media_range(b"text/html;level=1").unwrap();
        assert!(rem.is_empty());
        assert_eq!(mr.m_type, "text");
        assert_eq!(mr.m_subtype, "html");
        assert_eq!(mr.parameters.get("level"), Some(&"1".to_string()));

        let (rem_wild, mr_wild) = media_range(b"audio/*").unwrap();
        assert!(rem_wild.is_empty());
        assert_eq!(mr_wild.m_type, "audio");
        assert_eq!(mr_wild.m_subtype, "*");
        assert!(mr_wild.parameters.is_empty());
        
        let (rem_all, mr_all) = media_range(b"*/*").unwrap();
        assert!(rem_all.is_empty());
        assert_eq!(mr_all.m_type, "*");
        assert_eq!(mr_all.m_subtype, "*");
    }

    #[test]
    fn test_accept_range() {
        let (rem, ar) = accept_range(b"application/sdp;q=0.9;custom=val").unwrap();
        assert!(rem.is_empty());
        assert_eq!(ar.media_range.m_type, "application");
        assert_eq!(ar.media_range.m_subtype, "sdp");
        assert!(ar.media_range.parameters.is_empty());
        assert_eq!(ar.accept_params.len(), 2);
        assert!(ar.accept_params.contains(&Param::Q(NotNan::new(0.9).unwrap())));
        assert!(ar.accept_params.contains(&Param::Other("custom".to_string(), Some(GenericValue::Token("val".to_string())))));
    }

    #[test]
    fn test_parse_accept_single() {
        let input = b"audio/*; q=0.2";
        let result = parse_accept(input);
        assert!(result.is_ok());
        let (rem, ranges) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].media_range.m_type, "audio");
        assert_eq!(ranges[0].media_range.m_subtype, "*");
        assert_eq!(ranges[0].accept_params.len(), 1);
        assert!(matches!(ranges[0].accept_params[0], Param::Q(q) if q == NotNan::new(0.2).unwrap()));
    }
    
    #[test]
    fn test_parse_accept_multiple() {
        let input = b"text/plain, application/sdp;level=1, */*;q=0.1";
        let result = parse_accept(input);
        assert!(result.is_ok());
        let (rem, ranges) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(ranges.len(), 3);
        
        assert_eq!(ranges[0].media_range.m_type, "text");
        assert_eq!(ranges[0].media_range.m_subtype, "plain");
        assert!(ranges[0].accept_params.is_empty());

        assert_eq!(ranges[1].media_range.m_type, "application");
        assert_eq!(ranges[1].media_range.m_subtype, "sdp");
        assert_eq!(ranges[1].accept_params.len(), 1);
        assert!(matches!(ranges[1].accept_params[0], Param::Other(n, Some(GenericValue::Token(v))) if n == "level" && v == "1"));

        assert_eq!(ranges[2].media_range.m_type, "*");
        assert_eq!(ranges[2].media_range.m_subtype, "*");
        assert_eq!(ranges[2].accept_params.len(), 1);
        assert!(matches!(ranges[2].accept_params[0], Param::Q(q) if q == NotNan::new(0.1).unwrap()));
    }
    
    #[test]
    fn test_parse_accept_empty() {
        let input = b""; // Empty value allowed
        let result = parse_accept(input);
        assert!(result.is_ok());
        let (rem, ranges) = result.unwrap();
        // Should consume nothing if input is empty and parser accepts empty
        assert!(rem.is_empty()); 
        assert!(ranges.is_empty());
    }
} 