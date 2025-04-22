// Parser for Date header (RFC 3261 Section 20.17)
// Date = "Date" HCOLON SIP-date
// SIP-date = rfc1123-date (defined in RFC 2616)

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_while_m_n},
    character::complete::{digit1, space1, char},
    combinator::{map_res, recognize},
    sequence::{tuple, preceded, delimited},
    IResult,
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
use crate::parser::common_chars::take_till_crlf; // Helper to take rest of line
use crate::parser::ParseResult;

// Assuming chrono is available as a dependency
// If not, we'd need manual parsing logic.
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime, NaiveDateTime, Datelike, Timelike, Weekday};

// wkday = "Mon" / "Tue" / "Wed" / "Thu" / "Fri" / "Sat" / "Sun"
fn wkday(input: &[u8]) -> ParseResult<&[u8]> {
    alt((
        tag_no_case("Mon"), tag_no_case("Tue"), tag_no_case("Wed"),
        tag_no_case("Thu"), tag_no_case("Fri"), tag_no_case("Sat"),
        tag_no_case("Sun")
    ))(input)
}

// month = "Jan" / "Feb" / "Mar" / "Apr" / "May" / "Jun" / 
//         "Jul" / "Aug" / "Sep" / "Oct" / "Nov" / "Dec"
fn month(input: &[u8]) -> ParseResult<&[u8]> {
    alt((
        tag_no_case("Jan"), tag_no_case("Feb"), tag_no_case("Mar"),
        tag_no_case("Apr"), tag_no_case("May"), tag_no_case("Jun"),
        tag_no_case("Jul"), tag_no_case("Aug"), tag_no_case("Sep"),
        tag_no_case("Oct"), tag_no_case("Nov"), tag_no_case("Dec")
    ))(input)
}

// 2DIGIT helper
fn two_digit(input: &[u8]) -> ParseResult<&[u8]> {
    take_while_m_n(2, 2, |c: u8| c.is_ascii_digit())(input)
}

// 4DIGIT helper
fn four_digit(input: &[u8]) -> ParseResult<&[u8]> {
    take_while_m_n(4, 4, |c: u8| c.is_ascii_digit())(input)
}

// date1 = 2DIGIT SP month SP 4DIGIT
fn date1(input: &[u8]) -> ParseResult<(&[u8], &[u8], &[u8])> {
    tuple((two_digit, preceded(space1, month), preceded(space1, four_digit)))(input)
}

// time = 2DIGIT ":" 2DIGIT ":" 2DIGIT
fn time(input: &[u8]) -> ParseResult<(&[u8], &[u8], &[u8])> {
    tuple((
        two_digit, 
        preceded(char(':'), two_digit), 
        preceded(char(':'), two_digit)
    ))(input)
}

// rfc1123-date = wkday "," SP date1 SP time SP "GMT"
// Returns DateTime<FixedOffset> assuming chrono is available
pub(crate) fn sip_date(input: &[u8]) -> ParseResult<DateTime<FixedOffset>> {
    map_res(
        recognize( // Recognize the full pattern first
            tuple((
                wkday,
                tag(","),
                space1,
                date1,
                space1,
                time,
                space1,
                tag("GMT") // Assumes GMT/UTC timezone
            ))
        ),
        |bytes| {
            // Use chrono to parse the recognized RFC 1123 string
            let date_str = str::from_utf8(bytes)?;
            DateTime::parse_from_rfc2822(date_str) // RFC 2822 is compatible with RFC 1123 format
                .map_err(|e| format!("Chrono parsing failed: {}", e))
        }
    )(input)
}

// Date = "Date" HCOLON SIP-date
// Note: HCOLON handled by message_header
pub(crate) fn parse_date(input: &[u8]) -> ParseResult<DateTime<FixedOffset>> {
    sip_date(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_sip_date() {
        let input = b"Wed, 02 Oct 2002 08:00:00 GMT";
        let result = sip_date(input);
        assert!(result.is_ok());
        let (rem, dt) = result.unwrap();
        assert!(rem.is_empty());
        assert_eq!(dt.year(), 2002);
        assert_eq!(dt.month(), 10);
        assert_eq!(dt.day(), 2);
        assert_eq!(dt.hour(), 8);
        assert_eq!(dt.minute(), 0);
        assert_eq!(dt.second(), 0);
        assert_eq!(dt.weekday(), Weekday::Wed);
        assert_eq!(dt.offset().local_minus_utc(), 0); // Check timezone is GMT/UTC

        let input_fri = b"Fri, 04 Aug 2006 15:30:00 GMT";
        let result_fri = sip_date(input_fri);
        assert!(result_fri.is_ok());
        let (rem_fri, dt_fri) = result_fri.unwrap();
        assert!(rem_fri.is_empty());
        assert_eq!(dt_fri.weekday(), Weekday::Fri);
        assert_eq!(dt_fri.hour(), 15);
    }
    
    #[test]
    fn test_invalid_sip_date() {
        assert!(sip_date(b"Wed, 02 Oct 2002 08:00:00 EST").is_err()); // Invalid TZ
        assert!(sip_date(b"Wednesday, 02 Oct 2002 08:00:00 GMT").is_err()); // Full weekday
        assert!(sip_date(b"Wed, 2 Oct 2002 08:00:00 GMT").is_err()); // Single digit day
        assert!(sip_date(b"Wed, 02 October 2002 08:00:00 GMT").is_err()); // Full month
        assert!(sip_date(b"Wed, 02 Oct 02 08:00:00 GMT").is_err()); // 2 digit year
        assert!(sip_date(b"Wed, 02 Oct 2002 8:00:00 GMT").is_err()); // Single digit hour
    }
} 