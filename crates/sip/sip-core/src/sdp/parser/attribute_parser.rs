//! SDP attribute parsing functionality
//!
//! This module handles parsing of SDP attribute lines (a=) according to RFC 8866 and related specifications.
//! SDP attributes appear in either the session or media level sections and provide additional information
//! about the session or media streams.
//!
//! The module supports parsing various attribute types including:
//! - Media attributes: rtpmap, fmtp, ptime, maxptime
//! - Direction attributes: sendrecv, sendonly, recvonly, inactive
//! - ICE attributes: ice-ufrag, ice-pwd, ice-options, candidate
//! - DTLS attributes: fingerprint, setup
//! - Identification attributes: mid, msid, ssrc
//! - Grouping attributes: group
//! - RTCP attributes: rtcp-fb, rtcp-mux
//! - Extension attributes: extmap
//! - Simulcast & RID attributes: rid, simulcast
//! - Data channel attributes: sctpmap, sctp-port, max-message-size
//!
//! Most attributes are categorized into either flag attributes (a=flag) or value attributes (a=key:value).

use std::str::FromStr;

use crate::error::{Error, Result};
use crate::sdp::attributes::MediaDirection;
use crate::types::sdp::{
    DcMapAttribute, DcsaAttribute, ParsedAttribute, RemoteCandidateAttribute, RtcpAttribute,
    SimulcastAlternative as SdpSimulcastAlternative,
    SimulcastDescription as SdpSimulcastDescription, SimulcastVersion as SdpSimulcastVersion,
    SsrcGroupAttribute,
};

// Import specialized parse functions
use crate::sdp::attributes::candidate::parse_candidate;
use crate::sdp::attributes::datachannel;
use crate::sdp::attributes::dtls;
use crate::sdp::attributes::extmap;
use crate::sdp::attributes::fmtp::parse_fmtp;
use crate::sdp::attributes::group;
use crate::sdp::attributes::ice;
use crate::sdp::attributes::mid;
use crate::sdp::attributes::msid;
use crate::sdp::attributes::ptime;
use crate::sdp::attributes::rid;
use crate::sdp::attributes::rid::RidDirection;
use crate::sdp::attributes::rtcp;
use crate::sdp::attributes::rtpmap::parse_rtpmap;
use crate::sdp::attributes::sctpmap::parse_sctpmap;
use crate::sdp::attributes::simulcast;
use crate::sdp::attributes::ssrc::parse_ssrc;

