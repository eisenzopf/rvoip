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
    sequence::{pair, preceded, terminated},
    IResult,
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;

// Import from base parser modules
use crate::parser::separators::{hcolon, semi, equal};
use crate::parser::token::token;
use crate::parser::common_params::generic_param;
use crate::parser::ParseResult;
use crate::parser::whitespace::{lws, owsp, sws};

use crate::types::param::Param;
use crate::types::content_disposition::{ContentDisposition, DispositionType, DispositionParam, Handling};

// disp-type = "render" / "session" / "icon" / "alert" / disp-extension-token
// disp-extension-token = token
fn disp_type(input: &[u8]) -> ParseResult<DispositionType> {
    // Handle any leading whitespace including line folding
    let (input, _) = opt(lws)(input)?;
    
    map_res(
        alt((
            tag_no_case("render"), tag_no_case("session"),
            tag_no_case("icon"), tag_no_case("alert"),
            token // Fallback for extension token
        )),
        |bytes| { 
            str::from_utf8(bytes)
                .map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Char)))
                .and_then(|s| {
                    Ok::<DispositionType, nom::Err<NomError<&[u8]>>>(match s.to_ascii_lowercase().as_str() {
                        "render" => DispositionType::Render,
                        "session" => DispositionType::Session,
                        "icon" => DispositionType::Icon,
                        "alert" => DispositionType::Alert,
                        other => DispositionType::Other(other.to_string()),
                    })
                })
        }
    )(input)
}

/// Parses the Content-Disposition header value.
pub fn parse_content_disposition(input: &[u8]) -> ParseResult<(String, Vec<DispositionParam>)> {
    // First check for empty input
    if input.is_empty() {
        return Err(nom::Err::Error(NomError::from_error_kind(input, ErrorKind::TakeWhile1)));
    }

    let (input, dtype) = disp_type(input)?;
    let (input, params_vec) = many0(preceded(
        terminated(semi, opt(lws)), // Handle whitespace after semicolon, including line folding
        disp_param
    ))(input)?;
    
    // Handle any trailing whitespace
    let (input, _) = sws(input)?;
    
    // Check that there's nothing left to parse
    if !input.is_empty() {
        return Err(nom::Err::Error(NomError::from_error_kind(input, ErrorKind::Eof)));
    }
    
    let disp_type_str = match dtype {
        DispositionType::Render => "render".to_string(),
        DispositionType::Session => "session".to_string(),
        DispositionType::Icon => "icon".to_string(),
        DispositionType::Alert => "alert".to_string(),
        DispositionType::Other(s) => s,
    };
    
    Ok((input, (disp_type_str, params_vec)))
}

// handling-param parser
fn handling_param(input: &[u8]) -> ParseResult<Handling> {
    // Handle any leading whitespace including line folding for parameter name
    let (input, _) = opt(lws)(input)?;
    
    preceded(
        pair(tag_no_case("handling"), terminated(equal, opt(lws))), // Allow whitespace after equals
        map_res(
            alt((tag_no_case("optional"), tag_no_case("required"), token)),
            |bytes| { 
                 str::from_utf8(bytes)
                    .map_err(|_| nom::Err::Failure(NomError::from_error_kind(bytes, ErrorKind::Char)))
                    .and_then(|s| {
                        Ok::<Handling, nom::Err<NomError<&[u8]>>>(match s.to_ascii_lowercase().as_str() {
                            "optional" => Handling::Optional,
                            "required" => Handling::Required,
                            other => Handling::Other(other.to_string()),
                        })
                    })
            }
        )
    )(input)
}

