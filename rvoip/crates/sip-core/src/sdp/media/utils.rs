// Media parsing utilities
//
// Common utility functions used across media parsing

use nom::{
    IResult,
    error::{Error, ErrorKind},
};

/// Tag that is case-insensitive
pub fn tag_no_case<'a>(t: &'a str) -> impl Fn(&'a str) -> IResult<&'a str, &'a str> {
    move |i: &str| {
        let t_len = t.len();
        let t_lower = t.to_lowercase();
        
        if i.len() < t_len {
            return Err(nom::Err::Error(Error::new(
                i,
                ErrorKind::Tag
            )));
        }
        
        let i_prefix_lower = i[..t_len].to_lowercase();
        if i_prefix_lower == t_lower {
            Ok((&i[t_len..], &i[..t_len]))
        } else {
            Err(nom::Err::Error(Error::new(
                i,
                ErrorKind::Tag
            )))
        }
    }
}

/// Validates if a string is a valid token per RFC 4566
pub fn is_valid_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    
    token.chars().all(|c| {
        matches!(c,
            'a'..='z' | 'A'..='Z' | '0'..='9' |
            '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '.' |
            '^' | '_' | '`' | '{' | '|' | '}' | '~'
        )
    })
}

/// Validates that a string contains only valid ID characters
/// (restricted token subset) as per various RFCs
pub fn is_valid_id(id: &str) -> bool {
    // ID characters are a more restricted set than token characters
    // Usually alphanumeric plus possibly some punctuation
    
    if id.is_empty() {
        return false;
    }
    
    id.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'
    })
}

/// Validates ICE candidate related identifiers
pub fn is_valid_ice_char(c: char) -> bool {
    // ICE identifiers use a specific set of characters
    // Usually alphanumeric plus some punctuation
    c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '-'
}

/// Special validator for ICE ufrag and password fields
pub fn is_valid_ice_string(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    
    s.chars().all(is_valid_ice_char)
} 