/// Parse an attribute line (a=) from SDP.
///
/// This function parses SDP attribute lines according to RFC 8866 and relevant extension RFCs.
/// It handles both flag attributes (a=flag) and key-value attributes (a=key:value).
///
/// # Format
///
/// There are two formats for SDP attributes:
/// - Flag attributes: `a=<flag>`
/// - Key-value attributes: `a=<key>:<value>`
///
/// # Parameters
///
/// - `value`: The value part of the attribute line (without the leading 'a=')
///
/// # Returns
///
/// - `Ok(ParsedAttribute)` if parsing succeeds
/// - `Err` with error details if parsing fails
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::sdp::parser::parse_attribute;
/// use rvoip_sip_core::types::sdp::ParsedAttribute;
/// use rvoip_sip_core::sdp::attributes::MediaDirection;
///
/// // Parse a flag attribute
/// let sendrecv = parse_attribute("sendrecv").unwrap();
/// assert!(matches!(sendrecv, ParsedAttribute::Direction(MediaDirection::SendRecv)));
///
/// // Parse a key-value attribute
/// let rtpmap = parse_attribute("rtpmap:96 VP8/90000").unwrap();
/// if let ParsedAttribute::RtpMap(map) = rtpmap {
///     assert_eq!(map.payload_type, 96);
///     assert_eq!(map.encoding_name, "VP8");
///     assert_eq!(map.clock_rate, 90000);
/// }
///
/// // Parse an ICE candidate
/// let candidate = parse_attribute("candidate:1 1 UDP 2113667327 192.168.1.4 46416 typ host").unwrap();
/// if let ParsedAttribute::Candidate(cand) = candidate {
///     assert_eq!(cand.foundation, "1");
///     assert_eq!(cand.connection_address, "192.168.1.4");
///     assert_eq!(cand.port, 46416);
/// }
/// ```
///
/// # Errors
///
/// Returns an error if:
/// - The attribute format is invalid
/// - A specialized parser for a specific attribute type fails
/// - Required attribute values are missing or in an incorrect format
///
/// # Specifications
///
/// - [RFC 8866: SDP](https://tools.ietf.org/html/rfc8866)
/// - [RFC 8839: ICE](https://tools.ietf.org/html/rfc8839)
/// - [RFC 8851: RID](https://tools.ietf.org/html/rfc8851)
/// - [RFC 8853: Simulcast](https://tools.ietf.org/html/rfc8853)
/// - [RFC 8122: DTLS-SRTP](https://tools.ietf.org/html/rfc8122)
/// - [RFC 5888: Grouping](https://tools.ietf.org/html/rfc5888)
/// - [RFC 8830: MSID](https://tools.ietf.org/html/rfc8830)
/// - [RFC 8285: RTP Header Extensions](https://tools.ietf.org/html/rfc8285)
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
            }
            "maxptime" => {
                let maxptime_val = ptime::parse_maxptime(val)?;
                Ok(ParsedAttribute::MaxPtime(maxptime_val as u64))
            }

            // ICE attributes
            "ice-ufrag" => Ok(ParsedAttribute::IceUfrag(ice::parse_ice_ufrag(val)?)),
            "ice-pwd" => Ok(ParsedAttribute::IcePwd(ice::parse_ice_pwd(val)?)),
            "ice-options" => {
                let options = ice::parse_ice_options(val)?;
                Ok(ParsedAttribute::IceOptions(options))
            }
            "candidate" => parse_candidate(val),
            "remote-candidates" => Ok(ParsedAttribute::RemoteCandidates(parse_remote_candidates(
                val,
            )?)),

            // DTLS attributes
            "fingerprint" => {
                let (hash, fingerprint) = dtls::parse_fingerprint(val)?;
                Ok(ParsedAttribute::Fingerprint(hash, fingerprint))
            }
            "setup" => Ok(ParsedAttribute::Setup(dtls::parse_setup(val)?)),
            "tls-id" => Ok(ParsedAttribute::TlsId(parse_nonempty_value("tls-id", val)?)),

            // SDES-SRTP `a=crypto:` (RFC 4568 §9.1).
            // Wire form: `<tag> <crypto-suite> inline:<base64-key>[|<lifetime>][|<MKI>:<MKI_LEN>] [<session-params>]`
            "crypto" => {
                use crate::types::sdp::{CryptoAttribute, CryptoSuite};
                let mut parts = val.split_whitespace();
                let tag = parts
                    .next()
                    .and_then(|s| s.parse::<u32>().ok())
                    .ok_or_else(|| {
                        Error::SdpParsingError("a=crypto: missing or non-numeric tag".to_string())
                    })?;
                let suite_str = parts.next().ok_or_else(|| {
                    Error::SdpParsingError("a=crypto: missing crypto-suite".to_string())
                })?;
                let suite = match CryptoSuite::from_str(suite_str) {
                    Ok(suite) => suite,
                    Err(_) => {
                        return Ok(ParsedAttribute::Value(
                            "crypto".to_string(),
                            val.to_string(),
                        ));
                    }
                };
                let key_param = parts.next().ok_or_else(|| {
                    Error::SdpParsingError("a=crypto: missing inline= parameter".to_string())
                })?;
                let key_inline = key_param.strip_prefix("inline:").ok_or_else(|| {
                    Error::SdpParsingError(format!(
                        "a=crypto: only `inline` key-method supported, got {}",
                        key_param
                    ))
                })?;
                // RFC 4568 §6.1 — lifetime + MKI ride along the
                // inline= value separated by `|`. Strip them off the
                // base64 key for the typed field but preserve the
                // entire string verbatim so callers needing
                // round-trip fidelity can parse them back themselves.
                let mut attr = CryptoAttribute::new(tag, suite, key_inline.to_string());
                // Remaining whitespace-delimited tokens are session-params.
                attr.session_params = parts.map(|s| s.to_string()).collect();
                Ok(ParsedAttribute::Crypto(attr))
            }

            // Identification attributes
            "mid" => {
                let mid_val = mid::parse_mid(val)?;
                Ok(ParsedAttribute::Mid(mid_val))
            }
            "msid" => {
                let (stream_id, track_id) = msid::parse_msid(val)?;
                Ok(ParsedAttribute::Msid(stream_id, track_id))
            }
            "msid-semantic" => {
                let (semantic, tokens) = parse_msid_semantic(val)?;
                Ok(ParsedAttribute::MsidSemantic(semantic, tokens))
            }
            "ssrc" => parse_ssrc(val),

            // Grouping attributes
            "group" => {
                let (semantics, tags) = group::parse_group(val)?;
                Ok(ParsedAttribute::Group(semantics, tags))
            }

            // RTCP attributes
            "rtcp" => Ok(ParsedAttribute::Rtcp(parse_rtcp_attribute(val)?)),
            "rtcp-fb" => {
                let (pt, fb_type, fb_param) = rtcp::parse_rtcp_fb(val)?;
                Ok(ParsedAttribute::RtcpFb(pt, fb_type, fb_param))
            }

            // Extension attributes
            "extmap" => {
                let (id, direction, uri, params) = extmap::parse_extmap(val)?;
                // Convert id from u16 to u8, verifying it's in range
                if id > 255 {
                    return Err(Error::SdpParsingError(format!(
                        "Extmap id {} is out of range for u8",
                        id
                    )));
                }
                Ok(ParsedAttribute::ExtMap(id as u8, direction, uri, params))
            }

            // Simulcast & RID attributes
            "rid" => {
                let rid_attr = rid::parse_rid(val)?;
                Ok(ParsedAttribute::Rid(rid_attr))
            }
            "simulcast" => Ok(ParsedAttribute::SimulcastStructured(
                parse_structured_simulcast(val)?,
            )),
            "ssrc-group" => Ok(ParsedAttribute::SsrcGroup(parse_ssrc_group(val)?)),

            // Data channel attributes
            "sctpmap" => {
                let (port, app, streams) = parse_sctpmap(val)?;
                Ok(ParsedAttribute::SctpMap(port, app, streams as u16))
            }
            "sctp-port" => Ok(ParsedAttribute::SctpPort(datachannel::parse_sctp_port(
                val,
            )?)),
            "max-message-size" => Ok(ParsedAttribute::MaxMessageSize(
                datachannel::parse_max_message_size(val)?,
            )),
            "dcmap" => Ok(ParsedAttribute::DcMap(parse_dcmap(val)?)),
            "dcsa" => Ok(ParsedAttribute::Dcsa(parse_dcsa(val)?)),

            // RFC 8866 standard attributes
            "cat" => Ok(ParsedAttribute::Category(parse_nonempty_value("cat", val)?)),
            "keywds" => Ok(ParsedAttribute::Keywords(parse_nonempty_value(
                "keywds", val,
            )?)),
            "tool" => Ok(ParsedAttribute::Tool(parse_nonempty_value("tool", val)?)),
            "orient" => Ok(ParsedAttribute::Orientation(parse_nonempty_value(
                "orient", val,
            )?)),
            "type" => Ok(ParsedAttribute::ConferenceType(parse_nonempty_value(
                "type", val,
            )?)),
            "charset" => Ok(ParsedAttribute::Charset(parse_nonempty_value(
                "charset", val,
            )?)),
            "sdplang" => Ok(ParsedAttribute::SdpLanguage(parse_nonempty_value(
                "sdplang", val,
            )?)),
            "lang" => Ok(ParsedAttribute::Language(parse_nonempty_value(
                "lang", val,
            )?)),
            "framerate" => Ok(ParsedAttribute::Framerate(parse_framerate(val)?)),
            "quality" => Ok(ParsedAttribute::Quality(parse_quality(val)?)),

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
            "rtcp-rsize" => Ok(ParsedAttribute::RtcpRsize),

            // ICE attributes
            "end-of-candidates" => Ok(ParsedAttribute::EndOfCandidates),
            "ice-lite" => Ok(ParsedAttribute::IceLite),

            // BUNDLE and RTP header extension markers
            "bundle-only" => Ok(ParsedAttribute::BundleOnly),
            "extmap-allow-mixed" => Ok(ParsedAttribute::ExtMapAllowMixed),

            // Generic flag attribute if no specialized parser
            _ => Ok(ParsedAttribute::Flag(value.to_string())),
        }
    }
}

