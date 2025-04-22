// Parser for the Content-Disposition header (RFC 3261 Section 20.13)
// Content-Disposition = "Content-Disposition" HCOLON disp-type *( SEMI disp-param )
// disp-type = "render" / "session" / "icon" / "alert" / disp-extension-token (token)
// disp-param = handling-param / generic-param
// handling-param = "handling" EQUAL ( "optional" / "required" / other-handling (token) )

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    combinator::{map, map_res, opt, value},
    multi::many0,
    sequence::{pair, preceded},
    IResult,
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, equal};
use crate::parser::token::token;
use crate::parser::common_params::generic_param;
use crate::parser::ParseResult;

use crate::types::param::Param;
use crate::types::content_disposition::{DispositionType, DispositionParam, Handling};

// disp-type = "render" / "session" / "icon" / "alert" / disp-extension-token
// disp-extension-token = token
fn disp_type(input: &[u8]) -> ParseResult<DispositionType> {
    map_res(
        alt((
            tag_no_case("render"), tag_no_case("session"),
            tag_no_case("icon"), tag_no_case("alert"),
            token // Fallback for extension token
        )),
        |bytes| {
            let s = str::from_utf8(bytes)?;
            Ok(match s.to_ascii_lowercase().as_str() {
                "render" => DispositionType::Render,
                "session" => DispositionType::Session,
                "icon" => DispositionType::Icon,
                "alert" => DispositionType::Alert,
                other => DispositionType::Other(other.to_string()),
            })
        }
    )(input)
}

// handling-param = "handling" EQUAL ( "optional" / "required" / other-handling )
// other-handling = token
fn handling_param(input: &[u8]) -> ParseResult<Handling> {
    map_res(
        preceded(
            pair(tag_no_case(b"handling"), equal),
            alt((tag_no_case("optional"), tag_no_case("required"), token))
        ),
        |bytes| {
             let s = str::from_utf8(bytes)?;
            Ok(match s.to_ascii_lowercase().as_str() {
                "optional" => Handling::Optional,
                "required" => Handling::Required,
                other => Handling::Other(other.to_string()),
            })
        }
    )(input)
}

// disp-param = handling-param / generic-param
fn disp_param(input: &[u8]) -> ParseResult<DispositionParam> {
    alt((
        map(handling_param, DispositionParam::Handling),
        map(generic_param, DispositionParam::Generic) // Map Param::Other
    ))(input)
}

// Define structure for Content-Disposition value
#[derive(Debug, PartialEq, Clone)]
pub struct ContentDispositionValue {
    pub disp_type: String,
    pub params: Vec<Param>,
}

// Content-Disposition = "Content-Disposition" HCOLON disp-type *( SEMI disp-param )
pub(crate) fn parse_content_disposition(input: &[u8]) -> ParseResult<(DispositionType, Vec<DispositionParam>)> {
    pair(
        disp_type,
        many0(preceded(semi, disp_param))
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::param::{GenericValue, Param};

    #[test]
    fn test_disp_param() {
        let (rem_h, param_h) = disp_param(b"handling=required").unwrap();
        assert!(rem_h.is_empty());
        assert!(matches!(param_h, DispositionParam::Handling(Handling::Required)));

        let (rem_g, param_g) = disp_param(b"filename=myfile.txt").unwrap();
        assert!(rem_g.is_empty());
        assert!(matches!(param_g, DispositionParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) if n == "filename" && v == "myfile.txt"));
    }
    
    #[test]
    fn test_parse_content_disposition_simple() {
        let input = b"session";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (rem, (dtype, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(dtype, DispositionType::Session);
        assert!(params.is_empty());
    }
    
    #[test]
    fn test_parse_content_disposition_with_params() {
        let input = b"attachment; filename=pic.jpg; handling=optional";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (rem, (dtype, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(dtype, DispositionType::Extension("attachment".to_string())); // Common non-SIP value
        assert_eq!(params.len(), 2);
        assert!(params.contains(&DispositionParam::Generic(Param::Other("filename".to_string(), Some(GenericValue::Token("pic.jpg".to_string()))))));
        assert!(params.contains(&DispositionParam::Handling(Handling::Optional)));
    }
} 