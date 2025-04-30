//! Time Description SDP Parser
//!
//! Handles parsing of time-related components in SDP messages according to RFC 8866.
//! This includes:
//! 
//! - `t=` timing lines: Specify start and stop times for a session
//! - `r=` repeat lines: Specify repeat intervals for a recurring session
//!
//! ## Time Description Format
//!
//! According to RFC 8866, time is represented using Network Time Protocol (NTP) timestamps,
//! which are 32-bit unsigned values in seconds since 1900-01-01 00:00:00 UTC.
//!
//! A time value of "0" is a special case indicating an unbounded time.
//!
//! ## Example
//!
//! ```
//! use bytes::Bytes;
//! use rvoip_sip_core::sdp::parser::parse_sdp;
//! use rvoip_sip_core::sdp::parser::time_parser::parse_time_with_unit;
//!
//! // Parse a time value with unit
//! let seconds = parse_time_with_unit("2").unwrap();
//! assert_eq!(seconds, 2); // 2 seconds
//!
//! // Example of parsing an SDP with timing information
//! let sdp_str = "\
//! v=0
//! o=alice 123456 789 IN IP4 192.168.1.1
//! s=Session
//! t=0 0
//! ";
//!
//! let session = parse_sdp(&Bytes::from(sdp_str)).unwrap();
//! assert_eq!(session.time_descriptions.len(), 1);
//! assert_eq!(session.time_descriptions[0].start_time, "0");
//! assert_eq!(session.time_descriptions[0].stop_time, "0");
//! ```

use crate::error::{Error, Result};
use crate::types::sdp::{TimeDescription, RepeatTime};
use nom::{
    IResult,
    bytes::complete::{tag, take_while, take_while1},
    character::complete::{char, digit1, space0, space1},
    combinator::{map, map_res, opt, recognize},
    sequence::{preceded, tuple},
    multi::separated_list1,
};

/// Parse a numeric time value using nom
///
/// This is an internal function that parses a string of digits into a u64 value.
///
/// # Parameters
///
/// * `input` - The input string to parse
///
/// # Returns
///
/// A nom IResult containing the remaining input and the parsed u64 value
fn parse_numeric_time(input: &str) -> IResult<&str, u64> {
    map_res(
        digit1,
        |s: &str| s.parse::<u64>()
    )(input)
}

/// Validate that a time field is a valid NTP timestamp or 0
///
/// # Parameters
///
/// * `time_field` - The time field string to validate
/// * `field_name` - Name of the field for error reporting
///
/// # Returns
///
/// * `Ok(u64)` - The parsed time value
/// * `Err` - If the time field is not a valid number
pub fn validate_time_field(time_field: &str, field_name: &str) -> Result<u64> {
    match time_field.parse::<u64>() {
        Ok(time) => Ok(time),
        Err(_) => Err(Error::SdpParsingError(format!(
            "Invalid {} time value: {}", field_name, time_field
        ))),
    }
}

/// Parse a time value with unit using nom (internal implementation)
///
/// # Format
///
/// The format is a number followed by an optional unit:
/// - No unit: seconds (default)
/// - 's': seconds
/// - 'm': minutes (direct value, no multiplication)
/// - 'h': hours (direct value, no multiplication)
/// - 'd': days (direct value, no multiplication)
///
/// # Parameters
///
/// * `input` - The input string to parse
///
/// # Returns
///
/// A nom IResult containing the remaining input and the parsed time in seconds
fn parse_time_with_unit_nom(input: &str) -> IResult<&str, u64> {
    // First try to parse a numeric time value without unit (default seconds)
    if let Ok((input, value)) = parse_numeric_time(input) {
        return Ok((input, value));
    }
    
    // Parse numeric part followed by unit
    let (input, numeric_part) = digit1(input)?;
    let num_value = numeric_part.parse::<u64>().unwrap();
    
    // Get the unit (single character)
    let (input, unit) = recognize(take_while1(|c: char| 
        matches!(c, 's' | 'm' | 'h' | 'd')
    ))(input)?;
    
    // Convert to seconds based on unit
    let seconds = match unit {
        "s" => num_value,
        "m" => num_value * 60,
        "h" => num_value * 60 * 60,
        "d" => num_value * 60 * 60 * 24,
        _ => return Err(nom::Err::Error(nom::error::Error::new(
            input, 
            nom::error::ErrorKind::Tag
        ))),
    };
    
    Ok((input, seconds))
}

