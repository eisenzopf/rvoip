use bytes::{Buf, BufMut, Bytes, BytesMut};
use std::fmt;
use tracing::debug;

use crate::error::Error;
use crate::{Result, RtpCsrc, RtpSequenceNumber, RtpSsrc, RtpTimestamp};
use super::extension::{ExtensionFormat, RtpHeaderExtensions};

/// RTP protocol version (always 2 in practice)
pub const RTP_VERSION: u8 = 2;

/// Padding flag bit position (5th bit, 0-indexed)
pub const RTP_PADDING_FLAG: usize = 5;

/// Extension flag bit position (4th bit, 0-indexed)
pub const RTP_EXTENSION_FLAG: usize = 4;

/// CSRC count position (4 bits starting at the 7th position, 0-indexed)
pub const RTP_CC_OFFSET: usize = 4;
pub const RTP_CC_MASK: u8 = 0x0F;

/// Marker bit position in the second byte
pub const RTP_MARKER_FLAG: usize = 7;

/// Payload type position in the second byte (7 bits)
pub const RTP_PT_OFFSET: usize = 1;

/// Minimum header size (without CSRC or extensions)
pub const RTP_MIN_HEADER_SIZE: usize = 12;

/// RTP header implementation according to RFC 3550
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpHeader {
    /// RTP version (should be 2)
    pub version: u8,
    
    /// Padding flag
    pub padding: bool,
    
    /// Extension flag
    pub extension: bool,
    
    /// CSRC count (number of contributing sources)
    pub cc: u8,
    
    /// Marker bit
    pub marker: bool,
    
    /// Payload type
    pub payload_type: u8,
    
    /// Sequence number
    pub sequence_number: RtpSequenceNumber,
    
    /// Timestamp
    pub timestamp: RtpTimestamp,
    
    /// Synchronization source identifier
    pub ssrc: RtpSsrc,
    
    /// Contributing source identifiers
    pub csrc: Vec<RtpCsrc>,
    
    /// Header extensions (RFC 8285)
    pub extensions: Option<RtpHeaderExtensions>,
}

impl Default for RtpHeader {
    fn default() -> Self {
        Self {
            version: RTP_VERSION,
            padding: false,
            extension: false,
            cc: 0,
            marker: false,
            payload_type: 0,
            sequence_number: 0,
            timestamp: 0,
            ssrc: 0,
            csrc: Vec::new(),
            extensions: None,
        }
    }
}

impl RtpHeader {
    /// Create a new RTP header with default values
    pub fn new(payload_type: u8, sequence_number: RtpSequenceNumber, 
               timestamp: RtpTimestamp, ssrc: RtpSsrc) -> Self {
        Self {
            version: RTP_VERSION,
            padding: false,
            extension: false,
            cc: 0,
            marker: false,
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            csrc: Vec::new(),
            extensions: None,
        }
    }

    /// Get the size of the header in bytes
    pub fn size(&self) -> usize {
        let mut size = RTP_MIN_HEADER_SIZE;
        
        // Add CSRC list size
        size += self.csrc.len() * 4;
        
        // Add extension header size if present
        if self.extension {
            if let Some(extensions) = &self.extensions {
                // 4 bytes for profile ID and length + extension data size
                size += 4 + extensions.size();
            } else {
                // Extension flag is set but no data - reserve minimal 4 bytes
                size += 4;
            }
        }
        
        size
    }

