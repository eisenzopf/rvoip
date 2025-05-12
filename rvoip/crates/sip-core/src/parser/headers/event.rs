// Event Header Field
//
// The Event header field is used to indicate a subscription to or notification of a
// resource event. It is defined in RFC 6665, which obsoletes RFC 3265.
//
// ABNF (from RFC 6665):
// Event            = "Event" HCOLON event-type *( SEMI event-param )
// event-type       = LAQUOT event-package RAQUOT / token
// event-package    = token
// event-param      = generic-param / ( "id" EQUAL token )
//
// Note: generic-param is defined in RFC 3261.
// generic-param    = token [ EQUAL ( token / host / quoted-string ) ]

// Adjusted imports based on parser module structure
use crate::parser::token::token;
use crate::parser::separators::{laquot, raquot, hcolon}; // CHANGED to hcolon
use crate::parser::common_params::semicolon_params0;
use crate::types::param::{Param as RichParam, GenericValue as RichGenericValue}; // For processing semicolon_params0 output

// Import main Error type for the crate and Hdr (HeaderName, TypedHeader) + Event types
use crate::Error;
use crate::types::header::{Header, HeaderValue, TypedHeaderTrait};
use crate::types::headers::HeaderName;
use crate::types::event::{Event, EventType, Params, ParamValue};

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    combinator::{map, opt, eof},
    sequence::{delimited, pair, preceded, tuple, terminated},
    IResult,
    error::Error as NomError,
    character::complete::multispace0 // For SWS/OWS
};
use std::fmt;
use std::collections::BTreeMap;

// EventType enum, Event struct, and their impls are MOVED to types/headers/event.rs

// fmt::Display for Event is MOVED to types/headers/event.rs

// Parser for event-type: LAQUOT event-package RAQUOT / token
fn parse_event_type_value(input: &[u8]) -> IResult<&[u8], EventType, NomError<&[u8]>> { 
    alt((
        map(
            delimited(laquot, token, raquot),
            |pkg_bytes: &[u8]| EventType::Package(String::from_utf8_lossy(pkg_bytes).into_owned())
        ),
        map(
            token,
            |tok_bytes: &[u8]| EventType::Token(String::from_utf8_lossy(tok_bytes).into_owned())
        ),
    ))(input)
}

// Parser for event parameters using semicolon_params0
fn parse_event_params(input: &[u8]) -> IResult<&[u8], (Option<String>, Params), NomError<&[u8]>> { 
    map(semicolon_params0, |rich_params_vec: Vec<RichParam>| {
        let mut id_val = None;
        let mut other_params_map = Params::new();

        for rich_param in rich_params_vec {
            if let RichParam::Other(key_str, opt_rich_gen_val) = rich_param {
                if key_str.eq_ignore_ascii_case("id") && id_val.is_none() {
                    if let Some(rich_gen_val) = opt_rich_gen_val {
                        id_val = Some(rich_gen_val.to_string()); 
                    } else {
                        other_params_map.insert(key_str.clone(), ParamValue::None);
                    }
                } else {
                    let local_param_val = match opt_rich_gen_val {
                        Some(rich_gen_val) => ParamValue::Value(rich_gen_val.to_string()),
                        None => ParamValue::None,
                    };
                    other_params_map.insert(key_str, local_param_val);
                }
            } 
        }
        (id_val, other_params_map)
    })(input)
}

/// Parses the value part of an Event header (the content after "Event: ").
/// Example: "presence;id=123"
/// Returns types::headers::event::Event
pub fn parse_event_header_value(input: &[u8]) -> IResult<&[u8], Event, NomError<&[u8]>> { 
    let core_event_parser = map(
        tuple((
            parse_event_type_value,
            opt(parse_event_params) 
        )),
        |(event_type, params_opt)| {
            let (id, params) = params_opt.unwrap_or_else(|| (None, Params::new()));
            Event { 
                event_type,
                id,
                params,
            }
        }
    );

    // The event value itself, then ensure only optional whitespace and EOF remain.
    map(
        terminated(core_event_parser, tuple((multispace0, eof))), 
        |event| event // event is the output of core_event_parser
    )(input)
}