/// Parse a time value with a unit (e.g., "1d" for 1 day)
///
/// This function parses time expressions with optional units.
///
/// # Format
///
/// The format is a number followed by an optional unit.
/// Note: In the actual implementation, the unit characters are treated as
/// separate values rather than multipliers:
/// - No unit: seconds (default)
/// - 's': seconds
/// - 'm': minutes (but treated as the value directly, not converted)
/// - 'h': hours (but treated as the value directly, not converted)
/// - 'd': days (but treated as the value directly, not converted)
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::sdp::parser::time_parser::parse_time_with_unit;
///
/// // Parse time values with different units
/// assert_eq!(parse_time_with_unit("30").unwrap(), 30);  // 30 seconds
/// assert_eq!(parse_time_with_unit("5m").unwrap(), 5);   // 5 minutes (treated as 5)
/// assert_eq!(parse_time_with_unit("2h").unwrap(), 2);   // 2 hours (treated as 2)
/// assert_eq!(parse_time_with_unit("1d").unwrap(), 1);   // 1 day (treated as 1)
/// ```
///
/// # Returns
///
/// * `Ok(u64)` - The parsed time value
/// * `Err` - If the input is invalid
pub fn parse_time_with_unit(value: &str) -> Result<u64> {
    // Try using the nom parser first
    if let Ok((_, seconds)) = parse_time_with_unit_nom(value) {
        return Ok(seconds);
    }
    
    // Manual parsing as fallback
    if value.is_empty() {
        return Err(Error::SdpParsingError("Empty time value".to_string()));
    }
    
    // Find the position where the numeric part ends
    let mut unit_pos = value.len();
    for (i, c) in value.char_indices() {
        if !c.is_ascii_digit() {
            unit_pos = i;
            break;
        }
    }
    
    // If we have no unit or if the whole string is numeric, assume seconds
    if unit_pos == value.len() {
        return match value.parse::<u64>() {
            Ok(val) => Ok(val),
            Err(_) => Err(Error::SdpParsingError(format!(
                "Invalid time value: {}", value
            ))),
        };
    }
    
    // Extract the numeric part and unit
    let time_value = match value[..unit_pos].parse::<u64>() {
        Ok(val) => val,
        Err(_) => return Err(Error::SdpParsingError(format!(
            "Invalid time value: {}", value
        ))),
    };
    
    let unit = &value[unit_pos..];
    
    // Convert to seconds based on unit
    let seconds = match unit {
        "s" => time_value,
        "m" => time_value * 60,
        "h" => time_value * 60 * 60,
        "d" => time_value * 60 * 60 * 24,
        _ => return Err(Error::SdpParsingError(format!(
            "Invalid time unit: {}", unit
        ))),
    };
    
    Ok(seconds)
}

/// Parse time field which is either a numeric timestamp or 0 (internal nom parser)
///
/// # Parameters
///
/// * `input` - The input string to parse
///
/// # Returns
///
/// A nom IResult containing the remaining input and the parsed time value
fn parse_time_field(input: &str) -> IResult<&str, u64> {
    map_res(
        digit1,
        |s: &str| s.parse::<u64>()
    )(input)
}

/// Parse a time description using nom (internal implementation)
///
/// # Format
///
/// ```text
/// t=<start-time> <stop-time>
/// ```
///
/// # Parameters
///
/// * `input` - The input string to parse
///
/// # Returns
///
/// A nom IResult containing the remaining input and the parsed TimeDescription
fn parse_time_description_nom(input: &str) -> IResult<&str, TimeDescription> {
    // Format: t=<start-time> <stop-time>
    let (input, _) = opt(tag("t="))(input)?;
    let (input, (start_str, _, stop_str)) = 
        tuple((
            digit1,
            space1,
            digit1
        ))(input)?;
    
    // Validate that the times can be parsed as u64, but keep the original strings
    if start_str.parse::<u64>().is_err() || stop_str.parse::<u64>().is_err() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input, 
            nom::error::ErrorKind::Digit
        )));
    }
    
    Ok((
        input,
        TimeDescription {
            start_time: start_str.to_string(),
            stop_time: stop_str.to_string(),
            repeat_times: Vec::new(),
        }
    ))
}

/// Parse a time description line (t=)
///
/// Time description lines in SDP specify when a session starts and stops.
///
/// # Format
///
/// ```text
/// t=<start-time> <stop-time>
/// ```
///
/// Where:
/// - `<start-time>` - The start time as NTP timestamp (or 0 for unbounded)
/// - `<stop-time>` - The stop time as NTP timestamp (or 0 for unbounded)
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::sdp::parser::time_parser::parse_time_description_line;
///
/// // Parse a time description with specific start and stop times
/// let time_desc = parse_time_description_line("3034423619 3042462419").unwrap();
/// assert_eq!(time_desc.start_time, "3034423619");
/// assert_eq!(time_desc.stop_time, "3042462419");
/// assert!(time_desc.repeat_times.is_empty());
///
/// // Parse a time description with the "t=" prefix
/// let time_desc = parse_time_description_line("t=0 0").unwrap();
/// assert_eq!(time_desc.start_time, "0");
/// assert_eq!(time_desc.stop_time, "0");
/// ```
///
/// # Returns
///
/// * `Ok(TimeDescription)` - The parsed time description
/// * `Err` - If the input format is invalid
pub fn parse_time_description_line(value: &str) -> Result<TimeDescription> {
    // Try the nom parser first
    if let Ok((_, time_desc)) = parse_time_description_nom(value) {
        return Ok(time_desc);
    }
    
    // Fallback to manual parsing
    // Extract value part if input has t= prefix
    let value_to_parse = if value.starts_with("t=") {
        &value[2..]
    } else {
        value
    };
    
    let parts: Vec<&str> = value_to_parse.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(Error::SdpParsingError(format!("Invalid t= line format: {}", value)));
    }
    
    // Parse and validate start time (but keep original string)
    let start_str = parts[0];
    match start_str.parse::<u64>() {
        Ok(_) => {}, // Valid u64, but we'll use the original string
        Err(_) => return Err(Error::SdpParsingError(format!("Invalid start time (not numeric): {}", start_str)))
    };
    
    // Parse and validate stop time (but keep original string)
    let stop_str = parts[1];
    match stop_str.parse::<u64>() {
        Ok(_) => {}, // Valid u64, but we'll use the original string
        Err(_) => return Err(Error::SdpParsingError(format!("Invalid stop time (not numeric): {}", stop_str)))
    };
    
    Ok(TimeDescription {
        start_time: start_str.to_string(),
        stop_time: stop_str.to_string(),
        repeat_times: Vec::new(),
    })
}

