use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    combinator::recognize,
    multi::many0,
    sequence::{delimited, preceded, terminated, tuple, pair},
    IResult,
};

use super::whitespace::sws;

// Type alias for parser result
pub type ParseResult<'a, O> = IResult<&'a [u8], O>;


fn is_separator_char(c: u8) -> bool {
    c == b'(' || c == b')' || c == b'<' || c == b'>' || c == b'@' ||
    c == b',' || c == b';' || c == b':' || c == b'\\' || c == b'"' ||
    c == b'/' || c == b'[' || c == b']' || c == b'?' || c == b'=' ||
    c == b'{' || c == b'}' || c == b' ' || c == b'\t'
}

pub fn separators(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(is_separator_char)(input)
}

pub fn hcolon(input: &[u8]) -> ParseResult<&[u8]> {
    // HCOLON = *( SP / HTAB ) ":" SWS
    recognize(tuple((many0(alt((tag(b" "), tag(b"\t")))), tag(b":"), sws)))(input)
}

pub fn dquote(input: &[u8]) -> ParseResult<&[u8]> {
    tag(b"\"")(input)
}

// Separator wrappers with SWS
pub fn star(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(tuple((sws, tag(b"*"), sws)))(input)
}

pub fn slash(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(tuple((sws, tag(b"/"), sws)))(input)
}

pub fn equal(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(tuple((sws, tag(b"="), sws)))(input)
}

pub fn lparen(input: &[u8]) -> ParseResult<&[u8]> {
    // LPAREN = SWS "("
    // RFC 3261 defines LPAREN as optional whitespace followed by open paren
    recognize(pair(sws, tag(b"(")))(input)
}

pub fn rparen(input: &[u8]) -> ParseResult<&[u8]> {
    // RPAREN = ")" SWS
    // RFC 3261 defines RPAREN as close paren followed by optional whitespace
    recognize(pair(tag(b")"), sws))(input)
}

pub fn raquot(input: &[u8]) -> ParseResult<&[u8]> {
    // RAQUOT = ">" SWS
    recognize(pair(tag(b">"), sws))(input)
}

pub fn laquot(input: &[u8]) -> ParseResult<&[u8]> {
    // LAQUOT = SWS "<"
    recognize(pair(sws, tag(b"<")))(input)
}

pub fn comma(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(tuple((sws, tag(b","), sws)))(input)
}

pub fn semi(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(tuple((sws, tag(b";"), sws)))(input)
}

pub fn colon(input: &[u8]) -> ParseResult<&[u8]> {
    recognize(tuple((sws, tag(b":"), sws)))(input)
}

pub fn ldquot(input: &[u8]) -> ParseResult<&[u8]> {
    // LDQUOT = SWS DQUOTE
    recognize(pair(sws, dquote))(input)
}

