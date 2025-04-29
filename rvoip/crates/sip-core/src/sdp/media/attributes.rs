// Media attribute handling for SDP
//
// Functions for working with media-level attributes

use crate::error::{Error, Result};
use crate::types::sdp::{MediaDescription, ParsedAttribute};
use crate::sdp::attributes::MediaDirection;

/// Update a media description with a parsed attribute
pub fn update_media_with_attribute(media: &mut MediaDescription, attr: ParsedAttribute) -> Result<()> {
    match attr {
        ParsedAttribute::Direction(dir) => {
            media.direction = Some(dir);
        },
        ParsedAttribute::Ptime(ptime) => {
            media.ptime = Some(ptime as u32);
        },
        // Handle other specific media attributes that map to fields
        // ...
        
        // All other attributes go to generic_attributes
        _ => {
            media.generic_attributes.push(attr);
        }
    }
    
    Ok(())
}

/// Parses media-level attributes for a media description
pub fn parse_media_attributes(media: &mut MediaDescription, attributes: Vec<ParsedAttribute>) {
    // Add all attributes to the media description
    media.generic_attributes = attributes;
}

/// Checks if the given attribute is a media-level attribute
pub fn is_media_level_attribute(attribute: &str) -> bool {
    matches!(attribute,
        "rtpmap" | "fmtp" | "ptime" | "maxptime" | "direction" | "sendrecv" | "sendonly" |
        "recvonly" | "inactive" | "candidate" | "ssrc" | "ssrc-group" | "rtcp" | "rtcp-mux" |
        "rtcp-fb" | "extmap" | "mid" | "msid" | "setup" | "fingerprint" | "ice-ufrag" |
        "ice-pwd" | "ice-options" | "ice-lite" | "rid" | "simulcast" | "imageattr" | "sctpmap" |
        "max-message-size" | "sctp-port"
    )
}

/// Get media direction from attributes if set
pub fn get_media_direction(media: &MediaDescription) -> Option<MediaDirection> {
    media.direction
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::sdp::{SsrcAttribute, RtpMapAttribute};
    
    #[test]
    fn test_update_media_with_direction_attribute() {
        let mut media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
        let attr = ParsedAttribute::Direction(MediaDirection::SendRecv);
        
        let result = update_media_with_attribute(&mut media, attr);
        assert!(result.is_ok(), "Failed to update media with direction attribute");
        assert_eq!(media.direction, Some(MediaDirection::SendRecv), "Media direction not updated correctly");
        assert_eq!(media.generic_attributes.len(), 0, "Direction should not be added to generic attributes");
    }
    
    #[test]
    fn test_update_media_with_ptime_attribute() {
        let mut media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
        let attr = ParsedAttribute::Ptime(20);
        
        let result = update_media_with_attribute(&mut media, attr);
        assert!(result.is_ok(), "Failed to update media with ptime attribute");
        assert_eq!(media.ptime, Some(20), "Media ptime not updated correctly");
        assert_eq!(media.generic_attributes.len(), 0, "Ptime should not be added to generic attributes");
    }
    
    #[test]
    fn test_update_media_with_generic_attribute() {
        let mut media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
        let ssrc_attr = SsrcAttribute {
            ssrc_id: 12345,
            attribute: "cname".to_string(),
            value: Some("user@example.com".to_string()),
        };
        let attr = ParsedAttribute::Ssrc(ssrc_attr.clone());
        
        let result = update_media_with_attribute(&mut media, attr);
        assert!(result.is_ok(), "Failed to update media with generic attribute");
        assert_eq!(media.generic_attributes.len(), 1, "Generic attribute not added");
        
        if let ParsedAttribute::Ssrc(stored_attr) = &media.generic_attributes[0] {
            assert_eq!(stored_attr.ssrc_id, ssrc_attr.ssrc_id, "SSRC ID not stored correctly");
            assert_eq!(stored_attr.attribute, ssrc_attr.attribute, "SSRC attribute not stored correctly");
            assert_eq!(stored_attr.value, ssrc_attr.value, "SSRC value not stored correctly");
        } else {
            panic!("Stored attribute is not of type SSRC");
        }
    }
    
    #[test]
    fn test_parse_media_attributes() {
        let mut media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
        let attrs = vec![
            ParsedAttribute::Direction(MediaDirection::SendOnly),
            ParsedAttribute::Ptime(20),
        ];
        
        parse_media_attributes(&mut media, attrs.clone());
        
        assert_eq!(media.generic_attributes.len(), 2, "Incorrect number of attributes added");
        assert_eq!(media.generic_attributes, attrs, "Attributes not stored correctly");
    }
    
    #[test]
    fn test_is_media_level_attribute() {
        // Test media level attributes according to RFC 4566, RFC 5245, etc.
        let media_attrs = [
            "rtpmap", "fmtp", "ptime", "maxptime", "sendrecv", "sendonly",
            "recvonly", "inactive", "ssrc", "rtcp", "rtcp-mux", "mid", "extmap"
        ];
        
        for attr in &media_attrs {
            assert!(is_media_level_attribute(attr), "Failed to recognize media attribute: {}", attr);
        }
        
        // Test non-media level attributes
        let non_media_attrs = [
            "group", "tool", "timing", "charset", "invalid-attribute"
        ];
        
        for attr in &non_media_attrs {
            assert!(!is_media_level_attribute(attr), "Incorrectly recognized as media attribute: {}", attr);
        }
    }
    
    #[test]
    fn test_get_media_direction() {
        let mut media = MediaDescription::new("audio", 49170, "RTP/AVP", vec!["0".to_string()]);
        assert_eq!(get_media_direction(&media), None, "Default media should have no direction");
        
        media.direction = Some(MediaDirection::SendRecv);
        assert_eq!(get_media_direction(&media), Some(MediaDirection::SendRecv), "Failed to get SendRecv direction");
        
        media.direction = Some(MediaDirection::SendOnly);
        assert_eq!(get_media_direction(&media), Some(MediaDirection::SendOnly), "Failed to get SendOnly direction");
        
        media.direction = Some(MediaDirection::RecvOnly);
        assert_eq!(get_media_direction(&media), Some(MediaDirection::RecvOnly), "Failed to get RecvOnly direction");
        
        media.direction = Some(MediaDirection::Inactive);
        assert_eq!(get_media_direction(&media), Some(MediaDirection::Inactive), "Failed to get Inactive direction");
    }
} 