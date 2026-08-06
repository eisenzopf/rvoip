// Replaces Header Field
//
// Defined in RFC 3891, "The Session Initiation Protocol (SIP) 'Replaces'
// Header". It names one existing dialog that the incoming INVITE is to shut
// down and logically replace.
//
// ABNF (RFC 3891 Section 6.1):
// Replaces        = "Replaces" HCOLON callid *(SEMI replaces-param)
// replaces-param  = to-tag / from-tag / early-flag / generic-param
// to-tag          = "to-tag" EQUAL token
// from-tag        = "from-tag" EQUAL token
// early-flag      = "early-only"
//
// The same section requires exactly one to-tag and exactly one from-tag,
// "as they are required for unique dialog matching". Both are therefore
// mandatory here and their absence, or a duplicate, is a parse failure. That
// keeps the malformed case away from dialog matching entirely, which is what
// lets the UAS answer 400 Bad Request without inspecting the value.

use crate::parser::common_params::semicolon_params0;
use crate::parser::headers::call_id::callid;
use crate::parser::separators::hcolon;
use crate::parser::ParseResult;
use crate::types::param::Param;
use crate::types::replaces::Replaces;

use nom::{
    bytes::complete::tag_no_case,
    character::complete::multispace0,
    combinator::{eof, map_res},
    sequence::{pair, preceded, terminated, tuple},
};

/// Pull `to-tag`, `from-tag` and the `early-only` flag out of the generic
/// parameter list, leaving everything else as an opaque parameter.
///
/// A repeated `to-tag` or `from-tag` is rejected rather than last-wins: RFC
/// 3891 Section 6.1 says "exactly one", and a request carrying two of them has
/// no single dialog to match.
fn collect_replaces_params(params: Vec<Param>) -> Result<(String, String, bool, Vec<Param>), ()> {
    let mut to_tag: Option<String> = None;
    let mut from_tag: Option<String> = None;
    let mut early_only = false;
    let mut others = Vec::new();

    for param in params {
        match &param {
            Param::Other(name, value) if name.eq_ignore_ascii_case("to-tag") => {
                let value = value.as_ref().ok_or(())?.to_string();
                if to_tag.replace(value).is_some() {
                    return Err(());
                }
            }
            Param::Other(name, value) if name.eq_ignore_ascii_case("from-tag") => {
                let value = value.as_ref().ok_or(())?.to_string();
                if from_tag.replace(value).is_some() {
                    return Err(());
                }
            }
            Param::Other(name, None) if name.eq_ignore_ascii_case("early-only") => {
                early_only = true;
            }
            _ => others.push(param),
        }
    }

    match (to_tag, from_tag) {
        (Some(to_tag), Some(from_tag)) if !to_tag.is_empty() && !from_tag.is_empty() => {
            Ok((to_tag, from_tag, early_only, others))
        }
        _ => Err(()),
    }
}

/// Parse the value part of a Replaces header, everything after `Replaces:`.
///
/// Example: `12adf2f34456gs5;to-tag=12345;from-tag=54321;early-only`
pub fn parse_replaces_value(input: &[u8]) -> ParseResult<'_, Replaces> {
    terminated(
        map_res(
            pair(callid, semicolon_params0),
            |(call_id, params)| -> Result<Replaces, ()> {
                let (to_tag, from_tag, early_only, params) = collect_replaces_params(params)?;
                Ok(Replaces {
                    call_id,
                    to_tag,
                    from_tag,
                    early_only,
                    params,
                })
            },
        ),
        tuple((multispace0, eof)),
    )(input)
}

/// Parse a full Replaces header line, including the `Replaces:` name.
pub fn parse_replaces_header(input: &[u8]) -> ParseResult<'_, Replaces> {
    preceded(pair(tag_no_case(b"Replaces"), hcolon), parse_replaces_value)(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_rfc_3891_examples() {
        // RFC 3891 Section 6.1, second example.
        let (rem, replaces) =
            parse_replaces_value(b"12adf2f34456gs5;to-tag=12345;from-tag=54321;early-only")
                .expect("parse example with early-only");
        assert!(rem.is_empty());
        assert_eq!(replaces.call_id, "12adf2f34456gs5");
        assert_eq!(replaces.to_tag, "12345");
        assert_eq!(replaces.from_tag, "54321");
        assert!(replaces.early_only);

        // Third example: a tag of zero is legal, for RFC 2543 compatibility.
        let (rem, replaces) = parse_replaces_value(b"87134@171.161.34.23;to-tag=24796;from-tag=0")
            .expect("parse example with zero tag");
        assert!(rem.is_empty());
        assert_eq!(replaces.call_id, "87134@171.161.34.23");
        assert_eq!(replaces.from_tag, "0");
        assert!(!replaces.early_only);
    }

    /// The reason this parser exists. The previous string handling split the
    /// header on ':' and took the second field, which truncates any Call-ID
    /// carrying a port.
    #[test]
    fn call_id_containing_a_colon_is_not_truncated() {
        let (rem, replaces) =
            parse_replaces_value(b"call-abc@192.168.0.1:5060;to-tag=t1;from-tag=f1")
                .expect("parse Call-ID with a port");
        assert!(rem.is_empty());
        assert_eq!(replaces.call_id, "call-abc@192.168.0.1:5060");
    }

    #[test]
    fn parameter_order_does_not_matter() {
        let (_, from_first) = parse_replaces_value(b"cid;from-tag=f1;to-tag=t1").unwrap();
        let (_, to_first) = parse_replaces_value(b"cid;to-tag=t1;from-tag=f1").unwrap();
        assert_eq!(from_first, to_first);
    }

    #[test]
    fn both_tags_are_mandatory() {
        assert!(parse_replaces_value(b"cid;to-tag=t1").is_err());
        assert!(parse_replaces_value(b"cid;from-tag=f1").is_err());
        assert!(parse_replaces_value(b"cid").is_err());
    }

    /// "exactly one to-tag and exactly one from-tag" — a duplicate is not
    /// last-wins, it is malformed.
    #[test]
    fn duplicate_tags_are_rejected() {
        assert!(parse_replaces_value(b"cid;to-tag=t1;to-tag=t2;from-tag=f1").is_err());
        assert!(parse_replaces_value(b"cid;to-tag=t1;from-tag=f1;from-tag=f2").is_err());
    }

    #[test]
    fn generic_parameters_are_preserved() {
        let (_, replaces) =
            parse_replaces_value(b"cid;to-tag=t1;from-tag=f1;custom=value").unwrap();
        assert_eq!(replaces.params.len(), 1);
        assert!(!replaces.early_only);
    }

    #[test]
    fn parses_a_full_header_line() {
        let (rem, replaces) =
            parse_replaces_header(b"Replaces: cid@example.test;to-tag=t1;from-tag=f1")
                .expect("parse full header line");
        assert!(rem.is_empty());
        assert_eq!(replaces.call_id, "cid@example.test");
    }
}
