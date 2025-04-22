use nom::{
    bytes::complete::take_while1,
    IResult,
};

// Type alias for parser result
pub type ParseResult<'a, O> = IResult<&'a [u8], O>;

fn is_token_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || 
    c == b'-' || c == b'.' || c == b'!' || c == b'%' || c == b'*' || 
    c == b'_' || c == b'+' || c == b'`' || c == b'\'' || c == b'~'
}

pub fn token(input: &[u8]) -> ParseResult<&[u8]> {
    // token = 1*(alphanum / "-" / "." / "!" / "%" / "*" / "_" / "+" / "`" / "'" / "~")
    take_while1(is_token_char)(input)
}

fn is_word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || 
    c == b'-' || c == b'.' || c == b'!' || c == b'%' || c == b'*' || 
    c == b'_' || c == b'+' || c == b'`' || c == b'\'' || c == b'~' || 
    c == b'(' || c == b')' || c == b'<' || c == b'>' || c == b':' || 
    c == b'\\' || c == b'"' || c == b'/' || c == b'[' || c == b']' || 
    c == b'?' || c == b'{' || c == b'}'
}

pub fn word(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(is_word_char)(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_token_char() {
        // Test alphanumeric
        for c in b'a'..=b'z' {
            assert!(is_token_char(c));
        }
        for c in b'A'..=b'Z' {
            assert!(is_token_char(c));
        }
        for c in b'0'..=b'9' {
            assert!(is_token_char(c));
        }
        
        // Test allowed special chars
        let special_chars = b"-._!%*+`'~";
        for &c in special_chars {
            assert!(is_token_char(c));
        }
        
        // Test disallowed chars
        let disallowed = b"()<>@,;:\\\"/?={}[] \t\r\n";
        for &c in disallowed {
            assert!(!is_token_char(c));
        }
    }
    
    #[test]
    fn test_token_parser() {
        // Valid tokens
        let (rem, tok) = token(b"token123").unwrap();
        assert!(rem.is_empty());
        assert_eq!(tok, b"token123");
        
        let (rem, tok) = token(b"a.b-c_d!e%f*g+h`i'j~k rest").unwrap();
        assert_eq!(rem, b" rest");
        assert_eq!(tok, b"a.b-c_d!e%f*g+h`i'j~k");
        
        // Invalid - empty input
        assert!(token(b"").is_err());
        
        // Invalid - starts with non-token char
        assert!(token(b" token").is_err());
    }
    
    #[test]
    fn test_is_word_char() {
        // Token chars should be valid word chars
        for c in b'a'..=b'z' {
            assert!(is_word_char(c));
        }
        
        // Additional word chars
        let word_specific = b"()<>:\\\"/?[]{}";
        for &c in word_specific {
            assert!(is_word_char(c));
            assert!(!is_token_char(c));
        }
    }
    
    #[test]
    fn test_word_parser() {
        // Valid words
        let (rem, word_val) = word(b"word(with)special<chars>").unwrap();
        assert!(rem.is_empty());
        assert_eq!(word_val, b"word(with)special<chars>");
        
        // With remainder
        let (rem, word_val) = word(b"word/with/slashes another_word").unwrap();
        assert_eq!(rem, b" another_word");
        assert_eq!(word_val, b"word/with/slashes");
        
        // Invalid - empty input
        assert!(word(b"").is_err());
        
        // Invalid - starts with space
        assert!(word(b" word").is_err());
    }
} 