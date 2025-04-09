use std::collections::HashMap;

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_while, take_while1},
    character::complete::{alpha1, alphanumeric1, char, digit1, space0, space1},
    combinator::{map, map_res, opt, recognize, verify},
    multi::{many0, many1, separated_list0, separated_list1},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};

use crate::error::{Error, Result};

/// Parser for CRLF (both \r\n and \n are accepted)
pub fn crlf(input: &str) -> IResult<&str, &str> {
    alt((tag("\r\n"), tag("\n")))(input)
}

/// Parser for a parameter name
pub fn parse_param_name(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| {
        c.is_alphanumeric() || 
        c == '-' || c == '_' || c == '.' || c == '+' || 
        c == '!' || c == '~' || c == '*' || c == '\''
    })(input)
}

/// Parser for a parameter value
pub fn parse_param_value(input: &str) -> IResult<&str, &str> {
    take_till(|c| c == ';' || c == ',' || c == '\r' || c == '\n')(input)
}

/// Parse a list of comma-separated values with optional whitespace
pub fn parse_comma_separated_values(input: &str) -> IResult<&str, Vec<&str>> {
    separated_list0(
        tuple((char(','), space0)),
        take_till(|c| c == ',' || c == '\r' || c == '\n')
    )(input)
}

/// Parser for a token value (alphanumeric, plus some special chars)
pub fn parse_token(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| {
        c.is_alphanumeric() ||
        c == '-' || c == '_' || c == '.' || c == '+' || c == '~' ||
        c == '!' || c == '*' || c == '\'' || c == '(' || c == ')'
    })(input)
}

/// Parser for a quoted string
pub fn parse_quoted_string(input: &str) -> IResult<&str, &str> {
    delimited(
        char('"'),
        take_till(|c| c == '"'),
        char('"')
    )(input)
}

/// Parser for a text value (either quoted or token)
pub fn parse_text_value(input: &str) -> IResult<&str, &str> {
    alt((
        parse_quoted_string,
        parse_token
    ))(input)
}

/// Parse all parameters in semicolon-delimited format: ;name=value;flag
pub fn parse_semicolon_params(input: &str) -> IResult<&str, HashMap<String, String>> {
    map(
        many0(
            preceded(
                char(';'),
                alt((
                    separated_pair(
                        map(parse_param_name, |s| s.to_string()),
                        char('='),
                        map(parse_param_value, |s| s.to_string())
                    ),
                    map(
                        parse_param_name,
                        |name| (name.to_string(), "".to_string())
                    )
                ))
            )
        ),
        |params| params.into_iter().collect()
    )(input)
}

/// Clone without lifetime - helper for string handling
pub fn clone_str(s: &str) -> String {
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_param_name() {
        assert_eq!(parse_param_name("branch").unwrap().1, "branch");
        assert_eq!(parse_param_name("user-agent").unwrap().1, "user-agent");
        assert_eq!(parse_param_name("extension.param").unwrap().1, "extension.param");
    }
    
    #[test]
    fn test_parse_param_value() {
        assert_eq!(parse_param_value("value").unwrap().1, "value");
        assert_eq!(parse_param_value("value;next").unwrap().1, "value");
        assert_eq!(parse_param_value("value,next").unwrap().1, "value");
    }
    
    #[test]
    fn test_parse_semicolon_params() {
        let input = ";branch=z9hG4bK776asdhds;received=10.0.0.1;rport";
        let (_, params) = parse_semicolon_params(input).unwrap();
        
        assert_eq!(params.get("branch").unwrap(), "z9hG4bK776asdhds");
        assert_eq!(params.get("received").unwrap(), "10.0.0.1");
        assert_eq!(params.get("rport").unwrap(), "");
    }
    
    #[test]
    fn test_parse_comma_separated_values() {
        let input = "value1, value2,value3 , value4";
        let (_, values) = parse_comma_separated_values(input).unwrap();
        
        assert_eq!(values.len(), 4);
        assert_eq!(values[0], "value1");
        assert_eq!(values[1], " value2");
        assert_eq!(values[2], "value3 ");
        assert_eq!(values[3], " value4");
    }
} 