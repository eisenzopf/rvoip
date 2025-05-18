//! RTP Header Extensions Module
//!
//! Implements the RTP header extensions as defined in RFC 8285.
//! 
//! This module supports both one-byte and two-byte header extension formats,
//! as well as the legacy RFC 5285 format.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::collections::HashMap;
use std::fmt;
use tracing::{debug, warn};

use crate::error::Error;
use crate::Result;

/// Magic signature for one-byte header extensions (0xBEDE)
pub const ONE_BYTE_EXTENSION_ID: u16 = 0xBEDE;

/// Magic signature for two-byte header extensions (0x1000 + variant bits)
pub const TWO_BYTE_EXTENSION_ID_BASE: u16 = 0x1000;

/// Extension format types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionFormat {
    /// Legacy extensions (RFC 5285 backward compatibility)
    Legacy,
    /// One-byte header extensions (RFC 8285)
    OneByte,
    /// Two-byte header extensions (RFC 8285)
    TwoByte,
}

impl ExtensionFormat {
    /// Determine the extension format from the extension ID
    pub fn from_extension_id(id: u16) -> Self {
        if id == ONE_BYTE_EXTENSION_ID {
            Self::OneByte
        } else if (id & 0xFFF0) == TWO_BYTE_EXTENSION_ID_BASE {
            Self::TwoByte
        } else {
            Self::Legacy
        }
    }
}

/// A single RTP header extension element
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionElement {
    /// The extension ID (1-14 for one-byte, 1-255 for two-byte)
    pub id: u8,
    /// The extension data
    pub data: Bytes,
}

impl ExtensionElement {
    /// Create a new extension element
    pub fn new(id: u8, data: impl Into<Bytes>) -> Self {
        Self {
            id,
            data: data.into(),
        }
    }
    
    /// Validate that this extension element is valid for the given format
    pub fn validate(&self, format: ExtensionFormat) -> Result<()> {
        // Check ID range
        match format {
            ExtensionFormat::OneByte => {
                if self.id < 1 || self.id > 14 {
                    return Err(Error::InvalidParameter(format!(
                        "Invalid extension ID for one-byte format: {} (must be 1-14)",
                        self.id
                    )));
                }
                
                if self.data.len() > 16 {
                    return Err(Error::InvalidParameter(format!(
                        "Invalid extension length for one-byte format: {} (must be ≤ 16 bytes)",
                        self.data.len()
                    )));
                }
            },
            ExtensionFormat::TwoByte => {
                if self.id < 1 || self.id > 255 {
                    return Err(Error::InvalidParameter(format!(
                        "Invalid extension ID for two-byte format: {} (must be 1-255)",
                        self.id
                    )));
                }
                
                if self.data.len() > 255 {
                    return Err(Error::InvalidParameter(format!(
                        "Invalid extension length for two-byte format: {} (must be ≤ 255 bytes)",
                        self.data.len()
                    )));
                }
            },
            ExtensionFormat::Legacy => {
                // No specific validation for legacy format
            },
        }
        
        Ok(())
    }
}

/// RTP Header Extensions container
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpHeaderExtensions {
    /// The extension format
    pub format: ExtensionFormat,
    /// The extension profile ID
    pub profile_id: u16,
    /// The extension elements
    pub elements: Vec<ExtensionElement>,
}

impl Default for RtpHeaderExtensions {
    fn default() -> Self {
        Self {
            format: ExtensionFormat::OneByte, // Default to one-byte format
            profile_id: ONE_BYTE_EXTENSION_ID,
            elements: Vec::new(),
        }
    }
}

impl RtpHeaderExtensions {
    /// Create a new empty header extensions container with the one-byte format
    pub fn new_one_byte() -> Self {
        Self {
            format: ExtensionFormat::OneByte,
            profile_id: ONE_BYTE_EXTENSION_ID,
            elements: Vec::new(),
        }
    }
    