    /// Parse an RTP header from bytes
    pub fn parse(buf: &mut impl Buf) -> Result<Self> {
        debug!("Starting RTP header parse with {} bytes available", buf.remaining());
        
        // Check if we have enough data for the minimum header
        if buf.remaining() < RTP_MIN_HEADER_SIZE {
            debug!("Buffer too small: need {} but have {}", RTP_MIN_HEADER_SIZE, buf.remaining());
            return Err(Error::BufferTooSmall {
                required: RTP_MIN_HEADER_SIZE,
                available: buf.remaining(),
            });
        }

        // First byte: version (2 bits), padding (1 bit), extension (1 bit), CSRC count (4 bits)
        let first_byte = buf.get_u8();
        debug!("First byte: 0x{:02x}", first_byte);
        
        // Extract version (2 bits)
        let version = (first_byte >> 6) & 0x03;
        debug!("Version: {}", version);
        
        if version != RTP_VERSION {
            debug!("Invalid RTP version: {} (expected {})", version, RTP_VERSION);
            return Err(Error::InvalidPacket(format!("Invalid RTP version: {}", version)));
        }
        
        // Extract flags and CSRC count
        let padding = ((first_byte >> 5) & 0x01) == 1;
        let extension = ((first_byte >> 4) & 0x01) == 1;
        let cc = first_byte & 0x0F;
        debug!("Flags: padding={}, extension={}, cc={}", padding, extension, cc);

        // Second byte: marker (1 bit), payload type (7 bits)
        let second_byte = buf.get_u8();
        debug!("Second byte: 0x{:02x}", second_byte);
        
        let marker = ((second_byte >> 7) & 0x01) == 1;
        let payload_type = second_byte & 0x7F;
        debug!("Marker: {}, payload_type: {}", marker, payload_type);

        // Check if we have enough remaining bytes for sequence, timestamp, and SSRC
        if buf.remaining() < 8 {
            debug!("Buffer too small for seq/ts/ssrc: need 8 but have {}", buf.remaining());
            return Err(Error::BufferTooSmall {
                required: 8,
                available: buf.remaining(),
            });
        }

        // Sequence number (16 bits)
        let sequence_number = buf.get_u16();
        debug!("Sequence number: {}", sequence_number);
        
        // Timestamp (32 bits)
        let timestamp = buf.get_u32();
        debug!("Timestamp: {}", timestamp);
        
        // SSRC (32 bits)
        let ssrc = buf.get_u32();
        debug!("SSRC: {}", ssrc);

        // Parse CSRC list if present
        let mut csrc = Vec::with_capacity(cc as usize);
        debug!("Parsing CSRC list with {} entries", cc);
        for i in 0..cc {
            // Make sure we have enough data for each CSRC (4 bytes)
            if buf.remaining() < 4 {
                debug!("Buffer too small for CSRC {}: need 4 but have {}", i, buf.remaining());
                return Err(Error::BufferTooSmall {
                    required: 4,
                    available: buf.remaining(),
                });
            }
            let csrc_value = buf.get_u32();
            debug!("CSRC {}: 0x{:08x}", i, csrc_value);
            csrc.push(csrc_value);
        }

        // Parse extension header if present
        let extensions = if extension {
            debug!("Parsing extension header");
            
            // Extension header requires at least 4 bytes (2 for profile ID, 2 for length)
            if buf.remaining() < 4 {
                debug!("Buffer too small for extension header: need 4 but have {}", buf.remaining());
                return Err(Error::BufferTooSmall {
                    required: 4,
                    available: buf.remaining(),
                });
            }
            
            let profile_id = buf.get_u16();
            let ext_length_words = buf.get_u16() as usize;
            debug!("Extension profile ID: 0x{:04x}, length: {} words", profile_id, ext_length_words);
            
            // Extension length is in 32-bit words (4 bytes each)
            let ext_length_bytes = ext_length_words * 4;
            debug!("Extension length in bytes: {}", ext_length_bytes);
            
            if ext_length_bytes > 0 {
                // Validate we have enough data for the extension
                if buf.remaining() < ext_length_bytes {
                    debug!("Buffer too small for extension data: need {} but have {}", 
                          ext_length_bytes, buf.remaining());
                    return Err(Error::BufferTooSmall {
                        required: ext_length_bytes,
                        available: buf.remaining(),
                    });
                }
                
                // Copy the extension data
                let mut ext_data = BytesMut::with_capacity(ext_length_bytes);
                for _ in 0..ext_length_bytes {
                    let byte = buf.get_u8();
                    ext_data.put_u8(byte);
                }
                debug!("Read {} bytes of extension data", ext_length_bytes);
                
                // Parse the extension data based on the profile ID
                Some(RtpHeaderExtensions::from_extension_data(profile_id, &ext_data)?)
            } else {
                // Extension with zero length
                debug!("Extension has zero length");
                Some(RtpHeaderExtensions::from_extension_data(profile_id, &[])?)
            }
        } else {
            debug!("No extension header");
            None
        };

        debug!("RTP header parsing completed successfully");
        Ok(Self {
            version,
            padding,
            extension,
            cc,
            marker,
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            csrc,
            extensions,
        })
    }

