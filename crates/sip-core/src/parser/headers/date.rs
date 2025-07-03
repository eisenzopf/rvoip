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
    error::{Error as NomError, ErrorKind, ParseError},
};
use std::str;

// Import from new modules
use crate::parser::separators::hcolon;
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
pub fn sip_date(input: &[u8]) -> ParseResult<DateTime<FixedOffset>> {
    // First, parse the structured parts to ensure RFC compliance
    let (remaining, date_parts) = tuple((
        wkday,
        tag(","),
        space1,
        two_digit, // day
        space1,
        month,
        space1,
        four_digit, // year
        space1,
        two_digit, // hour
        tag(":"),
        two_digit, // minute
        tag(":"),
        two_digit, // second
        space1,
        tag("GMT") // timezone
    ))(input)?;

    // Convert components to strings
    let day_of_week = match std::str::from_utf8(date_parts.0) {
        Ok(s) => s,
        Err(_) => return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))
    };
    
    let day = match std::str::from_utf8(date_parts.3) {
        Ok(s) => s.parse::<u32>().unwrap_or(0),
        Err(_) => return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))
    };
    
    let month_str = match std::str::from_utf8(date_parts.5) {
        Ok(s) => s.to_uppercase(),
        Err(_) => return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))
    };
    
    // Convert month string to month number
    let month = match month_str.as_str() {
        "JAN" => 1,
        "FEB" => 2,
        "MAR" => 3,
        "APR" => 4,
        "MAY" => 5,
        "JUN" => 6,
        "JUL" => 7,
        "AUG" => 8,
        "SEP" => 9,
        "OCT" => 10,
        "NOV" => 11,
        "DEC" => 12,
        _ => return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Verify)))
    };
    
    let year = match std::str::from_utf8(date_parts.7) {
        Ok(s) => s.parse::<i32>().unwrap_or(0),
        Err(_) => return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))
    };
    
    let hour = match std::str::from_utf8(date_parts.9) {
        Ok(s) => s.parse::<u32>().unwrap_or(0),
        Err(_) => return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))
    };
    
    let minute = match std::str::from_utf8(date_parts.11) {
        Ok(s) => s.parse::<u32>().unwrap_or(0),
        Err(_) => return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))
    };
    
    let second = match std::str::from_utf8(date_parts.13) {
        Ok(s) => s.parse::<u32>().unwrap_or(0),
        Err(_) => return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Char)))
    };
    
    // Validate date values
    if day == 0 || day > 31 || month == 0 || month > 12 || year < 0 ||
       hour > 23 || minute > 59 || second > 59 {
        return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Verify)));
    }
    
    // Additional validation for month lengths
    let is_leap_year = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let days_in_month = match month {
        2 => if is_leap_year { 29 } else { 28 },
        4 | 6 | 9 | 11 => 30,
        _ => 31
    };
    
    if day > days_in_month {
        return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Verify)));
    }
    
    // Construct a datetime from the parsed components
    let naive_date = match NaiveDate::from_ymd_opt(year, month, day) {
        Some(d) => d,
        None => return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Verify)))
    };
    
    let naive_time = match NaiveTime::from_hms_opt(hour, minute, second) {
        Some(t) => t,
        None => return Err(nom::Err::Failure(NomError::from_error_kind(input, ErrorKind::Verify)))
    };
    
    let naive_datetime = NaiveDateTime::new(naive_date, naive_time);
    let datetime = DateTime::<FixedOffset>::from_utc(naive_datetime, FixedOffset::east_opt(0).unwrap());
    
    Ok((remaining, datetime))
}