    /// Create a new empty header extensions container with the two-byte format
    pub fn new_two_byte() -> Self {
        Self {
            format: ExtensionFormat::TwoByte,
            profile_id: TWO_BYTE_EXTENSION_ID_BASE,
            elements: Vec::new(),
        }
    }
    
    /// Create a new header extensions container with a legacy profile ID
    pub fn new_legacy(profile_id: u16) -> Self {
        Self {
            format: ExtensionFormat::Legacy,
            profile_id,
            elements: Vec::new(),
        }
    }
    
    /// Create header extensions from existing extension data
    pub fn from_extension_data(profile_id: u16, data: &[u8]) -> Result<Self> {
        let format = ExtensionFormat::from_extension_id(profile_id);
        
        let elements = match format {
            ExtensionFormat::OneByte => Self::parse_one_byte_extensions(data)?,
            ExtensionFormat::TwoByte => Self::parse_two_byte_extensions(data)?,
            ExtensionFormat::Legacy => {
                // Legacy format doesn't have a standard parsing method
                // Store the entire block as a single extension
                vec![ExtensionElement::new(0, Bytes::copy_from_slice(data))]
            }
        };
        
        Ok(Self {
            format,
            profile_id,
            elements,
        })
    }
    
    /// Add an extension element
    pub fn add_extension(&mut self, id: u8, data: impl Into<Bytes>) -> Result<()> {
        let element = ExtensionElement::new(id, data);
        element.validate(self.format)?;
        self.elements.push(element);
        Ok(())
    }
    
    /// Get an extension by ID
    pub fn get_extension(&self, id: u8) -> Option<&ExtensionElement> {
        self.elements.iter().find(|ext| ext.id == id)
    }
    
    /// Remove an extension by ID
    pub fn remove_extension(&mut self, id: u8) -> Option<ExtensionElement> {
        if let Some(idx) = self.elements.iter().position(|ext| ext.id == id) {
            Some(self.elements.remove(idx))
        } else {
            None
        }
    }
    
    /// Clear all extensions
    pub fn clear(&mut self) {
        self.elements.clear();
    }
    