    /// Parse an RTP header from bytes without consuming the buffer
    /// Returns the header and the number of bytes consumed
    pub fn parse_without_consuming(data: &[u8]) -> Result<(Self, usize)> {
        debug!("Starting RTP header parse_without_consuming with {} bytes", data.len());
        
        // Check if we have enough data for the minimum header
        if data.len() < RTP_MIN_HEADER_SIZE {
            debug!("Buffer too small: need {} but have {}", RTP_MIN_HEADER_SIZE, data.len());
            return Err(Error::BufferTooSmall {
                required: RTP_MIN_HEADER_SIZE,
                available: data.len(),
            });
        }

        // First byte: version (2 bits), padding (1 bit), extension (1 bit), CSRC count (4 bits)
        // According to RFC 3550, the bit layout is:
        // 0                   1                   2                   3
        // 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
        // +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
        // |V=2|P|X|  CC   |M|     PT      |       sequence number         |
        let first_byte = data[0];
        debug!("First byte: 0x{:02x}", first_byte);
        
        // Use bitshifts to extract the bits directly
        let version = (first_byte >> 6) & 0x03;    // Version: bits 0-1
        debug!("Version: {}", version);
        
        if version != RTP_VERSION {
            debug!("Invalid RTP version: {} (expected {})", version, RTP_VERSION);
            return Err(Error::InvalidPacket(format!("Invalid RTP version: {}", version)));
        }
        
        // Extract flags and CSRC count
        let padding = ((first_byte >> 5) & 0x01) == 1;   // Padding: bit 2
        let extension = ((first_byte >> 4) & 0x01) == 1; // Extension: bit 3
        let cc = first_byte & 0x0F;                      // CSRC count: bits 4-7
        debug!("Flags: padding={}, extension={}, cc={}", padding, extension, cc);

        // Second byte: marker (1 bit), payload type (7 bits)
        let second_byte = data[1];
        debug!("Second byte: 0x{:02x}", second_byte);
        
        let marker = ((second_byte >> 7) & 0x01) == 1;   // Marker: bit 0
        let payload_type = second_byte & 0x7F;           // Payload type: bits 1-7
        debug!("Marker: {}, payload_type: {}", marker, payload_type);

        // Sequence number (16 bits) - big endian
        let sequence_number = ((data[2] as u16) << 8) | (data[3] as u16);
        debug!("Sequence number: {}", sequence_number);
        
        // Timestamp (32 bits) - big endian
        let timestamp = ((data[4] as u32) << 24) | 
                        ((data[5] as u32) << 16) | 
                        ((data[6] as u32) << 8) | 
                        (data[7] as u32);
        debug!("Timestamp: {}", timestamp);
        
        // SSRC (32 bits) - big endian
        let ssrc = ((data[8] as u32) << 24) | 
                   ((data[9] as u32) << 16) | 
                   ((data[10] as u32) << 8) | 
                   (data[11] as u32);
        debug!("SSRC: {}", ssrc);

        // Calculate total size including header extension and CSRC
        let mut bytes_consumed = RTP_MIN_HEADER_SIZE;
        
        // Parse CSRC list if present
        let mut csrc = Vec::with_capacity(cc as usize);
        debug!("Parsing CSRC list with {} entries", cc);
        for i in 0..cc {
            let csrc_offset = RTP_MIN_HEADER_SIZE + (i as usize) * 4;
            
            // Check if we have enough data for this CSRC
            if data.len() < csrc_offset + 4 {
                debug!("Buffer too small for CSRC {}: need {} but have {}", 
                       i, csrc_offset + 4, data.len());
                return Err(Error::BufferTooSmall {
                    required: csrc_offset + 4,
                    available: data.len(),
                });
            }
            
            // Extract CSRC
            let csrc_value = ((data[csrc_offset] as u32) << 24) | 
                             ((data[csrc_offset + 1] as u32) << 16) | 
                             ((data[csrc_offset + 2] as u32) << 8) | 
                             (data[csrc_offset + 3] as u32);
            debug!("CSRC {}: 0x{:08x} from offset {}", i, csrc_value, csrc_offset);
            csrc.push(csrc_value);
            
            bytes_consumed += 4;
        }

        // Parse extension header if present
        let extensions = if extension {
            debug!("Parsing extension header");
            
            let ext_offset = bytes_consumed;
            
            // Extension header requires at least 4 bytes (2 for profile ID, 2 for length)
            if data.len() < ext_offset + 4 {
                debug!("Buffer too small for extension header: need {} but have {}", 
                     ext_offset + 4, data.len());
                return Err(Error::BufferTooSmall {
                    required: ext_offset + 4,
                    available: data.len(),
                });
            }
            
            // Extract extension profile ID and length
            let profile_id = ((data[ext_offset] as u16) << 8) | (data[ext_offset + 1] as u16);
            let ext_length_words = ((data[ext_offset + 2] as u16) << 8) | (data[ext_offset + 3] as u16);
            debug!("Extension profile ID: 0x{:04x}, length: {} words", profile_id, ext_length_words);
            
            // Extension length is in 32-bit words (4 bytes each)
            let ext_length_bytes = ext_length_words as usize * 4;
            debug!("Extension length in bytes: {}", ext_length_bytes);
            
            bytes_consumed += 4; // Add the 4 bytes for ext header
            
            if ext_length_bytes > 0 {
                // Validate we have enough data for the extension
                if data.len() < bytes_consumed + ext_length_bytes {
                    debug!("Buffer too small for extension data: need {} but have {}", 
                         bytes_consumed + ext_length_bytes, data.len());
                    return Err(Error::BufferTooSmall {
                        required: bytes_consumed + ext_length_bytes,
                        available: data.len(),
                    });
                }
                
                // Extract the extension data
                let ext_data = &data[bytes_consumed..bytes_consumed + ext_length_bytes];
                debug!("Read {} bytes of extension data", ext_length_bytes);
                
                bytes_consumed += ext_length_bytes;
                
                // Parse the extension data based on the profile ID
                Some(RtpHeaderExtensions::from_extension_data(profile_id, ext_data)?)
            } else {
                // Extension with zero length
                debug!("Extension has zero length");
                Some(RtpHeaderExtensions::from_extension_data(profile_id, &[])?)
            }
        } else {
            debug!("No extension header");
            None
        };

        debug!("RTP header parsing completed successfully, consumed {} bytes", bytes_consumed);
        Ok((Self {
            version,
            padding,
            extension,
            cc,
            marker,
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            csrc,
            extensions,
        }, bytes_consumed))
    }