fn parse_nonempty_value(attribute: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::SdpParsingError(format!(
            "Attribute '{}' requires a non-empty value",
            attribute
        )));
    }
    Ok(value.to_string())
}

fn parse_remote_candidates(value: &str) -> Result<Vec<RemoteCandidateAttribute>> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.is_empty() || parts.len() % 3 != 0 {
        return Err(Error::SdpParsingError(format!(
            "Invalid remote-candidates format: {}",
            value
        )));
    }

    let mut candidates = Vec::new();
    for chunk in parts.chunks(3) {
        let component_id = chunk[0].parse::<u32>().map_err(|_| {
            Error::SdpParsingError(format!(
                "Invalid remote-candidates component id: {}",
                chunk[0]
            ))
        })?;
        let port = chunk[2].parse::<u16>().map_err(|_| {
            Error::SdpParsingError(format!("Invalid remote-candidates port: {}", chunk[2]))
        })?;
        candidates.push(RemoteCandidateAttribute {
            component_id,
            connection_address: chunk[1].to_string(),
            port,
        });
    }

    Ok(candidates)
}

fn parse_msid_semantic(value: &str) -> Result<(String, Vec<String>)> {
    let mut parts = value.split_whitespace();
    let semantic = parts
        .next()
        .ok_or_else(|| Error::SdpParsingError("Missing msid-semantic semantic".to_string()))?;
    Ok((
        semantic.to_string(),
        parts.map(|part| part.to_string()).collect(),
    ))
}

fn parse_rtcp_attribute(value: &str) -> Result<RtcpAttribute> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    if parts.is_empty() {
        return Err(Error::SdpParsingError("Missing rtcp port".to_string()));
    }

    let port = parts[0]
        .parse::<u16>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid rtcp port: {}", parts[0])))?;

    match parts.len() {
        1 => Ok(RtcpAttribute {
            port,
            net_type: None,
            addr_type: None,
            connection_address: None,
        }),
        4 => Ok(RtcpAttribute {
            port,
            net_type: Some(parts[1].to_string()),
            addr_type: Some(parts[2].to_string()),
            connection_address: Some(parts[3].to_string()),
        }),
        _ => Err(Error::SdpParsingError(format!(
            "Invalid rtcp attribute format: {}",
            value
        ))),
    }
}