    /// Check if the extensions container is empty
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }
    
    /// Get the number of extensions
    pub fn len(&self) -> usize {
        self.elements.len()
    }
    
    /// Calculate the total size of the extension data in bytes
    pub fn size(&self) -> usize {
        let mut size = 0;
        
        match self.format {
            ExtensionFormat::OneByte => {
                for element in &self.elements {
                    // ID + length byte + data
                    size += 1 + element.data.len();
                }
                
                // Add padding to align to 4 bytes
                if size % 4 != 0 {
                    size += 4 - (size % 4);
                }
            },
            ExtensionFormat::TwoByte => {
                for element in &self.elements {
                    // ID + length byte + data
                    size += 2 + element.data.len();
                }
                
                // Add padding to align to 4 bytes
                if size % 4 != 0 {
                    size += 4 - (size % 4);
                }
            },
            ExtensionFormat::Legacy => {
                // Legacy format is just raw data
                if let Some(element) = self.elements.first() {
                    size = element.data.len();
                    
                    // Add padding to align to 4 bytes
                    if size % 4 != 0 {
                        size += 4 - (size % 4);
                    }
                }
            },
        }
        
        size
    }
    
    /// Serialize the extensions to a buffer
    pub fn serialize(&self) -> Result<BytesMut> {
        let size = self.size();
        let mut buf = BytesMut::with_capacity(size);
        
        match self.format {
            ExtensionFormat::OneByte => {
                // Serialize one-byte header extensions
                for element in &self.elements {
                    element.validate(self.format)?;
                    
                    // Make sure ID is in range 1-14
                    if element.id < 1 || element.id > 14 {
                        return Err(Error::InvalidParameter(format!(
                            "Invalid extension ID for one-byte format: {} (must be 1-14)",
                            element.id
                        )));
                    }
                    
                    // Each extension starts with a byte where the 4 most significant bits
                    // are the ID and the 4 least significant bits are the length minus 1
                    let length = element.data.len();
                    if length > 16 {
                        return Err(Error::InvalidParameter(format!(
                            "Extension data too long for one-byte header: {} (max is 16 bytes)",
                            length
                        )));
                    }
                    
                    // The extension byte combines ID and length-1
                    let ext_byte = (element.id << 4) | ((length as u8 - 1) & 0x0F);
                    buf.put_u8(ext_byte);
                    
                    // Then the extension data
                    buf.put_slice(&element.data);
                }
                
                // End marker
                buf.put_u8(0x00);
                
                // Add padding to align to 32-bit boundary
                while buf.len() % 4 != 0 {
                    buf.put_u8(0x00);
                }
            },
            ExtensionFormat::TwoByte => {
                // Serialize two-byte header extensions
                for element in &self.elements {
                    element.validate(self.format)?;
                    
                    // Make sure ID is in range 1-255
                    if element.id < 1 {
                        return Err(Error::InvalidParameter(format!(
                            "Invalid extension ID for two-byte format: {} (must be 1-255)",
                            element.id
                        )));
                    }
                    
                    // Two-byte header has ID and length as separate bytes
                    buf.put_u8(element.id);
                    
                    let length = element.data.len();
                    if length > 255 {
                        return Err(Error::InvalidParameter(format!(
                            "Extension data too long for two-byte header: {} (max is 255 bytes)",
                            length
                        )));
                    }
                    
                    buf.put_u8(length as u8);
                    
                    // Then the extension data
                    buf.put_slice(&element.data);
                }
                
                // End marker (ID = 0)
                buf.put_u8(0x00);
                
                // Add padding to align to 32-bit boundary
                while buf.len() % 4 != 0 {
                    buf.put_u8(0x00);
                }
            },
            ExtensionFormat::Legacy => {
                // Legacy format is just raw data
                if let Some(element) = self.elements.first() {
                    buf.put_slice(&element.data);
                    
                    // Add padding to align to 32-bit boundary
                    while buf.len() % 4 != 0 {
                        buf.put_u8(0x00);
                    }
                }
            },
        }
        
        Ok(buf)
    }
    
    // Parse one-byte extensions (RFC 8285)
    fn parse_one_byte_extensions(data: &[u8]) -> Result<Vec<ExtensionElement>> {
        let mut elements = Vec::new();
        let mut i = 0;
        
        while i < data.len() {
            let header_byte = data[i];
            i += 1;
            
            // End of extensions is marked by a 0 byte
            if header_byte == 0 {
                break;
            }
            
            // If the 0xF0 bits are all set, this is padding
            if (header_byte & 0xF0) == 0xF0 {
                continue;
            }
            
            // Extract the ID (top 4 bits)
            let id = (header_byte >> 4) & 0x0F;
            
            // Length is stored as L-1, so need to add 1 (bottom 4 bits)
            let length = ((header_byte & 0x0F) + 1) as usize;
            
            // Make sure we have enough data left
            if i + length > data.len() {
                return Err(Error::BufferTooSmall {
                    required: i + length,
                    available: data.len(),
                });
            }
            
            // Extract the extension data
            let ext_data = Bytes::copy_from_slice(&data[i..i+length]);
            i += length;
            
            elements.push(ExtensionElement::new(id, ext_data));
        }
        
        Ok(elements)
    }
    
    // Parse two-byte extensions (RFC 8285)
    fn parse_two_byte_extensions(data: &[u8]) -> Result<Vec<ExtensionElement>> {
        let mut elements = Vec::new();
        let mut i = 0;
        
        while i < data.len() {
            // Extract the ID
            let id = data[i];
            i += 1;
            
            // End of extensions is marked by a 0 ID
            if id == 0 {
                break;
            }
            
            // Make sure we have enough data for the length
            if i >= data.len() {
                return Err(Error::BufferTooSmall {
                    required: i,
                    available: data.len(),
                });
            }
            
            // Extract the length
            let length = data[i] as usize;
            i += 1;
            
            // Make sure we have enough data left
            if i + length > data.len() {
                return Err(Error::BufferTooSmall {
                    required: i + length,
                    available: data.len(),
                });
            }
            
            // Extract the extension data
            let ext_data = Bytes::copy_from_slice(&data[i..i+length]);
            i += length;
            
            elements.push(ExtensionElement::new(id, ext_data));
        }
        
        Ok(elements)
    }
}