    /// Serialize the header to bytes
    pub fn serialize(&self, buf: &mut BytesMut) -> Result<()> {
        let required_size = self.size();
        if buf.remaining_mut() < required_size {
            buf.reserve(required_size - buf.remaining_mut());
        }

        // First byte: version (2 bits), padding (1 bit), extension (1 bit), CSRC count (4 bits)
        // According to RFC 3550, the bit layout is:
        // 0                   1                   2                   3
        // 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
        // +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
        // |V=2|P|X|  CC   |M|     PT      |       sequence number         |
        let mut first_byte = 0u8;
        first_byte |= (self.version & 0x03) << 6;  // Version: bits 0-1
        if self.padding {
            first_byte |= 1 << 5;                 // Padding: bit 2
        }
        if self.extension {
            first_byte |= 1 << 4;                 // Extension: bit 3
        }
        first_byte |= self.cc & 0x0F;             // CSRC count: bits 4-7
        
        debug!("Serializing first byte: 0x{:02x} (V={}, P={}, X={}, CC={})",
               first_byte, self.version, self.padding, self.extension, self.cc);
        
        buf.put_u8(first_byte);

        // Second byte: marker (1 bit), payload type (7 bits)
        let mut second_byte = 0u8;
        if self.marker {
            second_byte |= 1 << 7;                // Marker: bit 0
        }
        second_byte |= self.payload_type & 0x7F;  // Payload type: bits 1-7
        buf.put_u8(second_byte);

        // Sequence number (16 bits)
        buf.put_u16(self.sequence_number);
        
        // Timestamp (32 bits)
        buf.put_u32(self.timestamp);
        
        // SSRC (32 bits)
        buf.put_u32(self.ssrc);

        // CSRC list
        if self.cc as usize != self.csrc.len() {
            return Err(Error::InvalidParameter(format!(
                "CSRC count ({}) does not match CSRC list length ({})",
                self.cc, self.csrc.len()
            )));
        }
        
        for csrc in &self.csrc {
            buf.put_u32(*csrc);
        }

        // Extension header if present
        if self.extension {
            if let Some(extensions) = &self.extensions {
                // Put extension profile ID
                buf.put_u16(extensions.profile_id);
                
                // Serialize the extension data
                let ext_data = extensions.serialize()?;
                
                // Calculate length in 32-bit words - already padded to 4 bytes
                let ext_length_words = ext_data.len() / 4;
                buf.put_u16(ext_length_words as u16);
                
                // Put extension data (already padded to 32-bit boundary)
                buf.put_slice(&ext_data);
            } else {
                // Extension flag is set but no data
                // Use a default profile ID (one-byte format)
                buf.put_u16(super::extension::ONE_BYTE_EXTENSION_ID);
                
                // Zero length
                buf.put_u16(0);
            }
        }

        Ok(())
    }

