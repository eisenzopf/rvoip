// Shared parser for server-val structure (RFC 3261 Section 20.33/20.41)

use nom::{
    branch::alt,
    bytes::complete::tag,
    combinator::{map, map_res, opt},
    sequence::{pair, preceded},
    IResult,
};
use std::str;

// Import shared parsers
use crate::parser::token::token;
use crate::parser::quoted::comment;
use crate::parser::separators::slash;
use crate::parser::ParseResult;

// Import the types from the types module
use crate::types::server::{ServerVal, Product};

// Create an alias for compatibility with existing code
pub type ServerValComponent = ServerVal;

// product-version = token
fn product_version(input: &[u8]) -> ParseResult<String> {
    map_res(token, |bytes| str::from_utf8(bytes).map(String::from))(input)
}

// product = token [SLASH product-version]
fn product(input: &[u8]) -> ParseResult<Product> {
    map_res(
        pair(token, opt(preceded(slash, product_version))),
        |(name_bytes, version_opt)| {
            let name = str::from_utf8(name_bytes)?.to_string();
            Ok(Product { name, version: version_opt })
        }
    )(input)
}

// server-val = product / comment
pub fn server_val(input: &[u8]) -> ParseResult<ServerVal> {
    alt((
        map(product, ServerVal::Product),
        // Assuming comment parser returns the content as String
        map_res(comment, |bytes| str::from_utf8(bytes).map(|s| ServerVal::Comment(s.to_string())))
    ))(input)
}

// Alias for function to help with migration
pub fn server_val_parser(input: &[u8]) -> ParseResult<ServerVal> {
    server_val(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_product() {
        let (rem, p) = product(b"MyServer/1.0").unwrap();
        assert!(rem.is_empty());
        assert_eq!(p.name, "MyServer");
        assert_eq!(p.version, Some("1.0".to_string()));

        let (rem_no_ver, p_no_ver) = product(b"SomeAgent").unwrap();
        assert!(rem_no_ver.is_empty());
        assert_eq!(p_no_ver.name, "SomeAgent");
        assert_eq!(p_no_ver.version, None);
    }
    
    #[test]
    fn test_server_val() {
        let (rem_p, sv_p) = server_val(b"Product/V2").unwrap();
        assert!(rem_p.is_empty());
        assert!(matches!(sv_p, ServerVal::Product(p) if p.name == "Product" && p.version == Some("V2".to_string())));

        let (rem_c, sv_c) = server_val(b"(internal build)").unwrap();
        assert!(rem_c.is_empty());
        assert!(matches!(sv_c, ServerVal::Comment(c) if c == "internal build"));
    }
} 