/// Common extension ID constants
pub mod ids {
    /// SSRC audio level (RFC 6464)
    pub const AUDIO_LEVEL: u8 = 1;
    
    /// RTP timestamp offset (RFC 5450)
    pub const TIMESTAMP_OFFSET: u8 = 2;
    
    /// Video orientation (RFC 7742) 
    pub const VIDEO_ORIENTATION: u8 = 3;
    
    /// Transport-Wide Congestion Control (TWCC)
    pub const TRANSPORT_CC: u8 = 4;
    
    /// Frame marking for video (ID 5) (draft-ietf-avtext-framemarking)
    pub const FRAME_MARKING: u8 = 5;
    
    /// SDES items, such as MID, RID, etc.
    pub const SDES: u8 = 6;
}

// Re-export common extension IDs
pub use ids::*;

/// Common extension URI constants
pub mod uris {
    /// SSRC audio level (RFC 6464)
    pub const AUDIO_LEVEL: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
    
    /// RTP timestamp offset (RFC 5450)
    pub const TIMESTAMP_OFFSET: &str = "urn:ietf:params:rtp-hdrext:toffset";
    
    /// Video orientation (RFC 7742)
    pub const VIDEO_ORIENTATION: &str = "urn:3gpp:video-orientation";
    
    /// Transport-Wide Congestion Control (TWCC)
    pub const TRANSPORT_CC: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";
    
    /// Frame marking for video (draft-ietf-avtext-framemarking)
    pub const FRAME_MARKING: &str = "urn:ietf:params:rtp-hdrext:framemarking";
    
    /// Stream ID (RFC 8852)
    pub const STREAM_ID: &str = "urn:ietf:params:rtp-hdrext:sdes:rtp-stream-id";
    
    /// Repaired RTP Stream ID (RFC 8852)
    pub const REPAIR_RTP_STREAM_ID: &str = "urn:ietf:params:rtp-hdrext:sdes:repaired-rtp-stream-id";
    
    /// Media stream identification (MID) (RFC 8843)
    pub const MID: &str = "urn:ietf:params:rtp-hdrext:sdes:mid";
    
    /// Transmission time offsets for congestion control (RFC 5450)
    pub const ABS_SEND_TIME: &str = "http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time";
    
    /// Color space information for video (draft-ietf-avtext-color-space)
    pub const COLOR_SPACE: &str = "http://www.webrtc.org/experiments/rtp-hdrext/color-space";
    
    /// Video content type (screen content vs camera)
    pub const VIDEO_CONTENT_TYPE: &str = "http://www.webrtc.org/experiments/rtp-hdrext/video-content-type";
    
    /// Video timing for synchronization
    pub const VIDEO_TIMING: &str = "http://www.webrtc.org/experiments/rtp-hdrext/video-timing";
    
    /// Playout delay for audio/video synchronization
    pub const PLAYOUT_DELAY: &str = "http://www.webrtc.org/experiments/rtp-hdrext/playout-delay";
}