    /// Add a CSRC (Contributing Source) identifier to the header
    /// This will automatically update the cc count field
    pub fn add_csrc(&mut self, csrc: RtpCsrc) {
        self.csrc.push(csrc);
        self.cc = self.csrc.len() as u8;
    }
    
    /// Add multiple CSRC identifiers at once
    pub fn add_csrcs(&mut self, csrcs: &[RtpCsrc]) {
        self.csrc.extend_from_slice(csrcs);
        self.cc = self.csrc.len() as u8;
    }
    
    /// Get a CSRC by index
    pub fn get_csrc(&self, index: usize) -> Option<RtpCsrc> {
        self.csrc.get(index).copied()
    }
    
    /// Remove a CSRC by value
    /// Returns true if the CSRC was found and removed
    pub fn remove_csrc(&mut self, csrc: RtpCsrc) -> bool {
        if let Some(index) = self.csrc.iter().position(|&c| c == csrc) {
            self.csrc.remove(index);
            self.cc = self.csrc.len() as u8;
            true
        } else {
            false
        }
    }
    
    /// Remove a CSRC by index
    /// Returns the removed CSRC value if the index was valid
    pub fn remove_csrc_at(&mut self, index: usize) -> Option<RtpCsrc> {
        if index < self.csrc.len() {
            let csrc = self.csrc.remove(index);
            self.cc = self.csrc.len() as u8;
            Some(csrc)
        } else {
            None
        }
    }
    
    /// Clear all CSRC identifiers
    pub fn clear_csrcs(&mut self) {
        self.csrc.clear();
        self.cc = 0;
    }
    
    /// Check if a specific CSRC is present in the list
    pub fn has_csrc(&self, csrc: RtpCsrc) -> bool {
        self.csrc.contains(&csrc)
    }
    
    /// Add a header extension element
    /// This will automatically create and configure the extensions container if needed
    pub fn add_extension(&mut self, id: u8, data: impl Into<Bytes>) -> Result<()> {
        // Create extensions container if it doesn't exist
        if self.extensions.is_none() {
            self.extensions = Some(RtpHeaderExtensions::new_one_byte());
        }
        
        // Add the extension element
        if let Some(extensions) = &mut self.extensions {
            extensions.add_extension(id, data)?;
            
            // Set the extension flag
            self.extension = true;
        }
        
        Ok(())
    }
    
    /// Get a header extension by ID
    pub fn get_extension(&self, id: u8) -> Option<&Bytes> {
        if let Some(extensions) = &self.extensions {
            if let Some(element) = extensions.get_extension(id) {
                return Some(&element.data);
            }
        }
        None
    }
    
    /// Remove a header extension by ID
    pub fn remove_extension(&mut self, id: u8) -> Option<Bytes> {
        if let Some(extensions) = &mut self.extensions {
            if let Some(element) = extensions.remove_extension(id) {
                // If no extensions are left, clear the extension flag
                if extensions.is_empty() {
                    self.extension = false;
                }
                return Some(element.data);
            }
        }
        None
    }
    
    /// Get the extension format
    pub fn extension_format(&self) -> Option<ExtensionFormat> {
        self.extensions.as_ref().map(|ext| ext.format)
    }
    
    /// Set the extension format
    pub fn set_extension_format(&mut self, format: ExtensionFormat) -> Result<()> {
        if self.extensions.is_none() {
            // Create a new extensions container with the specified format
            self.extensions = match format {
                ExtensionFormat::OneByte => Some(RtpHeaderExtensions::new_one_byte()),
                ExtensionFormat::TwoByte => Some(RtpHeaderExtensions::new_two_byte()),
                ExtensionFormat::Legacy => Some(RtpHeaderExtensions::new_legacy(0)),
            };
            return Ok(());
        }
        
        // If we have an existing extensions container with a different format,
        // we need to convert it
        let extensions = self.extensions.as_mut().unwrap();
        if extensions.format == format {
            return Ok(());
        }
        
        // Create a new extensions container with the new format
        let mut new_extensions = match format {
            ExtensionFormat::OneByte => RtpHeaderExtensions::new_one_byte(),
            ExtensionFormat::TwoByte => RtpHeaderExtensions::new_two_byte(),
            ExtensionFormat::Legacy => RtpHeaderExtensions::new_legacy(0),
        };
        
        // Copy all elements to the new container
        for element in &extensions.elements {
            new_extensions.add_extension(element.id, element.data.clone())?;
        }
        
        // Replace the old container with the new one
        self.extensions = Some(new_extensions);
        
        Ok(())
    }
    
