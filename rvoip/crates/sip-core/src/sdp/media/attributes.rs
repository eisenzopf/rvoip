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