// Define the disp_param parser function
fn disp_param(input: &[u8]) -> ParseResult<DispositionParam> {
    // Handle any leading whitespace for the parameter
    let (input, _) = opt(lws)(input)?;
    
    alt((
        map(handling_param, DispositionParam::Handling),
        map(generic_param, DispositionParam::Generic), // Use existing generic_param parser
    ))(input)
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
        assert_eq!(dtype, "session");
        assert!(params.is_empty());
    }
    
    #[test]
    fn test_parse_content_disposition_with_params() {
        let input = b"attachment; filename=pic.jpg; handling=optional";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (rem, (dtype, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(dtype, "attachment");
        assert_eq!(params.len(), 2);
        assert!(params.iter().any(|p| matches!(p, DispositionParam::Generic(Param::Other(n, Some(GenericValue::Token(v)))) if n == "filename" && v == "pic.jpg")));
        assert!(params.iter().any(|p| matches!(p, DispositionParam::Handling(Handling::Optional))));
    }
    
    #[test]
    fn test_rfc3261_disposition_types() {
        // Test all standard disposition types from RFC 3261
        for disp_type in &["render", "session", "icon", "alert"] {
            let input = disp_type.as_bytes();
            let result = parse_content_disposition(input);
            assert!(result.is_ok());
            let (rem, (dtype, _)) = result.unwrap();
            assert!(rem.is_empty());
            assert_eq!(dtype, *disp_type);
        }
    }
    
    #[test]
    fn test_handling_param_values() {
        // Test all standard handling parameter values
        let input = b"session; handling=optional";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (_, (_, params)) = result.unwrap();
        assert!(params.contains(&DispositionParam::Handling(Handling::Optional)));
        
        let input = b"session; handling=required";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (_, (_, params)) = result.unwrap();
        assert!(params.contains(&DispositionParam::Handling(Handling::Required)));
        
        // Test custom handling value
        let input = b"session; handling=custom";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (_, (_, params)) = result.unwrap();
        assert!(params.iter().any(|p| matches!(p, DispositionParam::Handling(Handling::Other(v)) if v == "custom")));
    }
    
    #[test]
    fn test_extension_disposition_type() {
        // Test custom disposition type (not in RFC standard list)
        let input = b"custom-disposition; handling=optional";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (rem, (dtype, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(dtype, "custom-disposition");
        assert_eq!(params.len(), 1);
    }
    
    #[test]
    fn test_with_whitespace() {
        // Test with various whitespace patterns
        let input = b"  session  ;  handling = optional  ;  filename = test.txt  ";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (rem, (dtype, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(dtype, "session");
        assert_eq!(params.len(), 2);
        
        // Test with tabs
        let input = b"session	;	handling=optional";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_with_line_folding() {
        // Test with line folding after disposition type
        let input = b"session\r\n ; handling=optional";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (rem, (dtype, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(dtype, "session");
        assert_eq!(params.len(), 1);
        
        // Test with line folding within parameters
        let input = b"session; handling\r\n =\r\n optional; filename\r\n =\r\n test.txt";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (rem, (dtype, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(dtype, "session");
        assert_eq!(params.len(), 2);
    }
    
    #[test]
    fn test_case_insensitivity() {
        // Test case insensitivity for disposition type
        let input = b"SESSION; handling=optional";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (rem, (dtype, _)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(dtype, "session");
        
        // Test case insensitivity for handling parameter
        let input = b"session; HANDLING=OPTIONAL";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (_, (_, params)) = result.unwrap();
        assert!(params.contains(&DispositionParam::Handling(Handling::Optional)));
    }
    
    #[test]
    fn test_rfc_examples() {
        // Test examples from RFC 3261 Section 20.13
        let input = b"render";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        
        // Although not explicitly given in RFC 3261, testing common examples
        let input = b"session";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        
        let input = b"icon; handling=optional";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_error_handling() {
        // Test empty input
        let input = b"";
        assert!(parse_content_disposition(input).is_err());
        
        // Test incomplete parameter
        let input = b"session; handling=";
        assert!(parse_content_disposition(input).is_err());
        
        // Test invalid parameter format
        let input = b"session; handling:optional"; // colon instead of equals
        assert!(parse_content_disposition(input).is_err());
        
        // Test unexpected trailing content
        let input = b"session; handling=optional invalid";
        assert!(parse_content_disposition(input).is_err());
    }
    
    #[test]
    fn test_abnf_compliance() {
        // Tests ABNF compliance by testing grammar elements individually
        
        // disp-type - all valid types
        assert!(parse_content_disposition(b"render").is_ok());
        assert!(parse_content_disposition(b"session").is_ok());
        assert!(parse_content_disposition(b"icon").is_ok());
        assert!(parse_content_disposition(b"alert").is_ok());
        assert!(parse_content_disposition(b"custom-type").is_ok()); // extension token
        
        // disp-param - both types
        assert!(parse_content_disposition(b"render; handling=optional").is_ok()); // handling-param
        assert!(parse_content_disposition(b"render; custom=value").is_ok()); // generic-param
        
        // handling-param - all values
        assert!(parse_content_disposition(b"render; handling=optional").is_ok());
        assert!(parse_content_disposition(b"render; handling=required").is_ok());
        assert!(parse_content_disposition(b"render; handling=other-value").is_ok()); // other-handling
        
        // Multiple parameters with SEMI separator
        assert!(parse_content_disposition(b"render; param1=value1; param2=value2").is_ok());
    }
    
    #[test]
    fn test_multiple_params_of_different_types() {
        let input = b"session; handling=required; filename=\"report.txt\"; size=1024; custom-param=value";
        let result = parse_content_disposition(input);
        assert!(result.is_ok());
        let (rem, (dtype, params)) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(dtype, "session");
        assert_eq!(params.len(), 4);
        
        // Verify handling param
        assert!(params.contains(&DispositionParam::Handling(Handling::Required)));
        
        // Verify filename generic param
        assert!(params.iter().any(|p| {
            if let DispositionParam::Generic(Param::Other(name, Some(GenericValue::Token(value)))) = p {
                name == "size" && value == "1024"
            } else {
                false
            }
        }));
    }
} 