pub fn rdquot(input: &[u8]) -> ParseResult<&[u8]> {
    // RDQUOT = DQUOTE SWS
    recognize(pair(dquote, sws))(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::error::ErrorKind;

    #[test]
    fn test_is_separator_char() {
        // Test each separator character
        let separators = "()<>@,;:\\\"/?[]{}= \t";
        for c in separators.bytes() {
            assert!(is_separator_char(c), "Character '{}' should be a separator", c as char);
        }
        
        // Test some non-separator characters
        let non_separators = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!#$%&'*+-.^_`|~";
        for c in non_separators.bytes() {
            assert!(!is_separator_char(c), "Character '{}' should not be a separator", c as char);
        }
    }

    #[test]
    fn test_separators_parser() {
        // Test with single separator
        let (rem, val) = separators(b"(rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"(");
        
        // Test with multiple separators
        let (rem, val) = separators(b"<>@,;:rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"<>@,;:");
        
        // Test with invalid input
        assert!(separators(b"").is_err());
        assert!(separators(b"abc").is_err());
    }

    #[test]
    fn test_hcolon() {
        // RFC 3261 Section 25.1 defines HCOLON = *( SP / HTAB ) ":" SWS
        
        // Basic case
        let (rem, val) = hcolon(b":rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b":");
        
        // With spaces before colon
        let (rem, val) = hcolon(b"   :rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"   :");
        
        // With tabs before colon
        let (rem, val) = hcolon(b"\t\t:rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"\t\t:");
        
        // With mixed spaces and tabs
        let (rem, val) = hcolon(b" \t :rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b" \t :");
        
        // With SWS after colon
        let (rem, val) = hcolon(b": rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b": ");
        
        // With complex SWS after colon (including line folding)
        let (rem, val) = hcolon(b":\r\n rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b":\r\n ");
        
        // Invalid cases
        assert!(hcolon(b"").is_err());
        assert!(hcolon(b"rest").is_err());
    }

    #[test]
    fn test_dquote() {
        // Simple double-quote
        let (rem, val) = dquote(b"\"rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"\"");
        
        // Invalid cases
        assert!(dquote(b"").is_err());
        assert!(dquote(b"rest").is_err());
    }

    #[test]
    fn test_star() {
        // RFC spec allows for whitespace on both sides, implemented with delimited(sws, tag(b"*"), sws)
        
        // Basic case - no whitespace
        let (rem, val) = star(b"*rest").unwrap();
        assert_eq!(rem, b"rest");
        // With delimited(sws, tag(b"*"), sws), even with no whitespace, should just return "*"
        assert_eq!(val, b"*");
        
        // With SWS before and after - should include all whitespace as per RFC
        let (rem, val) = star(b" * rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include all whitespace on both sides
        assert_eq!(val, b" * ");
        
        // With complex SWS with folding
        let (rem, val) = star(b"\r\n * rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include all whitespace including line folding
        assert_eq!(val, b"\r\n * ");
        
        // Invalid cases
        assert!(star(b"").is_err());
        assert!(star(b"rest").is_err());
    }

    #[test]
    fn test_lparen_rparen() {
        // RFC 3261 defines:
        // LPAREN = SWS "("   (whitespace before but NOT after)
        // RPAREN = ")" SWS   (whitespace after but NOT before)
        
        // Test LPAREN - basic, no whitespace
        let (rem, val) = lparen(b"(rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"("); // No whitespace, just paren
        
        // LPAREN with SWS before - should include whitespace before
        let (rem, val) = lparen(b" (rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace before the paren
        assert_eq!(val, b" (");
        
        // LPAREN with complex SWS - should include all whitespace before
        let (rem, val) = lparen(b"\r\n (rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include all whitespace before
        assert_eq!(val, b"\r\n (");
        
        // Test RPAREN - basic, no whitespace
        let (rem, val) = rparen(b")rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b")"); // No whitespace, just paren
        
        // RPAREN with SWS after - should include whitespace after
        let (rem, val) = rparen(b") rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace after
        assert_eq!(val, b") ");
        
        // RPAREN with complex SWS - should include all whitespace after
        let (rem, val) = rparen(b")\r\n rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include all whitespace after
        assert_eq!(val, b")\r\n ");
        
        // RFC compliance for SWS positioning:
        
        // LPAREN should not consume whitespace AFTER '(' - it should remain in the input
        let (rem, val) = lparen(b"( rest").unwrap();
        assert_eq!(rem, b" rest"); // Whitespace after remains in input
        assert_eq!(val, b"(");
        
        // RPAREN should not consume whitespace BEFORE ')' - it should fail
        let input = b" )rest";
        assert!(rparen(input).is_err()); // Cannot parse with whitespace before
    }

    #[test]
    fn test_laquot_raquot() {
        // RFC 3261 defines:
        // LAQUOT = SWS "<"   (whitespace before but NOT after)
        // RAQUOT = ">" SWS   (whitespace after but NOT before)
        
        // Test LAQUOT - basic, no whitespace
        let (rem, val) = laquot(b"<rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"<"); // No whitespace, just angle bracket
        
        // LAQUOT with SWS before - should include whitespace before
        let (rem, val) = laquot(b" <rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace before
        assert_eq!(val, b" <");
        
        // LAQUOT with complex SWS - should include all whitespace before
        let (rem, val) = laquot(b"\r\n <rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include all whitespace before
        assert_eq!(val, b"\r\n <");
        
        // Test RAQUOT - basic, no whitespace
        let (rem, val) = raquot(b">rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b">"); // No whitespace, just angle bracket
        
        // RAQUOT with SWS after - should include whitespace after
        let (rem, val) = raquot(b"> rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace after
        assert_eq!(val, b"> ");
        
        // RAQUOT with complex SWS - should include all whitespace after
        let (rem, val) = raquot(b">\r\n rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include all whitespace after
        assert_eq!(val, b">\r\n ");
        
        // RFC compliance for SWS positioning:
        
        // LAQUOT should not consume whitespace AFTER '<' - it should remain in the input
        let (rem, val) = laquot(b"< rest").unwrap();
        assert_eq!(rem, b" rest"); // Whitespace after remains in input
        assert_eq!(val, b"<");
        
        // RAQUOT should not consume whitespace BEFORE '>' - it should fail
        let input = b" >rest";
        assert!(raquot(input).is_err()); // Cannot parse with whitespace before
    }

    #[test]
    fn test_ldquot_rdquot() {
        // RFC 3261 defines:
        // LDQUOT = SWS DQUOTE   (whitespace before but NOT after)
        // RDQUOT = DQUOTE SWS   (whitespace after but NOT before)
        
        // Test LDQUOT - basic, no whitespace
        let (rem, val) = ldquot(b"\"rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"\""); // No whitespace, just quote
        
        // LDQUOT with SWS before - should include whitespace before
        let (rem, val) = ldquot(b" \"rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace before
        assert_eq!(val, b" \"");
        
        // LDQUOT with complex SWS - should include all whitespace before
        let (rem, val) = ldquot(b"\r\n \"rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include all whitespace before
        assert_eq!(val, b"\r\n \"");
        
        // Test RDQUOT - basic, no whitespace
        let (rem, val) = rdquot(b"\"rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"\""); // No whitespace, just quote
        
        // RDQUOT with SWS after - should include whitespace after
        let (rem, val) = rdquot(b"\" rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace after
        assert_eq!(val, b"\" ");
        
        // RDQUOT with complex SWS - should include all whitespace after
        let (rem, val) = rdquot(b"\"\r\n rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include all whitespace after
        assert_eq!(val, b"\"\r\n ");
        
        // RFC compliance verification:
        
        // LDQUOT should not consume whitespace AFTER quote - it should remain in the input
        let (rem, val) = ldquot(b"\" rest").unwrap();
        assert_eq!(rem, b" rest"); // Whitespace after remains in input
        assert_eq!(val, b"\"");
        
        // RDQUOT should not consume whitespace BEFORE quote - it should fail
        let input = b" \"rest";
        assert!(rdquot(input).is_err()); // Cannot parse with whitespace before
    }

    #[test]
    fn test_comma_semi_colon() {
        // RFC 3261 allows SWS on both sides of comma, semi, and colon
        
        // Test comma with no whitespace
        let (rem, val) = comma(b",rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b","); // No whitespace
        
        // Comma with SWS on both sides - should include all whitespace
        let (rem, val) = comma(b" , rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace on both sides
        assert_eq!(val, b" , ");
        
        // Test semi with no whitespace
        let (rem, val) = semi(b";rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b";"); // No whitespace
        
        // Semi with SWS on both sides - should include all whitespace
        let (rem, val) = semi(b" ; rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace on both sides
        assert_eq!(val, b" ; ");
        
        // Test colon with no whitespace
        let (rem, val) = colon(b":rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b":"); // No whitespace
        
        // Colon with SWS on both sides - should include all whitespace
        let (rem, val) = colon(b" : rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace on both sides
        assert_eq!(val, b" : ");
        
        // With complex SWS (line folding)
        let (rem, val) = comma(b"\r\n ,\r\n rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include all whitespace
        assert_eq!(val, b"\r\n ,\r\n ");
    }

    #[test]
    fn test_rfc4475_separator_examples() {
        // Examples from RFC 4475 torture test messages
        
        // From 3.1.1.8 - excessive whitespace
        let (rem, val) = comma(b"  ,   rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should contain all whitespace
        assert_eq!(val, b"  ,   ");
        
        // From 3.1.1.10 - HCOLON with whitespace before and after colon
        let (rem, val) = hcolon(b"   : rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should contain all whitespace
        assert_eq!(val, b"   : ");
        
        // From 3.1.2.2 - Semi with whitespace
        let (rem, val) = semi(b" ; rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should contain all whitespace
        assert_eq!(val, b" ; ");
    }

    #[test]
    fn test_rfc3261_section25_examples() {
        // From RFC 3261 Section 25.1
        
        // Example with HCOLON:
        // Subject: I know you're there
        let (rem, val) = hcolon(b": rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b": ");
        
        // Example with parentheses in comments:
        // From: Bob (Bob Smith) <sip:bob@example.com>
        let (rem, val) = lparen(b"(Bob Smith)").unwrap();
        assert_eq!(rem, b"Bob Smith)");
        assert_eq!(val, b"(");
        
        let (rem, val) = rparen(b")rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b")");
        
        // Example with angle brackets in name-addr:
        // To: Alice <sip:alice@example.com>
        let (rem, val) = laquot(b"<sip:alice@example.com>").unwrap();
        assert_eq!(rem, b"sip:alice@example.com>");
        assert_eq!(val, b"<");
        
        let (rem, val) = raquot(b">rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b">");
    }

    #[test]
    fn test_equal_slash() {
        // RFC 3261 allows SWS on both sides of equal and slash
        
        // Test equal with no whitespace
        let (rem, val) = equal(b"=rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"="); // No whitespace
        
        // Equal with SWS on both sides - should include all whitespace
        let (rem, val) = equal(b" = rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace on both sides
        assert_eq!(val, b" = ");
        
        // Test slash with no whitespace
        let (rem, val) = slash(b"/rest").unwrap();
        assert_eq!(rem, b"rest");
        assert_eq!(val, b"/"); // No whitespace
        
        // Slash with SWS on both sides - should include all whitespace
        let (rem, val) = slash(b" / rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include whitespace on both sides
        assert_eq!(val, b" / ");
        
        // With complex SWS (line folding)
        let (rem, val) = equal(b"\r\n =\r\n rest").unwrap();
        assert_eq!(rem, b"rest");
        // Should include all whitespace
        assert_eq!(val, b"\r\n =\r\n ");
    }
} 