fn parse_structured_simulcast(value: &str) -> Result<Vec<SdpSimulcastDescription>> {
    let parsed = simulcast::parse_simulcast_struct(value)?;
    Ok(parsed
        .into_iter()
        .map(|desc| SdpSimulcastDescription {
            direction: match desc.direction {
                simulcast::SimulcastDirection::Send => RidDirection::Send,
                simulcast::SimulcastDirection::Recv => RidDirection::Recv,
            },
            versions: desc
                .stream_versions
                .into_iter()
                .map(|version| SdpSimulcastVersion {
                    alternatives: version
                        .alternatives
                        .into_iter()
                        .map(|alternative| SdpSimulcastAlternative {
                            rid: alternative.rid,
                            paused: matches!(
                                alternative.status,
                                simulcast::SimulcastStatus::Paused
                            ),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect())
}

fn parse_ssrc_group(value: &str) -> Result<SsrcGroupAttribute> {
    let mut parts = value.split_whitespace();
    let semantics = parts
        .next()
        .ok_or_else(|| Error::SdpParsingError("Missing ssrc-group semantics".to_string()))?;
    let mut ssrcs = Vec::new();
    for part in parts {
        ssrcs.push(
            part.parse::<u32>().map_err(|_| {
                Error::SdpParsingError(format!("Invalid ssrc-group SSRC: {}", part))
            })?,
        );
    }
    if ssrcs.is_empty() {
        return Err(Error::SdpParsingError(format!(
            "ssrc-group requires at least one SSRC: {}",
            value
        )));
    }
    Ok(SsrcGroupAttribute {
        semantics: semantics.to_string(),
        ssrcs,
    })
}

fn parse_dcmap(value: &str) -> Result<DcMapAttribute> {
    let mut parts = value.split_whitespace();
    let stream_id = parts
        .next()
        .ok_or_else(|| Error::SdpParsingError("Missing dcmap stream id".to_string()))?
        .parse::<u16>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid dcmap stream id: {}", value)))?;

    let parameters = parts
        .map(|part| {
            let mut kv = part.splitn(2, '=');
            let key = kv.next().unwrap_or("").to_string();
            let value = kv.next().map(|value| value.to_string());
            (key, value)
        })
        .collect();

    Ok(DcMapAttribute {
        stream_id,
        parameters,
    })
}

fn parse_dcsa(value: &str) -> Result<DcsaAttribute> {
    let mut parts = value.trim().splitn(2, char::is_whitespace);
    let stream_id = parts
        .next()
        .ok_or_else(|| Error::SdpParsingError("Missing dcsa stream id".to_string()))?
        .parse::<u16>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid dcsa stream id: {}", value)))?;
    let attribute = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::SdpParsingError("Missing dcsa attribute".to_string()))?;

    Ok(DcsaAttribute {
        stream_id,
        attribute: attribute.to_string(),
    })
}

fn parse_framerate(value: &str) -> Result<String> {
    let value = parse_nonempty_value("framerate", value)?;
    value
        .parse::<f32>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid framerate value: {}", value)))?;
    Ok(value)
}

fn parse_quality(value: &str) -> Result<u8> {
    let value = value.trim();
    let quality = value
        .parse::<u8>()
        .map_err(|_| Error::SdpParsingError(format!("Invalid quality value: {}", value)))?;
    if quality > 10 {
        return Err(Error::SdpParsingError(format!(
            "quality must be in the range 0-10: {}",
            quality
        )));
    }
    Ok(quality)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdp::attributes::rid::RidDirection;

    // --- Flag Attribute Tests ---

    #[test]
    fn test_parse_direction_attributes() {
        // Test all four media direction flags (RFC 8866 Section 6.7)
        assert!(matches!(
            parse_attribute("sendrecv").unwrap(),
            ParsedAttribute::Direction(MediaDirection::SendRecv)
        ));
        assert!(matches!(
            parse_attribute("sendonly").unwrap(),
            ParsedAttribute::Direction(MediaDirection::SendOnly)
        ));
        assert!(matches!(
            parse_attribute("recvonly").unwrap(),
            ParsedAttribute::Direction(MediaDirection::RecvOnly)
        ));
        assert!(matches!(
            parse_attribute("inactive").unwrap(),
            ParsedAttribute::Direction(MediaDirection::Inactive)
        ));
    }

    #[test]
    fn test_parse_rtcp_mux() {
        // Test RTCP multiplexing flag (RFC 8866 Section 6.7.1)
        assert!(matches!(
            parse_attribute("rtcp-mux").unwrap(),
            ParsedAttribute::RtcpMux
        ));
    }

    #[test]
    fn test_parse_ice_flags() {
        // Test ICE-related flags (RFC 8839 Section 5.2)
        assert!(matches!(
            parse_attribute("end-of-candidates").unwrap(),
            ParsedAttribute::EndOfCandidates
        ));

        assert!(matches!(
            parse_attribute("ice-lite").unwrap(),
            ParsedAttribute::IceLite
        ));
    }

    #[test]
    fn test_parse_custom_flag() {
        // Test custom flag attributes
        assert!(matches!(
            parse_attribute("custom-flag").unwrap(),
            ParsedAttribute::Flag(flag) if flag == "custom-flag"
        ));
    }

    // --- Value Attribute Tests ---

    #[test]
    fn test_parse_rtpmap() {
        // Test rtpmap attribute (RFC 8866 Section 6.6)
        let result = parse_attribute("rtpmap:0 PCMU/8000").unwrap();
        if let ParsedAttribute::RtpMap(rtpmap) = result {
            assert_eq!(rtpmap.payload_type, 0);
            assert_eq!(rtpmap.encoding_name, "PCMU");
            assert_eq!(rtpmap.clock_rate, 8000);
            assert_eq!(rtpmap.encoding_params, None);
        } else {
            panic!("Expected RtpMap attribute");
        }

        // Test rtpmap with encoding parameters
        let result = parse_attribute("rtpmap:111 opus/48000/2").unwrap();
        if let ParsedAttribute::RtpMap(rtpmap) = result {
            assert_eq!(rtpmap.payload_type, 111);
            assert_eq!(rtpmap.encoding_name, "opus");
            assert_eq!(rtpmap.clock_rate, 48000);
            assert_eq!(rtpmap.encoding_params, Some("2".to_string()));
        } else {
            panic!("Expected RtpMap attribute");
        }

        // Test invalid rtpmap format
        assert!(parse_attribute("rtpmap:").is_err());
        assert!(parse_attribute("rtpmap:abc").is_err());
    }

    #[test]
    fn test_parse_fmtp() {
        // Test fmtp attribute (RFC 8866 Section 6.6)
        let result = parse_attribute("fmtp:97 mode=20").unwrap();
        if let ParsedAttribute::Fmtp(fmtp) = result {
            assert_eq!(fmtp.format, "97");
            assert_eq!(fmtp.parameters, "mode=20");
        } else {
            panic!("Expected Fmtp attribute");
        }

        // Test fmtp with complex parameters
        let result =
            parse_attribute("fmtp:96 profile-level-id=42e01f;level-asymmetry-allowed=1").unwrap();
        if let ParsedAttribute::Fmtp(fmtp) = result {
            assert_eq!(fmtp.format, "96");
            assert_eq!(
                fmtp.parameters,
                "profile-level-id=42e01f;level-asymmetry-allowed=1"
            );
        } else {
            panic!("Expected Fmtp attribute");
        }

        // Test invalid fmtp format
        assert!(parse_attribute("fmtp:").is_err());
    }

    #[test]
    fn test_parse_ptime() {
        // Test ptime attribute (RFC 8866 Section 6.6)
        let result = parse_attribute("ptime:20").unwrap();
        if let ParsedAttribute::Ptime(ptime) = result {
            assert_eq!(ptime, 20);
        } else {
            panic!("Expected Ptime attribute");
        }

        // Test invalid ptime format
        assert!(parse_attribute("ptime:abc").is_err());
    }

    #[test]
    fn test_parse_maxptime() {
        // Test maxptime attribute (RFC 8866 Section 6.6)
        let result = parse_attribute("maxptime:40").unwrap();
        if let ParsedAttribute::MaxPtime(maxptime) = result {
            assert_eq!(maxptime, 40);
        } else {
            panic!("Expected MaxPtime attribute");
        }

        // Test invalid maxptime format
        assert!(parse_attribute("maxptime:abc").is_err());
    }

    #[test]
    fn test_parse_ice_attributes() {
        // Test ice-ufrag attribute (RFC 8839)
        let result = parse_attribute("ice-ufrag:F7gI").unwrap();
        if let ParsedAttribute::IceUfrag(ufrag) = result {
            assert_eq!(ufrag, "F7gI");
        } else {
            panic!("Expected IceUfrag attribute");
        }

        // Test ice-pwd attribute (RFC 8839)
        let result = parse_attribute("ice-pwd:x9cml/YzichV2+XlhiMu8g").unwrap();
        if let ParsedAttribute::IcePwd(pwd) = result {
            assert_eq!(pwd, "x9cml/YzichV2+XlhiMu8g");
        } else {
            panic!("Expected IcePwd attribute");
        }

        // Test ice-options attribute (RFC 8839)
        let result = parse_attribute("ice-options:trickle renomination").unwrap();
        if let ParsedAttribute::IceOptions(options) = result {
            assert_eq!(options.len(), 2);
            assert_eq!(options[0], "trickle");
            assert_eq!(options[1], "renomination");
        } else {
            panic!("Expected IceOptions attribute");
        }
    }

    #[test]
    fn test_parse_candidate() {
        // Test candidate attribute (RFC 8839 Section 5.1)
        let result =
            parse_attribute("candidate:1 1 UDP 2113667327 192.168.1.4 46416 typ host").unwrap();
        if let ParsedAttribute::Candidate(candidate) = result {
            assert_eq!(candidate.foundation, "1");
            assert_eq!(candidate.component_id, 1);
            assert_eq!(candidate.transport, "UDP");
            assert_eq!(candidate.priority, 2113667327);
            assert_eq!(candidate.connection_address, "192.168.1.4");
            assert_eq!(candidate.port, 46416);
            assert_eq!(candidate.candidate_type, "host");
            assert_eq!(candidate.related_address, None);
            assert_eq!(candidate.related_port, None);
        } else {
            panic!("Expected Candidate attribute");
        }

        // Test candidate with related address (RFC 8839 Section 5.1)
        let result = parse_attribute(
            "candidate:2 1 UDP 1694302207 1.2.3.4 46416 typ srflx raddr 192.168.1.4 rport 46416",
        )
        .unwrap();
        if let ParsedAttribute::Candidate(candidate) = result {
            assert_eq!(candidate.foundation, "2");
            assert_eq!(candidate.component_id, 1);
            assert_eq!(candidate.transport, "UDP");
            assert_eq!(candidate.priority, 1694302207);
            assert_eq!(candidate.connection_address, "1.2.3.4");
            assert_eq!(candidate.port, 46416);
            assert_eq!(candidate.candidate_type, "srflx");
            assert_eq!(candidate.related_address, Some("192.168.1.4".to_string()));
            assert_eq!(candidate.related_port, Some(46416));
        } else {
            panic!("Expected Candidate attribute");
        }
    }

    #[test]
    fn test_parse_fingerprint() {
        // Test fingerprint attribute (RFC 8122)
        let fingerprint = "D2:FA:0E:C3:22:59:5E:14:95:69:92:3D:13:B4:84:24:2C:C2:A2:C0:3E:FD:34:8E:5E:EA:6F:AF:52:CE:E6:0F";
        let result = parse_attribute(&format!("fingerprint:sha-256 {}", fingerprint)).unwrap();
        if let ParsedAttribute::Fingerprint(algorithm, value) = result {
            assert_eq!(algorithm, "sha-256");
            assert_eq!(value, fingerprint);
        } else {
            panic!("Expected Fingerprint attribute");
        }

        // Test invalid fingerprint format
        assert!(parse_attribute("fingerprint:sha-256").is_err());
    }

    #[test]
    fn test_parse_setup() {
        // Test setup attribute (RFC 4145)
        let result = parse_attribute("setup:actpass").unwrap();
        if let ParsedAttribute::Setup(role) = result {
            assert_eq!(role, "actpass");
        } else {
            panic!("Expected Setup attribute");
        }
    }

    #[test]
    fn test_parse_mid() {
        // Test mid attribute (RFC 5888)
        let result = parse_attribute("mid:audio").unwrap();
        if let ParsedAttribute::Mid(mid) = result {
            assert_eq!(mid, "audio");
        } else {
            panic!("Expected Mid attribute");
        }
    }

    #[test]
    fn test_parse_msid() {
        // Test msid attribute (RFC 8830)
        let result = parse_attribute("msid:stream1 track1").unwrap();
        if let ParsedAttribute::Msid(stream_id, track_id) = result {
            assert_eq!(stream_id, "stream1");
            assert_eq!(track_id, Some("track1".to_string()));
        } else {
            panic!("Expected Msid attribute");
        }

        // Test msid with only stream id
        let result = parse_attribute("msid:stream1").unwrap();
        if let ParsedAttribute::Msid(stream_id, track_id) = result {
            assert_eq!(stream_id, "stream1");
            assert_eq!(track_id, None);
        } else {
            panic!("Expected Msid attribute");
        }
    }

    #[test]
    fn test_parse_group() {
        // Test group attribute (RFC 5888)
        let result = parse_attribute("group:BUNDLE audio video").unwrap();
        if let ParsedAttribute::Group(semantics, mids) = result {
            assert_eq!(semantics, "BUNDLE");
            assert_eq!(mids.len(), 2);
            assert_eq!(mids[0], "audio");
            assert_eq!(mids[1], "video");
        } else {
            panic!("Expected Group attribute");
        }
    }

    #[test]
    fn test_parse_rtcp_fb() {
        // Test rtcp-fb attribute (RFC 4585)
        let result = parse_attribute("rtcp-fb:96 nack").unwrap();
        if let ParsedAttribute::RtcpFb(pt, fb_type, fb_param) = result {
            assert_eq!(pt, "96");
            assert_eq!(fb_type, "nack");
            assert_eq!(fb_param, None);
        } else {
            panic!("Expected RtcpFb attribute");
        }

        // Test rtcp-fb with feedback parameter
        let result = parse_attribute("rtcp-fb:96 nack pli").unwrap();
        if let ParsedAttribute::RtcpFb(pt, fb_type, fb_param) = result {
            assert_eq!(pt, "96");
            assert_eq!(fb_type, "nack");
            assert_eq!(fb_param, Some("pli".to_string()));
        } else {
            panic!("Expected RtcpFb attribute");
        }
    }

    #[test]
    fn test_parse_extmap() {
        // Test extmap attribute (RFC 8285)
        let result =
            parse_attribute("extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level").unwrap();
        if let ParsedAttribute::ExtMap(id, direction, uri, params) = result {
            assert_eq!(id, 1);
            assert_eq!(direction, None);
            assert_eq!(uri, "urn:ietf:params:rtp-hdrext:ssrc-audio-level");
            assert_eq!(params, None);
        } else {
            panic!("Expected ExtMap attribute");
        }

        // Test extmap with direction
        let result =
            parse_attribute("extmap:2/sendrecv urn:ietf:params:rtp-hdrext:toffset").unwrap();
        if let ParsedAttribute::ExtMap(id, direction, uri, params) = result {
            assert_eq!(id, 2);
            assert_eq!(direction, Some("sendrecv".to_string()));
            assert_eq!(uri, "urn:ietf:params:rtp-hdrext:toffset");
            assert_eq!(params, None);
        } else {
            panic!("Expected ExtMap attribute");
        }

        // Test extmap with parameters
        let result =
            parse_attribute("extmap:3 urn:ietf:params:rtp-hdrext:sdes:mid some-params").unwrap();
        if let ParsedAttribute::ExtMap(id, direction, uri, params) = result {
            assert_eq!(id, 3);
            assert_eq!(direction, None);
            assert_eq!(uri, "urn:ietf:params:rtp-hdrext:sdes:mid");
            assert_eq!(params, Some("some-params".to_string()));
        } else {
            panic!("Expected ExtMap attribute");
        }

        // Test extmap with id out of range
        assert!(parse_attribute("extmap:256 urn:ietf:params:rtp-hdrext:ssrc-audio-level").is_err());
    }

    #[test]
    fn test_parse_rid() {
        // Test rid attribute (RFC 8851)
        let result = parse_attribute("rid:low send pt=97").unwrap();
        if let ParsedAttribute::Rid(rid) = result {
            assert_eq!(rid.id, "low");
            assert_eq!(rid.direction, RidDirection::Send);
            assert_eq!(rid.formats, vec!["97"]);
            assert!(rid.restrictions.is_empty());
        } else {
            panic!("Expected Rid attribute");
        }

        // Test rid with restrictions
        // Note: Using space-separated restrictions instead of semicolon-separated
        // This matches the current behavior of the parser, though it's not RFC-compliant
        let result = parse_attribute("rid:high recv pt=96 max-width=1280 max-height=720").unwrap();
        if let ParsedAttribute::Rid(rid) = result {
            // Debug output to understand what's in the restrictions map
            println!("RID Restrictions count: {}", rid.restrictions.len());
            println!(
                "RID Restrictions keys: {:?}",
                rid.restrictions.keys().collect::<Vec<_>>()
            );

            assert_eq!(rid.id, "high");
            assert_eq!(rid.direction, RidDirection::Recv);
            assert_eq!(rid.formats, vec!["96"]);

            // Check for the expected restrictions
            assert_eq!(rid.restrictions.len(), 2);
            assert!(rid.restrictions.contains_key("max-width"));
            assert_eq!(rid.restrictions.get("max-width").unwrap(), "1280");
            assert!(rid.restrictions.contains_key("max-height"));
            assert_eq!(rid.restrictions.get("max-height").unwrap(), "720");
        } else {
            panic!("Expected Rid attribute");
        }
    }

    #[test]
    fn test_parse_simulcast() {
        // Test simulcast attribute (RFC 8853)
        let result = parse_attribute("simulcast:send low;mid;high").unwrap();
        if let ParsedAttribute::SimulcastStructured(descriptions) = result {
            assert_eq!(descriptions.len(), 1);
            assert_eq!(descriptions[0].versions.len(), 3);
            assert_eq!(descriptions[0].versions[0].alternatives[0].rid, "low");
            assert_eq!(descriptions[0].versions[1].alternatives[0].rid, "mid");
            assert_eq!(descriptions[0].versions[2].alternatives[0].rid, "high");
        } else {
            panic!("Expected structured Simulcast attribute");
        }

        // Test simulcast with both send and receive streams
        let result = parse_attribute("simulcast:send low,high recv low").unwrap();
        if let ParsedAttribute::SimulcastStructured(descriptions) = result {
            assert_eq!(descriptions.len(), 2);
            assert_eq!(descriptions[0].versions.len(), 1);
            assert_eq!(descriptions[0].versions[0].alternatives.len(), 2);
            assert_eq!(descriptions[0].versions[0].alternatives[0].rid, "low");
            assert_eq!(descriptions[0].versions[0].alternatives[1].rid, "high");
            assert_eq!(descriptions[1].versions[0].alternatives[0].rid, "low");
        } else {
            panic!("Expected structured Simulcast attribute");
        }
    }

    #[test]
    fn test_parse_standard_and_webrtc_extension_attributes() {
        assert!(matches!(
            parse_attribute("remote-candidates:1 192.0.2.1 5000 2 192.0.2.1 5001").unwrap(),
            ParsedAttribute::RemoteCandidates(candidates)
                if candidates.len() == 2 && candidates[0].component_id == 1
        ));
        assert!(matches!(
            parse_attribute("tls-id:abc123").unwrap(),
            ParsedAttribute::TlsId(id) if id == "abc123"
        ));
        assert!(matches!(
            parse_attribute("bundle-only").unwrap(),
            ParsedAttribute::BundleOnly
        ));
        assert!(matches!(
            parse_attribute("msid-semantic:WMS *").unwrap(),
            ParsedAttribute::MsidSemantic(semantic, tokens)
                if semantic == "WMS" && tokens == vec!["*".to_string()]
        ));
        assert!(matches!(
            parse_attribute("rtcp:9 IN IP4 0.0.0.0").unwrap(),
            ParsedAttribute::Rtcp(rtcp)
                if rtcp.port == 9 && rtcp.connection_address.as_deref() == Some("0.0.0.0")
        ));
        assert!(matches!(
            parse_attribute("rtcp-rsize").unwrap(),
            ParsedAttribute::RtcpRsize
        ));
        assert!(matches!(
            parse_attribute("extmap-allow-mixed").unwrap(),
            ParsedAttribute::ExtMapAllowMixed
        ));
        assert!(matches!(
            parse_attribute("ssrc-group:FID 1234 5678").unwrap(),
            ParsedAttribute::SsrcGroup(group)
                if group.semantics == "FID" && group.ssrcs == vec![1234, 5678]
        ));
        assert!(matches!(
            parse_attribute("dcmap:0 label=\"chat\" ordered=true").unwrap(),
            ParsedAttribute::DcMap(dcmap)
                if dcmap.stream_id == 0 && dcmap.parameters.len() == 2
        ));
        assert!(matches!(
            parse_attribute("dcsa:0 fmtp:webrtc-datachannel max-message-size=262144").unwrap(),
            ParsedAttribute::Dcsa(dcsa)
                if dcsa.stream_id == 0 && dcsa.attribute.starts_with("fmtp:")
        ));
        assert!(matches!(
            parse_attribute("cat:meeting").unwrap(),
            ParsedAttribute::Category(value) if value == "meeting"
        ));
        assert!(matches!(
            parse_attribute("keywds:voice video").unwrap(),
            ParsedAttribute::Keywords(value) if value == "voice video"
        ));
        assert!(matches!(
            parse_attribute("tool:rvoip").unwrap(),
            ParsedAttribute::Tool(value) if value == "rvoip"
        ));
        assert!(matches!(
            parse_attribute("orient:portrait").unwrap(),
            ParsedAttribute::Orientation(value) if value == "portrait"
        ));
        assert!(matches!(
            parse_attribute("type:broadcast").unwrap(),
            ParsedAttribute::ConferenceType(value) if value == "broadcast"
        ));
        assert!(matches!(
            parse_attribute("charset:UTF-8").unwrap(),
            ParsedAttribute::Charset(value) if value == "UTF-8"
        ));
        assert!(matches!(
            parse_attribute("sdplang:en-US").unwrap(),
            ParsedAttribute::SdpLanguage(value) if value == "en-US"
        ));
        assert!(matches!(
            parse_attribute("lang:en").unwrap(),
            ParsedAttribute::Language(value) if value == "en"
        ));
        assert!(matches!(
            parse_attribute("framerate:29.97").unwrap(),
            ParsedAttribute::Framerate(value) if value == "29.97"
        ));
        assert!(matches!(
            parse_attribute("quality:10").unwrap(),
            ParsedAttribute::Quality(10)
        ));
    }

    #[test]
    fn test_parse_data_channel_attributes() {
        // Test sctp-port attribute (RFC 8841)
        let result = parse_attribute("sctp-port:5000").unwrap();
        if let ParsedAttribute::SctpPort(port) = result {
            assert_eq!(port, 5000);
        } else {
            panic!("Expected SctpPort attribute");
        }

        // Test max-message-size attribute (RFC 8841)
        let result = parse_attribute("max-message-size:262144").unwrap();
        if let ParsedAttribute::MaxMessageSize(size) = result {
            assert_eq!(size, 262144);
        } else {
            panic!("Expected MaxMessageSize attribute");
        }

        // Test sctpmap attribute (RFC 4960, older format)
        let result = parse_attribute("sctpmap:5000 webrtc-datachannel 1024").unwrap();
        if let ParsedAttribute::SctpMap(port, app, streams) = result {
            assert_eq!(port, 5000);
            assert_eq!(app, "webrtc-datachannel");
            assert_eq!(streams, 1024);
        } else {
            panic!("Expected SctpMap attribute");
        }

        // Test invalid data channel attributes
        assert!(parse_attribute("sctp-port:abc").is_err());
        assert!(parse_attribute("max-message-size:abc").is_err());
    }

    #[test]
    fn test_parse_generic_attributes() {
        // Test generic key-value attribute
        let result = parse_attribute("custom-attr:some-value").unwrap();
        if let ParsedAttribute::Value(key, value) = result {
            assert_eq!(key, "custom-attr");
            assert_eq!(value, "some-value");
        } else {
            panic!("Expected Value attribute");
        }
    }

    #[test]
    fn test_parse_crypto_accepts_rfc6188_aes256_name() {
        let result = parse_attribute(
            "crypto:2 AES_256_CM_HMAC_SHA1_80 inline:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==",
        )
        .unwrap();
        if let ParsedAttribute::Crypto(attr) = result {
            assert_eq!(attr.tag, 2);
            assert_eq!(
                attr.suite,
                crate::types::sdp::CryptoSuite::AesCm256HmacSha1_80
            );
        } else {
            panic!("Expected Crypto attribute");
        }
    }

    #[test]
    fn test_parse_crypto_preserves_unknown_suite_as_value_attribute() {
        let raw = "3 AEAD_AES_128_GCM inline:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";
        let result = parse_attribute(&format!("crypto:{}", raw)).unwrap();
        if let ParsedAttribute::Value(key, value) = result {
            assert_eq!(key, "crypto");
            assert_eq!(value, raw);
        } else {
            panic!("Expected generic Value attribute for unsupported crypto suite");
        }
    }
}