/// Parses a full Event header line, including the "Event:" part.
/// Example: "Event: <conference>;id=conf-xyz"
/// Returns types::headers::event::Event
pub fn parse_event_header(input: &[u8]) -> IResult<&[u8], Event, NomError<&[u8]>> { 
    preceded(
        pair(tag_no_case(b"Event"), hcolon), // Use hcolon
        parse_event_header_value
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*; // Imports local ParamValue, Params, and parser functions
                  // Also imports Event, EventType from types via the use statement at file top

    // TypedHeader::parse is now TypedHeaderTrait::parse_value
    // parse_event_header_value is the main public value parser from this module.
    // Event::parse (the method from TypedHeaderTrait) will call parse_event_header_value.

    #[test]
    fn test_parse_event_type_value() {
        let (rem, etype) = parse_event_type_value(b"<presence>").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(etype, EventType::Package("presence".to_string()));

        let (rem, etype) = parse_event_type_value(b"dialog").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(etype, EventType::Token("dialog".to_string()));

        assert!(parse_event_type_value(b" aanwezigheid").is_err(), "Should not parse with leading space");
    }

    #[test]
    fn test_parse_event_header_value_simple_token() {
        let (rem, header) = parse_event_header_value(b"presence").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(header.event_type, EventType::Token("presence".to_string()));
        assert!(header.id.is_none());
        assert!(header.params.is_empty());
    }

    #[test]
    fn test_parse_event_header_value_simple_package() {
        let (rem, header) = parse_event_header_value(b"<conference>").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(header.event_type, EventType::Package("conference".to_string()));
        assert!(header.id.is_none());
        assert!(header.params.is_empty());
    }

    #[test]
    fn test_parse_event_header_value_with_id() {
        let (rem, header) = parse_event_header_value(b"presence;id=123").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(header.event_type, EventType::Token("presence".to_string()));
        assert_eq!(header.id, Some("123".to_string()));
        assert!(header.params.is_empty());
    }
    
    #[test]
    fn test_parse_event_header_value_with_id_case_insensitive_key() {
        let (rem, header) = parse_event_header_value(b"presence;ID=xyz123").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(header.id, Some("xyz123".to_string())); // ID extracted to dedicated field
        assert!(header.params.get("ID").is_none()); // Not in generic params
        assert!(header.params.get("id").is_none());
    }


    #[test]
    fn test_parse_event_header_value_with_package_and_id() {
        let (rem, header) = parse_event_header_value(b"<dialog>;id=xyz").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(header.event_type, EventType::Package("dialog".to_string()));
        assert_eq!(header.id, Some("xyz".to_string()));
        assert!(header.params.is_empty());
    }

    #[test]
    fn test_parse_event_header_value_with_generic_param() {
        let (rem, header) = parse_event_header_value(b"presence;foo=bar").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(header.event_type, EventType::Token("presence".to_string()));
        assert!(header.id.is_none());
        assert_eq!(header.params.get("foo"), Some(&ParamValue::Value("bar".to_string())));
    }
    
    #[test]
    fn test_parse_event_header_value_with_valueless_generic_param() {
        let (rem, header) = parse_event_header_value(b"presence;foo").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(header.event_type, EventType::Token("presence".to_string()));
        assert!(header.id.is_none());
        assert_eq!(header.params.get("foo"), Some(&ParamValue::None));
    }

    #[test]
    fn test_parse_event_header_value_with_id_and_generic_param() {
        let (rem, header) = parse_event_header_value(b"message-summary;id=msg-waiting;custom=value").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(header.event_type, EventType::Token("message-summary".to_string()));
        assert_eq!(header.id, Some("msg-waiting".to_string()));
        assert_eq!(header.params.get("custom"), Some(&ParamValue::Value("value".to_string())));
        assert_eq!(header.params.len(), 1); // 'id' is not in params map
    }
    
    #[test]
    fn test_parse_event_header_value_with_id_and_multiple_generic_params() {
        let (rem, header) = parse_event_header_value(b"presence;id=123;foo=bar;baz").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(header.event_type, EventType::Token("presence".to_string()));
        assert_eq!(header.id, Some("123".to_string()));
        assert_eq!(header.params.get("foo"), Some(&ParamValue::Value("bar".to_string())));
        assert_eq!(header.params.get("baz"), Some(&ParamValue::None));
        assert_eq!(header.params.len(), 2);
    }

    #[test]
    fn test_parse_full_event_header() {
        let (rem, header) = parse_event_header(b"Event: <presence>;id=aBcDeF;some-param=some-value").unwrap();
        assert_eq!(rem, b"");
        assert_eq!(header.event_type, EventType::Package("presence".to_string()));
        assert_eq!(header.id, Some("aBcDeF".to_string()));
        assert_eq!(header.params.get("some-param"), Some(&ParamValue::Value("some-value".to_string())));
    }
    
    #[test]
    fn test_parse_full_event_header_lowercase_name_and_whitespace() {
        let (rem, header) = parse_event_header(b"event \t  :  \t presence  ").unwrap();
        assert_eq!(rem, b""); // Now expects full consumption due to terminated in parse_event_header_value
        assert_eq!(header.event_type, EventType::Token("presence".to_string()));
    }

    #[test]
    fn test_event_display_simple_token() {
        let event = Event::new(EventType::Token("presence".to_string()));
        assert_eq!(event.to_string(), "presence");
    }

    #[test]
    fn test_event_display_simple_package() {
        let event = Event::new(EventType::Package("conference".to_string()));
        assert_eq!(event.to_string(), "<conference>");
    }

    #[test]
    fn test_event_display_with_id() {
        let event = Event::new(EventType::Token("presence".to_string()))
            .with_id("123");
        assert_eq!(event.to_string(), "presence;id=123"); // 'id' prints lowercase
    }
    
    #[test]
    fn test_event_display_with_uppercase_id_in_struct() {
        // Ensure canonical lowercase 'id' printing even if field somehow got uppercase
        let event = Event {
            event_type: EventType::Token("presence".to_string()),
            id: Some("UPPER123".to_string()), // Value itself is case-sensitive
            params: Params::new(),
        };
        assert_eq!(event.to_string(), "presence;id=UPPER123");
    }

    #[test]
    fn test_event_display_with_package_and_id() {
        let event = Event::new(EventType::Package("dialog".to_string()))
            .with_id("xyz");
        assert_eq!(event.to_string(), "<dialog>;id=xyz");
    }

    #[test]
    fn test_event_display_with_generic_param() {
        let event = Event::new(EventType::Token("presence".to_string()))
            .with_param("foo", Some("bar"));
        assert_eq!(event.to_string(), "presence;foo=bar");
    }
    
    #[test]
    fn test_event_display_with_valueless_generic_param() {
        let event = Event::new(EventType::Token("presence".to_string()))
            .with_param("foo", None::<String>);
        assert_eq!(event.to_string(), "presence;foo");
    }

    #[test]
    fn test_event_display_with_id_and_generic_param() {
        let event = Event::new(EventType::Token("message-summary".to_string()))
            .with_id("msg-waiting")
            .with_param("custom", Some("value"));
        assert_eq!(event.to_string(), "message-summary;id=msg-waiting;custom=value");
    }
    
    #[test]
    fn test_event_display_with_id_and_multiple_generic_params_order() {
        // Params order from BTreeMap is by key: baz, foo
        let event = Event::new(EventType::Token("presence".to_string()))
            .with_id("123")
            .with_param("foo", Some("bar")) // Added out of alpha order
            .with_param("baz", None::<String>);        // Added out of alpha order
        assert_eq!(event.to_string(), "presence;id=123;baz;foo=bar");
    }

    #[test]
    fn test_typed_header_trait_from_header() {
        let header = Header::new(
            HeaderName::Event, 
            HeaderValue::text("refer;id=987zyx;arbitrary=data")
        );
        let event = Event::from_header(&header).unwrap();
        assert_eq!(event.event_type, EventType::Token("refer".to_string()));
        assert_eq!(event.id, Some("987zyx".to_string()));
        assert_eq!(event.params.get("arbitrary"), Some(&ParamValue::Value("data".to_string())));
    }

    #[test]
    fn test_typed_header_trait_to_header_round_trip() {
        let input_str = r#"refer;id=987zyx;action=notify;resource="sip:user@example.com""#;
        let header_in = Header::new(HeaderName::Event, HeaderValue::text(input_str));
        let event = Event::from_header(&header_in).unwrap();
        let header_out = event.to_header();
        assert_eq!(header_out.name, HeaderName::Event);
        match header_out.value {
            HeaderValue::Raw(bytes) => assert_eq!(std::str::from_utf8(&bytes).unwrap(), input_str),
            _ => panic!("Expected HeaderValue::Raw from to_header for Event")
        }
    }
    
    #[test]
    fn test_typed_header_trait_mismatched_name() {
        let header = Header::new(HeaderName::From, HeaderValue::text("presence"));
        assert!(Event::from_header(&header).is_err());
    }

    #[test]
    fn test_typed_header_trait_invalid_value_utf8() {
        let invalid_utf8_bytes = vec![0xf0, 0x90, 0x80];
        let header = Header::new(HeaderName::Event, HeaderValue::Raw(invalid_utf8_bytes));
        assert!(Event::from_header(&header).is_err());
    }

    #[test]
    fn test_typed_header_trait_parser_error_incomplete() {
        // Test parsing an incomplete Event header value via TypedHeaderTrait
        let header = Header::new(HeaderName::Event, HeaderValue::text("<incomplete".to_string()));
        let result = Event::from_header(&header); // This calls Event::from_str
        assert!(result.is_err());
        if let Err(Error::Parser(msg)) = result {
            // Expected: Failed to parse Event value string '<incomplete': Parser error at '<incomplete' (code: Tag)
            // Or: Failed to parse Event value string '<incomplete': Parser error at 'incomplete' (code: TakeWhile1)
            // Or: Failed to parse Event value string '<incomplete': Parser error at '' (code: Eof) if <incomplete is valid then eof fails
            // The exact point of failure and code might vary based on nom's internal behavior for this specific malformed input.
            // We'll check for the key parts.
            assert!(
                msg.starts_with("Failed to parse Event value string '<incomplete': Parser error at '") && (msg.contains("(code: Tag)") || msg.contains("(code: TakeWhile1)") || msg.contains("(code: Eof)") || msg.contains("(code: Alt)")),
                "Unexpected error message for incomplete input: {}",
                msg
            );
        } else {
            panic!("Expected a Parser error for incomplete input, got {:?}", result);
        }
    }

    #[test]
    fn test_typed_header_trait_parser_error_trailing_rubbish() {
        // Test parsing an Event header value with trailing rubbish via TypedHeaderTrait
        let header_value_str = "presence;id=123 then rubbish";
        let header = Header::new(HeaderName::Event, HeaderValue::text(header_value_str.to_string()));
        let result = Event::from_header(&header); // This calls Event::from_str
        assert!(result.is_err());
        if let Err(Error::Parser(msg)) = result {
            let expected_msg_part1 = "Failed to parse Event value string 'presence;id=123 then rubbish': Parser error at '";
            let expected_rubbish_part = "then rubbish"; // This is what e.input should be for the Eof error
            let expected_msg_part2 = format!("{}' (code: Eof)", expected_rubbish_part);
            
            assert!(
                msg.starts_with(expected_msg_part1) && msg.contains(&expected_rubbish_part) && msg.ends_with("(code: Eof)"),
                "Unexpected error message for trailing rubbish. Got: {}\\nExpected to start with: {}\\nAnd contain: {}\\nAnd end with: (code: Eof)",
                msg, expected_msg_part1, expected_rubbish_part
            );
        } else {
            panic!("Expected a Parser error for trailing rubbish, got {:?}", result);
        }
    }

    #[test]
    fn test_parse_errors_for_direct_parsers() {
        assert!(parse_event_header(b"Event: <pkg").is_err()); 
        assert!(parse_event_header(b"Event: @@@").is_err()); 
        assert!(parse_event_header_value(b"<pkg;id=1").is_err()); 
        // This assertion was the one failing. It should be true that it IS an error.
        assert!(parse_event_header_value(b"valid;param=true thenrubbish").is_err());
    
        let parsed_pkg_id = Event::from_header(&Header::new(HeaderName::Event, HeaderValue::text("pkg;id"))).unwrap();
        assert_eq!(parsed_pkg_id.event_type, EventType::Token("pkg".to_string()));
        assert!(parsed_pkg_id.id.is_none());
        assert_eq!(parsed_pkg_id.params.get("id"), Some(&ParamValue::None));
    }
} 