// Date = "Date" HCOLON SIP-date
// Note: HCOLON handled by message_header
pub fn parse_date(input: &[u8]) -> ParseResult<DateTime<FixedOffset>> {
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
    
    #[test]
    fn test_all_weekdays() {
        // Test all valid weekdays
        assert!(sip_date(b"Mon, 01 Jan 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Tue, 01 Jan 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Wed, 01 Jan 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Thu, 01 Jan 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Fri, 01 Jan 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Sat, 01 Jan 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Sun, 01 Jan 2023 00:00:00 GMT").is_ok());
        
        // Test case insensitivity
        assert!(sip_date(b"mon, 01 Jan 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"MON, 01 Jan 2023 00:00:00 GMT").is_ok());
    }
    
    #[test]
    fn test_all_months() {
        // Test all valid months
        assert!(sip_date(b"Mon, 01 Jan 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 Feb 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 Mar 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 Apr 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 May 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 Jun 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 Jul 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 Aug 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 Sep 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 Oct 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 Nov 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 Dec 2023 00:00:00 GMT").is_ok());
        
        // Test case insensitivity
        assert!(sip_date(b"Mon, 01 jan 2023 00:00:00 GMT").is_ok());
        assert!(sip_date(b"Mon, 01 JAN 2023 00:00:00 GMT").is_ok());
    }
    
    #[test]
    fn test_edge_cases() {
        // Test leap year date
        let leap_year = b"Mon, 29 Feb 2024 00:00:00 GMT";
        let (_, dt) = sip_date(leap_year).unwrap();
        assert_eq!(dt.day(), 29);
        assert_eq!(dt.month(), 2);
        assert_eq!(dt.year(), 2024);
        
        // Test invalid leap year date
        assert!(sip_date(b"Mon, 29 Feb 2023 00:00:00 GMT").is_err());
        
        // Test date boundaries
        assert!(sip_date(b"Mon, 00 Jan 2023 00:00:00 GMT").is_err()); // Invalid day 0
        assert!(sip_date(b"Mon, 32 Jan 2023 00:00:00 GMT").is_err()); // Invalid day 32 for Jan
        assert!(sip_date(b"Mon, 31 Apr 2023 00:00:00 GMT").is_err()); // Invalid day 31 for Apr
        
        // Test time boundaries
        assert!(sip_date(b"Mon, 01 Jan 2023 24:00:00 GMT").is_err()); // Invalid hour 24
        assert!(sip_date(b"Mon, 01 Jan 2023 00:60:00 GMT").is_err()); // Invalid minute 60
        assert!(sip_date(b"Mon, 01 Jan 2023 00:00:60 GMT").is_err()); // Invalid second 60
    }
    
    #[test]
    fn test_rfc3261_examples() {
        // Example from RFC 3261 Section 20.17
        let example = b"Sat, 13 Nov 2010 23:29:00 GMT";
        let result = sip_date(example);
        assert!(result.is_ok());
        
        // Example from RFC 3261 Section 25.1 (the SIP message with Date header)
        let invite_example = b"Thu, 21 Feb 2002 13:02:03 GMT";
        let result_invite = sip_date(invite_example);
        assert!(result_invite.is_ok());
    }
    
    #[test]
    fn test_malformed_formats() {
        // Missing comma after weekday
        assert!(sip_date(b"Wed 02 Oct 2002 08:00:00 GMT").is_err());
        
        // Missing space after comma
        assert!(sip_date(b"Wed,02 Oct 2002 08:00:00 GMT").is_err());
        
        // Missing space between day and month
        assert!(sip_date(b"Wed, 02Oct 2002 08:00:00 GMT").is_err());
        
        // Missing space between month and year
        assert!(sip_date(b"Wed, 02 Oct2002 08:00:00 GMT").is_err());
        
        // Missing space between date and time
        assert!(sip_date(b"Wed, 02 Oct 200208:00:00 GMT").is_err());
        
        // Missing space between time and timezone
        assert!(sip_date(b"Wed, 02 Oct 2002 08:00:00GMT").is_err());
        
        // Missing GMT timezone
        assert!(sip_date(b"Wed, 02 Oct 2002 08:00:00").is_err());
        
        // Wrong timezone format (lowercase)
        assert!(sip_date(b"Wed, 02 Oct 2002 08:00:00 gmt").is_err());
    }
} 