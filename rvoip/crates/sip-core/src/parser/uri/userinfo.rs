use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{char},
    combinator::{map_res, opt, recognize},
    multi::{many0, many1},
    sequence::{pair, preceded, terminated},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::common_chars::{unreserved, escaped};
use crate::parser::ParseResult;
use crate::parser.utils::unescape_uri_component; // Import unescape helper
use crate::error::Error; // For error type

// user-unreserved = "&" / "=" / "+" / "$" / "," / ";" / "?" / "/"
fn is_user_unreserved(c: u8) -> bool {
    matches!(c, b'&' | b'=' | b'+' | b'$' | b',' | b';' | b'?' | b'/')
}

// user = 1*( unreserved / escaped / user-unreserved )
// Returns raw bytes, unescaping happens in userinfo
fn user(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many1(alt((
        unreserved,
        escaped,
        take_while1(is_user_unreserved),
    ))))(input)
}

// password = *( unreserved / escaped / "&" / "=" / "+" / "$" / "," )
fn is_password_char(c: u8) -> bool {
    unreserved(&[c]).is_ok() || // Check if unreserved
    matches!(c, b'&' | b'=' | b'+' | b'$' | b',')
}

fn password_char(input: &[u8]) -> ParseResult<&[u8]> {
    alt((escaped, take_while1(is_password_char)))(input)
}

// password = *( unreserved / escaped / "&" / "=" / "+" / "$" / "," )
// Returns raw bytes, unescaping happens in userinfo
fn password(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(many0(password_char))(input)
}

// userinfo = user [ ":" password ] "@"
// Returns (unescaped_user_string, Option<unescaped_password_string>)
// Corrected structure: Parses user and optional password, terminated by '@'
pub(crate) fn userinfo(input: &[u8]) -> ParseResult<(String, Option<String>)> {
    map_res(
        terminated(
            pair(user, opt(preceded(tag(b":"), password))),
            tag(b"@")
        ),
        |(user_bytes, pass_opt_bytes)| {
            let user_str = unescape_uri_component(user_bytes)?;
            let pass_str_opt = pass_opt_bytes
                .map(|p| unescape_uri_component(p))
                .transpose()?;
            Ok((user_str, pass_str_opt))
        }
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_userinfo_unescaped() {
        let (rem, (user, pass)) = userinfo(b"user%40example.com:p%40ssw%3rd@").unwrap();
        assert!(rem.is_empty());
        assert_eq!(user, "user@example.com");
        assert_eq!(pass, Some("p@ssw3rd".to_string()));
    }
} 