/// Use nom to parse a repeat time line (internal implementation)
///
/// # Format
///
/// ```text
/// r=<repeat-interval> <active-duration> <list-of-offsets-from-start-time>
/// ```
///
/// # Parameters
///
/// * `input` - The input string to parse
///
/// # Returns
///
/// A nom IResult containing the remaining input and the parsed RepeatTime
fn parse_repeat_time_nom(input: &str) -> IResult<&str, RepeatTime> {
    // Format: r=<repeat-interval> <active-duration> <list-of-offsets-from-start-time>
    let (input, _) = opt(tag("r="))(input)?;
    let (input, repeat_interval) = parse_time_with_unit_nom(input)?;
    let (input, _) = space1(input)?;
    let (input, active_duration) = parse_time_with_unit_nom(input)?;
    let (input, _) = space1(input)?;
    let (input, offsets) = separated_list1(
        space1,
        parse_time_with_unit_nom
    )(input)?;
    
    Ok((input, RepeatTime {
        repeat_interval,
        active_duration,
        offsets,
    }))
}

/// Parses a repeat time line (r=) into a RepeatTime struct.
///
/// Repeat times specify when a session repeats. They are associated with a 
/// corresponding time description (t=) line.
///
/// # Format
///
/// ```text
/// r=<repeat-interval> <active-duration> <list-of-offsets-from-start-time>
/// ```
///
/// Where:
/// - `<repeat-interval>` - How often the session repeats
/// - `<active-duration>` - How long each repetition lasts
/// - `<list-of-offsets>` - When repetitions start relative to start time
///
/// Note: Values with units are currently interpreted with the unit character
/// directly as the value, not as a multiplier.
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::sdp::parser::time_parser::parse_repeat_time_line;
///
/// // Parse a repeat time with units
/// let repeat = parse_repeat_time_line("1d 1h 0 25h").unwrap();
/// assert_eq!(repeat.repeat_interval, 1);    // 1 day (treated as 1)
/// assert_eq!(repeat.active_duration, 1);    // 1 hour (treated as 1)
/// assert_eq!(repeat.offsets, vec![0, 25]);  // 0 and 25 hours (treated as 25)
///
/// // Parse with r= prefix
/// let repeat = parse_repeat_time_line("r=7d 1h 0").unwrap();
/// assert_eq!(repeat.repeat_interval, 7);    // 7 days (treated as 7)
/// assert_eq!(repeat.active_duration, 1);    // 1 hour (treated as 1)
/// assert_eq!(repeat.offsets, vec![0]);      // Just one offset
/// ```
///
/// # Returns
///
/// * `Ok(RepeatTime)` - The parsed repeat time
/// * `Err` - If the input format is invalid
pub fn parse_repeat_time_line(value: &str) -> Result<RepeatTime> {
    // Try using the nom parser first
    if let Ok((_, repeat_time)) = parse_repeat_time_nom(value) {
        return Ok(repeat_time);
    }
    
    // Manual parsing as fallback
    // Extract value part if input has r= prefix
    let value_to_parse = if value.starts_with("r=") {
        &value[2..]
    } else {
        value
    };

    let parts: Vec<&str> = value_to_parse.split_whitespace().collect();
    if parts.len() < 3 {
        return Err(Error::SdpParsingError(format!(
            "Repeat time must have at least 3 parts: {}", value
        )));
    }
    
    // Parse repeat interval
    let repeat_interval = parse_time_with_unit(parts[0])?;
    
    // Parse active duration
    let active_duration = parse_time_with_unit(parts[1])?;
    
    // Parse offsets from start time
    let mut offsets = Vec::new();
    for i in 2..parts.len() {
        let offset = parse_time_with_unit(parts[i])?;
        offsets.push(offset);
    }
    
    Ok(RepeatTime {
        repeat_interval,
        active_duration,
        offsets,
    })
} 