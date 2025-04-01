use bytes::{Buf, BufMut, Bytes, BytesMut};
use bitvec::prelude::*;
use std::fmt;

use crate::error::Error;
use crate::{Result, RtpCsrc, RtpSequenceNumber, RtpSsrc, RtpTimestamp};

/// RTP protocol version (always 2 in practice)
pub const RTP_VERSION: u8 = 2;

/// Padding flag position in the first byte
pub const RTP_PADDING_FLAG: usize = 5;

/// Extension flag position in the first byte
pub const RTP_EXTENSION_FLAG: usize = 4;

/// CSRC count position in the first byte (4 bits)
pub const RTP_CC_OFFSET: usize = 0;

/// Marker bit position in the second byte
pub const RTP_MARKER_FLAG: usize = 7;

/// Payload type position in the second byte (7 bits)
pub const RTP_PT_OFFSET: usize = 0;

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
        if buf.remaining() < RTP_MIN_HEADER_SIZE {
            return Err(Error::BufferTooSmall {
                required: RTP_MIN_HEADER_SIZE,
                available: buf.remaining(),
            });
        }

        // First byte: version (2 bits), padding (1 bit), extension (1 bit), CSRC count (4 bits)
        let first_byte = buf.get_u8();
        let bits = first_byte.view_bits::<Msb0>();
        
        let version = bits[0..2].load::<u8>();
        if version != RTP_VERSION {
            return Err(Error::InvalidPacket(format!("Invalid RTP version: {}", version)));
        }
        
        let padding = bits[RTP_PADDING_FLAG];
        let extension = bits[RTP_EXTENSION_FLAG];
        let cc = bits[RTP_CC_OFFSET..RTP_CC_OFFSET + 4].load::<u8>();

        // Second byte: marker (1 bit), payload type (7 bits)
        let second_byte = buf.get_u8();
        let bits = second_byte.view_bits::<Msb0>();
        
        let marker = bits[RTP_MARKER_FLAG];
        let payload_type = bits[RTP_PT_OFFSET..RTP_PT_OFFSET + 7].load::<u8>();

        // Sequence number (16 bits)
        let sequence_number = buf.get_u16();
        
        // Timestamp (32 bits)
        let timestamp = buf.get_u32();
        
        // SSRC (32 bits)
        let ssrc = buf.get_u32();

        // Parse CSRC list if present
        let mut csrc = Vec::with_capacity(cc as usize);
        for _ in 0..cc {
            if buf.remaining() < 4 {
                return Err(Error::BufferTooSmall {
                    required: 4,
                    available: buf.remaining(),
                });
            }
            csrc.push(buf.get_u32());
        }

        // Parse extension header if present
        let (extension_id, extension_data) = if extension {
            if buf.remaining() < 4 {
                return Err(Error::BufferTooSmall {
                    required: 4,
                    available: buf.remaining(),
                });
            }
            
            let ext_id = buf.get_u16();
            let ext_length = buf.get_u16() as usize * 4; // Length in 32-bit words
            
            if buf.remaining() < ext_length {
                return Err(Error::BufferTooSmall {
                    required: ext_length,
                    available: buf.remaining(),
                });
            }
            
            let mut ext_data = BytesMut::with_capacity(ext_length);
            for _ in 0..ext_length {
                ext_data.put_u8(buf.get_u8());
            }
            
            (Some(ext_id), Some(ext_data.freeze()))
        } else {
            (None, None)
        };

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

    /// Serialize the header to bytes
    pub fn serialize(&self, buf: &mut BytesMut) -> Result<()> {
        let required_size = self.size();
        if buf.remaining_mut() < required_size {
            buf.reserve(required_size - buf.remaining_mut());
        }

        // First byte: version (2 bits), padding (1 bit), extension (1 bit), CSRC count (4 bits)
        let mut first_byte = 0u8;
        first_byte |= (self.version & 0x03) << 6;
        if self.padding {
            first_byte |= 1 << 5;
        }
        if self.extension {
            first_byte |= 1 << 4;
        }
        first_byte |= self.cc & 0x0F;
        buf.put_u8(first_byte);

        // Second byte: marker (1 bit), payload type (7 bits)
        let mut second_byte = 0u8;
        if self.marker {
            second_byte |= 1 << 7;
        }
        second_byte |= self.payload_type & 0x7F;
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
        let mut buf = Bytes::copy_from_slice(data);
        
        // Parse header
        let header = RtpHeader::parse(&mut buf)?;
        
        // Calculate padding bytes if padding flag is set
        let padding_bytes = if header.padding && !buf.is_empty() {
            let padding = *buf.as_ref().last().unwrap_or(&0) as usize;
            if padding > buf.len() {
                return Err(Error::InvalidPacket(format!(
                    "Invalid padding value: {} exceeds remaining bytes: {}",
                    padding, buf.len()
                )));
            }
            padding
        } else {
            0
        };

        // Extract payload (excluding padding)
        let payload_len = buf.len().saturating_sub(padding_bytes);
        let payload = buf.slice(0..payload_len);

        Ok(Self { header, payload })
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