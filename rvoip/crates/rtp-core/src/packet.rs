use bytes::{Buf, BufMut, Bytes, BytesMut};
use bitvec::prelude::*;
use std::fmt;
use tracing::debug;

use crate::error::Error;
use crate::{Result, RtpCsrc, RtpSequenceNumber, RtpSsrc, RtpTimestamp};

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
    
    /// Extension header ID
    pub extension_id: Option<u16>,
    
    /// Extension data
    pub extension_data: Option<Bytes>,
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
            extension_id: None,
            extension_data: None,
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
            extension_id: None,
            extension_data: None,
        }
    }

    /// Get the size of the header in bytes
    pub fn size(&self) -> usize {
        let mut size = RTP_MIN_HEADER_SIZE;
        
        // Add CSRC list size
        size += self.csrc.len() * 4;
        
        // Add extension header size if present
        if self.extension {
            if let Some(ext_data) = &self.extension_data {
                // 4 bytes for extension header plus extension data
                size += 4 + ext_data.len();
            } else {
                // Extension flag is set but no data
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
        let (extension_id, extension_data) = if extension {
            debug!("Parsing extension header");
            
            // Extension header requires at least 4 bytes (2 for ID, 2 for length)
            if buf.remaining() < 4 {
                debug!("Buffer too small for extension header: need 4 but have {}", buf.remaining());
                return Err(Error::BufferTooSmall {
                    required: 4,
                    available: buf.remaining(),
                });
            }
            
            let ext_id = buf.get_u16();
            let ext_length_words = buf.get_u16() as usize;
            debug!("Extension ID: {}, length: {} words", ext_id, ext_length_words);
            
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
                
                (Some(ext_id), Some(ext_data.freeze()))
            } else {
                // Extension with zero length
                debug!("Extension has zero length");
                (Some(ext_id), Some(Bytes::new()))
            }
        } else {
            debug!("No extension header");
            (None, None)
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
            extension_id,
            extension_data,
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
        let (extension_id, extension_data) = if extension {
            debug!("Parsing extension header");
            
            let ext_offset = bytes_consumed;
            
            // Extension header requires at least 4 bytes (2 for ID, 2 for length)
            if data.len() < ext_offset + 4 {
                debug!("Buffer too small for extension header: need {} but have {}", 
                     ext_offset + 4, data.len());
                return Err(Error::BufferTooSmall {
                    required: ext_offset + 4,
                    available: data.len(),
                });
            }
            
            // Extract extension ID and length
            let ext_id = ((data[ext_offset] as u16) << 8) | (data[ext_offset + 1] as u16);
            let ext_length_words = ((data[ext_offset + 2] as u16) << 8) | (data[ext_offset + 3] as u16);
            debug!("Extension ID: {}, length: {} words", ext_id, ext_length_words);
            
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
                
                // Copy the extension data
                let ext_data = Bytes::copy_from_slice(&data[bytes_consumed..bytes_consumed + ext_length_bytes]);
                debug!("Read {} bytes of extension data", ext_length_bytes);
                
                bytes_consumed += ext_length_bytes;
                
                (Some(ext_id), Some(ext_data))
            } else {
                // Extension with zero length
                debug!("Extension has zero length");
                bytes_consumed += 0;
                (Some(ext_id), Some(Bytes::new()))
            }
        } else {
            debug!("No extension header");
            (None, None)
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
            extension_id,
            extension_data,
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
            if let (Some(ext_id), Some(ext_data)) = (self.extension_id, &self.extension_data) {
                // Extension ID (16 bits)
                buf.put_u16(ext_id);
                
                // Calculate length in 32-bit words (rounded up)
                let ext_length = (ext_data.len() + 3) / 4;
                buf.put_u16(ext_length as u16);
                
                // Extension data
                buf.put_slice(ext_data);
                
                // Padding to 32-bit boundary if needed
                let padding_bytes = (4 - (ext_data.len() % 4)) % 4;
                for _ in 0..padding_bytes {
                    buf.put_u8(0);
                }
            } else {
                return Err(Error::InvalidParameter(
                    "Extension flag is set but extension data is missing".to_string()
                ));
            }
        }

        Ok(())
    }
}

/// RTP packet implementation
#[derive(Clone)]
pub struct RtpPacket {
    /// RTP header
    pub header: RtpHeader,
    
    /// Payload data
    pub payload: Bytes,
}

impl RtpPacket {
    /// Create a new RTP packet
    pub fn new(header: RtpHeader, payload: Bytes) -> Self {
        Self { header, payload }
    }