    /// Clear all extensions
    pub fn clear_extensions(&mut self) {
        if let Some(extensions) = &mut self.extensions {
            extensions.clear();
            // Clear the extension flag
            self.extension = false;
        }
    }
}

/// Utility function to dump binary data as hex
pub fn hex_dump(data: &[u8]) -> String {
    let mut result = String::new();
    for (i, byte) in data.iter().enumerate() {
        if i > 0 {
            result.push(' ');
        }
        result.push_str(&format!("{:02x}", byte));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber;
    
    fn init_logging() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .try_init();
    }
    
    #[test]
    fn test_header_create() {
        let header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        assert_eq!(header.version, 2);
        assert_eq!(header.padding, false);
        assert_eq!(header.extension, false);
        assert_eq!(header.cc, 0);
        assert_eq!(header.marker, false);
        assert_eq!(header.payload_type, 96);
        assert_eq!(header.sequence_number, 1000);
        assert_eq!(header.timestamp, 12345);
        assert_eq!(header.ssrc, 0xabcdef01);
        assert!(header.csrc.is_empty());
        assert_eq!(header.extensions, None);
    }
    
    #[test]
    fn test_header_size() {
        // Basic header
        let header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        assert_eq!(header.size(), RTP_MIN_HEADER_SIZE);
        
        // Header with CSRC
        let mut header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        header.csrc = vec![0x11111111, 0x22222222];
        header.cc = 2;
        assert_eq!(header.size(), RTP_MIN_HEADER_SIZE + 8); // 8 = 2 CSRC * 4 bytes
        
        // Header with extension
        let mut header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        header.extension = true;
        header.extensions = Some(RtpHeaderExtensions::new_one_byte());
        assert_eq!(header.size(), RTP_MIN_HEADER_SIZE + 4); // 4 = 4 (ext header)
    }
    
    #[test]
    fn test_header_serialize_parse() {
        init_logging();
        
        let original = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        
        // Serialize
        let mut buf = BytesMut::with_capacity(12);
        original.serialize(&mut buf).unwrap();
        
        // Parse
        let mut reader = buf.freeze();
        let parsed = RtpHeader::parse(&mut reader).unwrap();
        
        // Verify
        assert_eq!(parsed.version, original.version);
        assert_eq!(parsed.padding, original.padding);
        assert_eq!(parsed.extension, original.extension);
        assert_eq!(parsed.cc, original.cc);
        assert_eq!(parsed.marker, original.marker);
        assert_eq!(parsed.payload_type, original.payload_type);
        assert_eq!(parsed.sequence_number, original.sequence_number);
        assert_eq!(parsed.timestamp, original.timestamp);
        assert_eq!(parsed.ssrc, original.ssrc);
        assert_eq!(parsed.csrc, original.csrc);
        assert_eq!(parsed.extensions, original.extensions);
    }
    
    #[test]
    fn test_header_with_extension() {
        init_logging();
        
        let mut original = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        original.extension = true;
        original.extensions = Some(RtpHeaderExtensions::new_one_byte());
        
        // Serialize
        let mut buf = BytesMut::with_capacity(12);
        original.serialize(&mut buf).unwrap();
        
        // Parse
        let mut reader = buf.freeze();
        let parsed = RtpHeader::parse(&mut reader).unwrap();
        
        // Verify
        assert_eq!(parsed.extension, true);
        assert!(parsed.extensions.is_some());
    }
    
    #[test]
    fn test_parse_without_consuming() {
        let header_bytes = [
            0x80, 0x60, 0x03, 0xe8, // V=2, P=0, X=0, CC=0, M=0, PT=96, Seq=1000
            0x00, 0x01, 0xe2, 0x40, // Timestamp = 123456
            0xab, 0xcd, 0xef, 0x01, // SSRC = 0xabcdef01
        ];
        
        let (header, consumed) = RtpHeader::parse_without_consuming(&header_bytes).unwrap();
        
        assert_eq!(consumed, 12); // RTP_MIN_HEADER_SIZE
        assert_eq!(header.version, 2);
        assert_eq!(header.padding, false);
        assert_eq!(header.extension, false);
        assert_eq!(header.cc, 0);
        assert_eq!(header.marker, false);
        assert_eq!(header.payload_type, 96);
        assert_eq!(header.sequence_number, 1000);
        assert_eq!(header.timestamp, 123456);
        assert_eq!(header.ssrc, 0xabcdef01);
    }
} 