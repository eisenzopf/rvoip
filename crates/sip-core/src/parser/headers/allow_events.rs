// RFC 6665 Section 7.2 - Allow-Events Header Parser
//
// Allow-Events = "Allow-Events" HCOLON event-type *(COMMA event-type)
// event-type = event-package *( "." event-template )
// event-package = token-nodot
// event-template = token-nodot
// token-nodot = 1*( alphanum / "-" / "!" / "%" / "*" / "_" / "+" / "`" / "'" / "~" )

// Remove unused import - we're using separated_list1 directly
use crate::parser::token::token;
use crate::parser::whitespace::{lws, sws};
use crate::parser::ParseResult;
use crate::types::allow_events::AllowEvents;
use nom::bytes::complete::{take_while1, is_not};
use nom::character::complete::char;
use nom::combinator::{map, opt, recognize};
use nom::multi::separated_list1;
use nom::sequence::{preceded, separated_pair, tuple};

/// Check if a byte is valid for token-nodot (excludes '.')
fn is_token_nodot_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || 
    matches!(c, b'-' | b'!' | b'%' | b'*' | b'_' | b'+' | b'`' | b'\'' | b'~')
}

/// Parse a token-nodot
fn token_nodot(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(is_token_nodot_char)(input)
}

/// Parse an event-type (event-package with optional dot-separated templates)
/// event-type = event-package *( "." event-template )
fn event_type(input: &[u8]) -> ParseResult<String> {
    map(
        recognize(
            tuple((
                token_nodot,
                nom::multi::many0(
                    preceded(char('.'), token_nodot)
                )
            ))
        ),
        |bytes| std::str::from_utf8(bytes).unwrap_or("").to_string()
    )(input)
}

/// Parse an Allow-Events header value
///
/// # ABNF
/// ```abnf
/// Allow-Events = "Allow-Events" HCOLON event-type *(COMMA event-type)
/// ```
///
/// # Example
/// ```text
/// Allow-Events: presence, message-summary, dialog
/// Allow-Events: presence.winfo, conference
/// ```
pub fn parse_allow_events(input: &[u8]) -> ParseResult<AllowEvents> {
    // Parse any leading whitespace
    let (input, _) = sws(input)?;
    
    // Parse comma-separated list of event types
    let (input, events) = separated_list1(
        tuple((sws, char(','), sws)),
        event_type
    )(input)?;
    
    // Parse any trailing whitespace
    let (input, _) = sws(input)?;
    
    Ok((input, AllowEvents::new(events)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_allow_events_single() {
        let input = b"presence";
        let (rem, allow) = parse_allow_events(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(allow.events(), &["presence"]);
    }
    
    #[test]
    fn test_parse_allow_events_multiple() {
        let input = b"presence, dialog, message-summary";
        let (rem, allow) = parse_allow_events(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(allow.events().len(), 3);
        assert!(allow.supports("presence"));
        assert!(allow.supports("dialog"));
        assert!(allow.supports("message-summary"));
    }
    
    #[test]
    fn test_parse_allow_events_with_templates() {
        let input = b"presence.winfo, conference.floor";
        let (rem, allow) = parse_allow_events(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(allow.events(), &["presence.winfo", "conference.floor"]);
    }
    
    #[test]
    fn test_parse_allow_events_with_spaces() {
        let input = b"  presence  ,  dialog  ,  refer  ";
        let (rem, allow) = parse_allow_events(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(allow.events().len(), 3);
    }
    
    #[test]
    fn test_parse_allow_events_complex() {
        let input = b"presence, presence.winfo, dialog, message-summary, conference";
        let (rem, allow) = parse_allow_events(input).unwrap();
        assert!(rem.is_empty());
        assert_eq!(allow.events().len(), 5);
    }
    
    #[test]
    fn test_parse_allow_events_empty_fails() {
        let input = b"";
        assert!(parse_allow_events(input).is_err());
    }
}