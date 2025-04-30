//! SDP attribute parsing functionality
//!
//! This module handles parsing of SDP attribute lines (a=).

use crate::error::{Error, Result};
use crate::types::sdp::ParsedAttribute;
use crate::sdp::attributes::MediaDirection;

// Import specialized parse functions
use crate::sdp::attributes::rtpmap::parse_rtpmap;
use crate::sdp::attributes::fmtp::parse_fmtp;
use crate::sdp::attributes::ptime;
use crate::sdp::attributes::candidate::parse_candidate;
use crate::sdp::attributes::ssrc::parse_ssrc;
use crate::sdp::attributes::mid;
use crate::sdp::attributes::msid;
use crate::sdp::attributes::group;
use crate::sdp::attributes::rtcp;
use crate::sdp::attributes::extmap;
use crate::sdp::attributes::rid;
use crate::sdp::attributes::simulcast;
use crate::sdp::attributes::sctpmap::parse_sctpmap;

/// Parse an attribute line (a=)
///
/// # Format
///
/// a=<attribute>
/// a=<attribute>:<value>
///
/// # Parameters
///
/// - `value`: The value part of the attribute line
///
/// # Returns
///
/// - `Ok(ParsedAttribute)` if parsing succeeds
/// - `Err` with error details if parsing fails
pub fn parse_attribute(value: &str) -> Result<ParsedAttribute> {
    // Check if this is a key-value attribute or a flag
    if let Some(colon_pos) = value.find(':') {
        let key = &value[0..colon_pos];
        let val = &value[colon_pos + 1..];
        
        // Handle different attribute types
        match key {
            // Media format attributes
            "rtpmap" => parse_rtpmap(val),
            "fmtp" => parse_fmtp(val),
            
            // Timing attributes
            "ptime" => {
                let ptime_val = ptime::parse_ptime(val)?;
                Ok(ParsedAttribute::Ptime(ptime_val as u64))
            },
            "maxptime" => {
                let maxptime_val = ptime::parse_maxptime(val)?;
                Ok(ParsedAttribute::MaxPtime(maxptime_val as u64))
            },
            
            // ICE attributes
            "ice-ufrag" => Ok(ParsedAttribute::IceUfrag(val.to_string())),
            "ice-pwd" => Ok(ParsedAttribute::IcePwd(val.to_string())),
            "ice-options" => {
                let options = val.split_whitespace().map(|s| s.to_string()).collect();
                Ok(ParsedAttribute::IceOptions(options))
            },
            "candidate" => parse_candidate(val),
            
            // DTLS attributes
            "fingerprint" => {
                let parts: Vec<&str> = val.splitn(2, ' ').collect();
                if parts.len() < 2 {
                    return Err(Error::SdpParsingError("Invalid fingerprint format".to_string()));
                }
                Ok(ParsedAttribute::Fingerprint(parts[0].to_string(), parts[1].to_string()))
            },
            "setup" => Ok(ParsedAttribute::Setup(val.to_string())),
            
            // Identification attributes
            "mid" => {
                let mid_val = mid::parse_mid(val)?;
                Ok(ParsedAttribute::Mid(mid_val))
            },
            "msid" => {
                let (stream_id, track_id) = msid::parse_msid(val)?;
                Ok(ParsedAttribute::Msid(stream_id, track_id))
            },
            "ssrc" => parse_ssrc(val),
            
            // Grouping attributes
            "group" => {
                let (semantics, tags) = group::parse_group(val)?;
                Ok(ParsedAttribute::Group(semantics, tags))
            },
            
            // RTCP attributes
            "rtcp-fb" => {
                let (pt, fb_type, fb_param) = rtcp::parse_rtcp_fb(val)?;
                Ok(ParsedAttribute::RtcpFb(pt, fb_type, fb_param))
            },
            
            // Extension attributes
            "extmap" => {
                let (id, direction, uri, params) = extmap::parse_extmap(val)?;
                // Convert id from u16 to u8, verifying it's in range
                if id > 255 {
                    return Err(Error::SdpParsingError(format!("Extmap id {} is out of range for u8", id)));
                }
                Ok(ParsedAttribute::ExtMap(id as u8, direction, uri, params))
            },
            
            // Simulcast & RID attributes
            "rid" => {
                let rid_attr = rid::parse_rid(val)?;
                Ok(ParsedAttribute::Rid(rid_attr))
            },
            "simulcast" => {
                let (send, recv) = simulcast::parse_simulcast(val)?;
                Ok(ParsedAttribute::Simulcast(send, recv))
            },
            
            // Data channel attributes
            "sctpmap" => {
                let (port, app, streams) = parse_sctpmap(val)?;
                Ok(ParsedAttribute::SctpMap(port, app, streams as u16))
            },
            "sctp-port" => Ok(ParsedAttribute::SctpPort(val.parse().map_err(|_| 
                Error::SdpParsingError(format!("Invalid sctp-port: {}", val)))?)),
            "max-message-size" => Ok(ParsedAttribute::MaxMessageSize(val.parse().map_err(|_| 
                Error::SdpParsingError(format!("Invalid max-message-size: {}", val)))?)),
            
            // Generic key-value attribute if no specialized parser
            _ => Ok(ParsedAttribute::Value(key.to_string(), val.to_string())),
        }
    } else {
        // Handle flag attributes
        match value {
            // Media direction attributes
            "sendrecv" => Ok(ParsedAttribute::Direction(MediaDirection::SendRecv)),
            "sendonly" => Ok(ParsedAttribute::Direction(MediaDirection::SendOnly)),
            "recvonly" => Ok(ParsedAttribute::Direction(MediaDirection::RecvOnly)),
            "inactive" => Ok(ParsedAttribute::Direction(MediaDirection::Inactive)),
            
            // RTCP multiplexing
            "rtcp-mux" => Ok(ParsedAttribute::RtcpMux),
            
            // ICE attributes
            "end-of-candidates" => Ok(ParsedAttribute::EndOfCandidates),
            "ice-lite" => Ok(ParsedAttribute::Flag("ice-lite".to_string())),
            
            // Generic flag attribute if no specialized parser
            _ => Ok(ParsedAttribute::Flag(value.to_string())),
        }
    }
} 