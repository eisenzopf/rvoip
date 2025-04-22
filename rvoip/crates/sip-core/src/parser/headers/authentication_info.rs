// RFC 3261 Section 22.5 Authentication-Info

use super::auth::common::ainfo;
use crate::parser::common::comma_separated_list1;
use crate::parser::ParseResult;
use crate::types::auth::AuthenticationInfoParam;
use nom::IResult;
use crate::parser::headers::auth::common::parse_auth_param;
use nom::multi::separated_list1;
use nom::sequence::tuple;
use nom::bytes::complete::tag;
use crate::parser::whitespace::lws as LWS;

// Authentication-Info = "Authentication-Info" HCOLON auth-param *(COMMA auth-param)
pub fn parse_authentication_info(input: &[u8]) -> ParseResult<Vec<AuthenticationInfoParam>> {
    let separator = tuple((LWS, tag(b","), LWS));
    separated_list1(separator, parse_auth_param)(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::{Qop};

    #[test]
    fn test_parse_authentication_info() {
        let input = br#"nextnonce="fedcba98", qop=auth, rspauth="abcdef01", cnonce="abc", nc=00000001"#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 5);
        assert!(params.contains(&AuthenticationInfoParam::NextNonce("fedcba98".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::Qop(Qop::Auth)));
        assert!(params.contains(&AuthenticationInfoParam::ResponseAuth("abcdef01".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::Cnonce("abc".to_string())));
        assert!(params.contains(&AuthenticationInfoParam::NonceCount(1)));
    }
     #[test]
    fn test_parse_authentication_info_single() {
        let input = br#"nextnonce="12345678""#;
        let result = parse_authentication_info(input);
        assert!(result.is_ok());
        let (rem, params) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(params.len(), 1);
        assert!(params.contains(&AuthenticationInfoParam::NextNonce("12345678".to_string())));
    }
} 