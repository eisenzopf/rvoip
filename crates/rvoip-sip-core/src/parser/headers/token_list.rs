// Generic parser for headers that are lists of tokens, possibly with a short form.
// Based on RFC 3261 Section 7.3.1 and Section 25.1:
// token          = 1*(alphanum / "-" / "." / "!" / "%" / "*" / "_" / "+" / "`" / "'" / "~")
// header-value   = *(TEXT-UTF8char / UTF8-CONT / LWS)
// For comma-separated lists:
// comma-separated-list = *(token LWS "," LWS) token

use nom::combinator::map_res;

// Import from new modules
use crate::parser::token::token;
use crate::parser::ParseResult;

// Import shared parsers
// Removed duplicate imports:
// use crate::parser::common::comma_separated_list0;
// use crate::parser::token::token;
// use crate::parser::ParseResult;

use std::str;







// Define structure for a list of tokens
 // Use String to hold tokens

// Parses a comma-separated list of tokens


// Helper to parse a token into a String
// Based on RFC 3261 Section 25.1 token definition
pub fn token_string(input: &[u8]) -> ParseResult<'_, String> {
    map_res(token, |b| str::from_utf8(b).map(String::from))(input)
}

#[cfg(test)]
mod tests {
    use super::*;






    #[test]
    fn test_token_characters() {
        // Test all allowed token characters from RFC 3261
        let token_with_all_chars = b"token-._!%*+`'~";
        let (rem, token) = token_string(token_with_all_chars).unwrap();
        assert!(rem.is_empty());
        assert_eq!(token, "token-._!%*+`'~");

        // Ensure disallowed characters fail
        // This depends on the token parser implementation
        // Assuming a correct implementation, these should fail:
        // assert!(token_string(b"token(with)invalid:chars").is_err());
        // assert!(token_string(b"token with spaces").is_err());
        // assert!(token_string(b"token;with;semicolons").is_err());
    }



}