    /// Create a new RTP packet with basic parameters
    pub fn new_with_payload(
        payload_type: u8,
        sequence_number: RtpSequenceNumber,
        timestamp: RtpTimestamp,
        ssrc: RtpSsrc,
        payload: Bytes,
    ) -> Self {
        let header = RtpHeader::new(payload_type, sequence_number, timestamp, ssrc);
        Self { header, payload }
    }

    /// Get the total size of the packet in bytes
    pub fn size(&self) -> usize {
        self.header.size() + self.payload.len()
    }

    /// Parse an RTP packet from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        // Check if we have enough data for the minimum header size
        if data.len() < RTP_MIN_HEADER_SIZE {
            return Err(Error::BufferTooSmall {
                required: RTP_MIN_HEADER_SIZE,
                available: data.len(),
            });
        }
        
        // Debug: log raw data for detailed troubleshooting
        debug!("Parsing RTP packet data: [{}] ({} bytes)", hex_dump(&data[..std::cmp::min(32, data.len())]), data.len());
        
        // Log first 2 bytes for quick header check
        if data.len() >= 2 {
            let first_byte = data[0];
            let second_byte = data[1];
            
            // Extract version, padding, extension and CSRC count from first byte
            let version = (first_byte >> 6) & 0x03;
            let padding = ((first_byte >> 2) & 0x01) == 1;
            let extension = ((first_byte >> 3) & 0x01) == 1;
            let cc = first_byte & 0x0F;
            
            // Extract marker and payload type from second byte
            let marker = ((second_byte >> 7) & 0x01) == 1;
            let payload_type = second_byte & 0x7F;
            
            debug!("Header quick check: version={}, padding={}, ext={}, cc={}, marker={}, pt={}", 
                  version, padding, extension, cc, marker, payload_type);
        }
        
        // Parse the header without consuming the buffer
        match RtpHeader::parse_without_consuming(data) {
            Ok((header, header_size)) => {
                debug!("Successfully parsed RTP header: header size = {}", header_size);

                // Calculate the payload size
                let payload_len = data.len() - header_size;
                debug!("Calculated payload length: {} bytes (data: {} - header: {})",
                     payload_len, data.len(), header_size);
                
                // Extract payload
                let payload = if payload_len > 0 {
                    let payload_data = &data[header_size..];
                    debug!("Extracting payload of {} bytes", payload_data.len());
                    
                    // Calculate padding bytes if padding flag is set
                    let padding_bytes = if header.padding && !payload_data.is_empty() {
                        // The last byte of the packet indicates the padding length
                        let padding = payload_data[payload_data.len() - 1] as usize;
                        
                        // Validate padding value
                        if padding == 0 {
                            debug!("Padding flag set but padding value is 0, ignoring padding");
                            0
                        } else if padding > payload_data.len() {
                            return Err(Error::InvalidPacket(format!(
                                "Invalid padding value: {} exceeds payload length: {}",
                                padding, payload_data.len()
                            )));
                        } else {
                            debug!("Padding detected: {} bytes", padding);
                            padding
                        }
                    } else {
                        if header.padding {
                            debug!("Padding flag set but payload is empty, ignoring padding");
                        }
                        0
                    };
                    
                    // Extract payload without padding
                    let actual_payload_len = payload_data.len().saturating_sub(padding_bytes);
                    
                    if actual_payload_len > 0 {
                        debug!("Final payload length: {} bytes", actual_payload_len);
                        Bytes::copy_from_slice(&payload_data[..actual_payload_len])
                    } else {
                        debug!("Empty payload after padding removal");
                        Bytes::new()
                    }
                } else {
                    debug!("No payload data (only header)");
                    Bytes::new()
                };
                
                Ok(Self { header, payload })
            },
            Err(e) => {
                debug!("Failed to parse RTP header: {}", e);
                Err(e)
            }
        }
    }

    /// Serialize the packet to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let total_size = self.size();
        let mut buf = BytesMut::with_capacity(total_size);
        
        // Serialize header
        self.header.serialize(&mut buf)?;
        
        // Add payload
        buf.put_slice(&self.payload);
        
        // Add padding if needed
        if self.header.padding {
            let padding_bytes = *self.payload.as_ref().last().unwrap_or(&0) as usize;
            for _ in 0..padding_bytes - 1 {
                buf.put_u8(0);
            }
            buf.put_u8(padding_bytes as u8);
        }
        
        Ok(buf.freeze())
    }
}

impl fmt::Debug for RtpPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RtpPacket")
            .field("header", &self.header)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

/// Utility function to generate a hex dump of data for debugging
pub fn hex_dump(data: &[u8]) -> String {
    let mut output = String::new();
    for (i, byte) in data.iter().enumerate() {
        if i > 0 {
            output.push(' ');
        }
        output.push_str(&format!("{:02x}", byte));
    }
    output
} 