// Re-export common extension URIs
pub use uris::*;

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extension_format_detection() {
        assert_eq!(ExtensionFormat::from_extension_id(0xBEDE), ExtensionFormat::OneByte);
        assert_eq!(ExtensionFormat::from_extension_id(0x1000), ExtensionFormat::TwoByte);
        assert_eq!(ExtensionFormat::from_extension_id(0x1005), ExtensionFormat::TwoByte);
        assert_eq!(ExtensionFormat::from_extension_id(0x100F), ExtensionFormat::TwoByte);
        assert_eq!(ExtensionFormat::from_extension_id(0x2000), ExtensionFormat::Legacy);
        assert_eq!(ExtensionFormat::from_extension_id(0x0000), ExtensionFormat::Legacy);
    }
    
    #[test]
    fn test_one_byte_extensions() {
        let mut ext = RtpHeaderExtensions::new_one_byte();
        
        // Add some extensions
        ext.add_extension(1, vec![1, 2, 3, 4]).unwrap();
        ext.add_extension(2, vec![5, 6]).unwrap();
        ext.add_extension(3, vec![7]).unwrap();
        
        // Validate properties
        assert_eq!(ext.format, ExtensionFormat::OneByte);
        assert_eq!(ext.profile_id, ONE_BYTE_EXTENSION_ID);
        assert_eq!(ext.len(), 3);
        assert!(!ext.is_empty());
        
        // Check content
        assert_eq!(ext.get_extension(1).unwrap().data, Bytes::from_static(&[1, 2, 3, 4]));
        assert_eq!(ext.get_extension(2).unwrap().data, Bytes::from_static(&[5, 6]));
        assert_eq!(ext.get_extension(3).unwrap().data, Bytes::from_static(&[7]));
        
        // Serialize and parse back
        let serialized = ext.serialize().unwrap();
        let parsed = RtpHeaderExtensions::from_extension_data(ONE_BYTE_EXTENSION_ID, &serialized).unwrap();
        
        // Verify parsed content
        assert_eq!(parsed.format, ExtensionFormat::OneByte);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed.get_extension(1).unwrap().data, Bytes::from_static(&[1, 2, 3, 4]));
        assert_eq!(parsed.get_extension(2).unwrap().data, Bytes::from_static(&[5, 6]));
        assert_eq!(parsed.get_extension(3).unwrap().data, Bytes::from_static(&[7]));
    }
    
    #[test]
    fn test_two_byte_extensions() {
        let mut ext = RtpHeaderExtensions::new_two_byte();
        
        // Add some extensions
        ext.add_extension(1, vec![1, 2, 3, 4]).unwrap();
        ext.add_extension(2, vec![5, 6]).unwrap();
        ext.add_extension(100, vec![7]).unwrap(); // ID > 14 for two-byte
        
        // Validate properties
        assert_eq!(ext.format, ExtensionFormat::TwoByte);
        assert_eq!(ext.profile_id, TWO_BYTE_EXTENSION_ID_BASE);
        assert_eq!(ext.len(), 3);
        assert!(!ext.is_empty());
        
        // Check content
        assert_eq!(ext.get_extension(1).unwrap().data, Bytes::from_static(&[1, 2, 3, 4]));
        assert_eq!(ext.get_extension(2).unwrap().data, Bytes::from_static(&[5, 6]));
        assert_eq!(ext.get_extension(100).unwrap().data, Bytes::from_static(&[7]));
        
        // Serialize and parse back
        let serialized = ext.serialize().unwrap();
        let parsed = RtpHeaderExtensions::from_extension_data(TWO_BYTE_EXTENSION_ID_BASE, &serialized).unwrap();
        
        // Verify parsed content
        assert_eq!(parsed.format, ExtensionFormat::TwoByte);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed.get_extension(1).unwrap().data, Bytes::from_static(&[1, 2, 3, 4]));
        assert_eq!(parsed.get_extension(2).unwrap().data, Bytes::from_static(&[5, 6]));
        assert_eq!(parsed.get_extension(100).unwrap().data, Bytes::from_static(&[7]));
    }
    
    #[test]
    fn test_legacy_extensions() {
        // Legacy extensions just store raw data
        let raw_data = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let legacy_ext = RtpHeaderExtensions::from_extension_data(0x1234, &raw_data).unwrap();
        
        assert_eq!(legacy_ext.format, ExtensionFormat::Legacy);
        assert_eq!(legacy_ext.profile_id, 0x1234);
        assert_eq!(legacy_ext.len(), 1);
        assert_eq!(legacy_ext.elements[0].id, 0);
        assert_eq!(legacy_ext.elements[0].data, Bytes::from(raw_data));
    }
    
    #[test]
    fn test_invalid_extensions() {
        let mut one_byte = RtpHeaderExtensions::new_one_byte();
        
        // Test ID out of range (15 > 14)
        assert!(one_byte.add_extension(15, vec![1, 2, 3]).is_err());
        
        // Test data too long
        let long_data: Vec<u8> = (0..20).collect(); // 20 bytes > 16
        assert!(one_byte.add_extension(1, long_data).is_err());
        
        // Test valid extensions
        assert!(one_byte.add_extension(1, vec![1, 2, 3]).is_ok());
        assert!(one_byte.add_extension(14, vec![4, 5, 6]).is_ok());
        
        // Two-byte tests
        let mut two_byte = RtpHeaderExtensions::new_two_byte();
        
        // Test ID = 0 (invalid)
        assert!(two_byte.add_extension(0, vec![1, 2, 3]).is_err());
        
        // Test data too long - create a vector that's longer than 255 bytes
        let very_long_data = vec![0; 256]; // Just over 255 bytes
        assert!(two_byte.add_extension(1, very_long_data).is_err());
        
        // Test valid extensions
        assert!(two_byte.add_extension(1, vec![1, 2, 3]).is_ok());
        assert!(two_byte.add_extension(255, vec![4, 5, 6]).is_ok());
    }
    
    #[test]
    fn test_remove_extension() {
        let mut ext = RtpHeaderExtensions::new_one_byte();
        
        // Add extensions
        ext.add_extension(1, vec![1, 2, 3]).unwrap();
        ext.add_extension(2, vec![4, 5, 6]).unwrap();
        
        // Initial validation
        assert_eq!(ext.len(), 2);
        assert!(ext.get_extension(1).is_some());
        assert!(ext.get_extension(2).is_some());
        
        // Remove one extension
        let removed = ext.remove_extension(1);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().data, Bytes::from_static(&[1, 2, 3]));
        
        // Verify state after removal
        assert_eq!(ext.len(), 1);
        assert!(ext.get_extension(1).is_none());
        assert!(ext.get_extension(2).is_some());
        
        // Remove non-existent extension
        assert!(ext.remove_extension(3).is_none());
        
        // Clear all
        ext.clear();
        assert_eq!(ext.len(), 0);
        assert!(ext.is_empty());
    }
    
    #[test]
    fn test_one_byte_serialization_alignment() {
        let mut ext = RtpHeaderExtensions::new_one_byte();
        
        // Add extension with 1 byte of data - should get padded to 4-byte alignment
        ext.add_extension(1, vec![42]).unwrap();
        
        let serialized = ext.serialize().unwrap();
        
        // Should have: 1 byte header + 1 byte data + 1 byte end marker (0x00) + 1 byte padding = 4 bytes
        assert_eq!(serialized.len(), 4);
        assert_eq!(serialized.len() % 4, 0);
    }
    
    #[test]
    fn test_two_byte_serialization_alignment() {
        let mut ext = RtpHeaderExtensions::new_two_byte();
        
        // Add extension with 1 byte of data - should get padded to 4-byte alignment
        ext.add_extension(1, vec![42]).unwrap();
        
        let serialized = ext.serialize().unwrap();
        
        // Should have: 1 byte ID + 1 byte length + 1 byte data + 1 byte end marker (0x00) = 4 bytes
        assert_eq!(serialized.len(), 4);
        assert_eq!(serialized.len() % 4, 0);
    }
} 