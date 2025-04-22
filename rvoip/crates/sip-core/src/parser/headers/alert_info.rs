// Parser for the Alert-Info header (RFC 3261 Section 20.4)
// Alert-Info = "Alert-Info" HCOLON alert-param *(COMMA alert-param)
// alert-param = LAQUOT absoluteURI RAQUOT *( SEMI generic-param )

use nom::{
    bytes::complete::{tag_no_case},
    combinator::{map, map_res},
    multi::{many0},
    sequence::{delimited, pair, preceded},
    IResult,
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, comma, laquot, raquot};
use crate::parser::common_params::{generic_param, semicolon_separated_params0};
use crate::parser::uri::absolute_uri; // Assuming an absolute_uri parser exists
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::uri::Uri;

// Import shared parsers
use super::uri_with_params::uri_with_generic_params;

use serde::{Serialize, Deserialize};

// Make this struct public
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertInfoValue { 
    pub uri: Uri,
    pub params: Vec<Param>
}

// alert-param = LAQUOT absoluteURI RAQUOT *( SEMI generic-param )
fn alert_param(input: &[u8]) -> ParseResult<AlertInfoValue> {
    let (remaining, (uri, params)) = uri_with_generic_params(input)?;
    Ok((remaining, AlertInfoValue { uri, params }))
}

/// Parses an Alert-Info header value.
pub fn parse_alert_info(input: &[u8]) -> ParseResult<Vec<AlertInfoValue>> {
    comma_separated_list1(alert_param)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{GenericValue, Param};

    #[test]
    fn test_parse_alert_info() {
        let input = b"<http://www.example.com/sounds/moo.wav>";
        let result = parse_alert_info(input);
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].uri, "http://www.example.com/sounds/moo.wav");
        assert!(infos[0].params.is_empty());
    }

    #[test]
    fn test_parse_alert_info_multiple() {
         let input = b"<http://a.com/sound>, <http://b.com/sound>;param=X";
         let result = parse_alert_info(input);
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 2);
         assert!(infos[1].params.len(), 1);
    }
} 