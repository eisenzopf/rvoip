// Time Description SDP Parser
//
// Handles parsing of time-related components in SDP messages per RFC 8866.
// This includes t= (timing) and r= (repeat) lines.

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

/// Use nom to parse a numeric time value
fn parse_numeric_time(input: &str) -> IResult<&str, u64> {
    map_res(
        digit1,
        |s: &str| s.parse::<u64>()
    )(input)
}

/// Validate that a time field is a valid NTP timestamp or 0
pub fn validate_time_field(time_field: &str, field_name: &str) -> Result<u64> {
    match time_field.parse::<u64>() {
        Ok(time) => Ok(time),
        Err(_) => Err(Error::SdpParsingError(format!(
            "Invalid {} time value: {}", field_name, time_field
        ))),
    }
}

/// Use nom to parse a time value with unit
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
/// Returns the value in seconds
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

/// Parse time field which is either a numeric timestamp or 0
fn parse_time_field(input: &str) -> IResult<&str, u64> {
    map_res(
        digit1,
        |s: &str| s.parse::<u64>()
    )(input)
}

/// Parse a time description using nom
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

/// Use nom to parse a repeat time line
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
/// Format: r=<repeat-interval> <active-duration> <list-of-offsets-from-start-time>
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