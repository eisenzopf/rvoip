// Parser for the Call-Info header (RFC 3261 Section 20.9)
// Call-Info = "Call-Info" HCOLON info *(COMMA info)
// info = LAQUOT absoluteURI RAQUOT *( SEMI info-param)
// info-param = ( "purpose" EQUAL ( "icon" / "info" / "card" / token ) ) / generic-param

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case},
    combinator::{map, map_res},
    multi::{many0, separated_list1},
    sequence::{pair, preceded, delimited, tuple},
    IResult,
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, comma, equal, laquot, raquot};
use crate::parser::common_params::generic_param;
use crate::parser::uri::absolute_uri; // Assuming an absolute_uri parser exists
use crate::parser::token::token;
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::uri::Uri;
use serde::{Serialize, Deserialize};

// Make these types public
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoPurpose {
    Icon,
    Info,
    Card,
    Other(String),
}
#[derive(Debug, Clone, PartialEq)]
pub enum InfoParam {
    Purpose(InfoPurpose),
    Generic(Param),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallInfoValue {
    pub uri: Uri,
    pub params: Vec<Param>,
}

// info-param = ( "purpose" EQUAL ( "icon" / "info" / "card" / token ) ) / generic-param
fn info_param(input: &[u8]) -> ParseResult<InfoParam> {
    alt((
        map(
            preceded(
                pair(tag_no_case(b"purpose"), equal),
                alt((
                    map_res(tag_no_case("icon"), |_| Ok::<_, ()>(InfoPurpose::Icon)),
                    map_res(tag_no_case("info"), |_| Ok(InfoPurpose::Info)),
                    map_res(tag_no_case("card"), |_| Ok(InfoPurpose::Card)),
                    map_res(token, |b| str::from_utf8(b).map(|s| InfoPurpose::Other(s.to_string())))
                ))
            ),
            InfoParam::Purpose
        ),
        map(generic_param, InfoParam::Generic)
    ))(input)
}

// info = LAQUOT absoluteURI RAQUOT *( SEMI info-param)
// Returns (Uri, Vec<Param>)
fn info(input: &[u8]) -> ParseResult<CallInfoValue> {
     map_res(
        pair(
             map_res( // Use map_res to handle potential UTF-8 error from absoluteURI bytes
                delimited(
                    crate::parser::separators::laquot,
                    crate::parser::uri::parse_absolute_uri, 
                    crate::parser::separators::raquot
                ),
                 |bytes| str::from_utf8(bytes).map(String::from)
            ),
            many0(preceded(semi, info_param))
        ),
        |(uri_str, params_vec)| Ok(CallInfoValue { uri: uri_str, params: params_vec })
    )(input)
}

// Call-Info = "Call-Info" HCOLON info *(COMMA info)
pub(crate) fn parse_call_info(input: &[u8]) -> ParseResult<Vec<CallInfoValue>> {
    comma_separated_list1(info)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{GenericValue, Param};

    #[test]
    fn test_info_param() {
        let (rem_p, param_p) = info_param(b"purpose=icon").unwrap();
        assert!(rem_p.is_empty());
        assert!(matches!(param_p, InfoParam::Purpose(InfoPurpose::Icon)));

        let (rem_g, param_g) = info_param(b"random=xyz").unwrap();
        assert!(rem_g.is_empty());
        assert!(matches!(param_g, InfoParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) if n=="random" && v=="xyz"));
    }
    
    #[test]
    fn test_parse_call_info() {
        let input = b"<http://www.example.com/alice/photo.jpg> ;purpose=icon, <http://www.example.com/alice/> ;purpose=info";
        let result = parse_call_info(input);
        assert!(result.is_ok());
        let (rem, infos) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].uri, "http://www.example.com/alice/photo.jpg");
        assert_eq!(infos[0].params.len(), 1);
        assert!(matches!(infos[0].params[0], InfoParam::Purpose(InfoPurpose::Icon)));
        assert_eq!(infos[1].uri, "http://www.example.com/alice/");
        assert!(matches!(infos[1].params[0], InfoParam::Purpose(InfoPurpose::Info)));
    }
} 