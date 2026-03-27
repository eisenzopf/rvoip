//! DTLS record layer implementation
//!
//! This module handles the record layer protocol for DTLS.

use bytes::{Bytes, BytesMut, Buf, BufMut};
use std::io::Cursor;
use super::{DtlsVersion, Result};

/// DTLS record content type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ContentType {
    /// ChangeCipherSpec message
    ChangeCipherSpec = 20,
    
    /// Alert message
    Alert = 21,
    
    /// Handshake message
    Handshake = 22,
    
    /// Application data
    ApplicationData = 23,
    
    /// Invalid content type
    Invalid = 255,
}

impl From<u8> for ContentType {
    fn from(value: u8) -> Self {
        match value {
            20 => ContentType::ChangeCipherSpec,
            21 => ContentType::Alert,
            22 => ContentType::Handshake,
            23 => ContentType::ApplicationData,
            _ => ContentType::Invalid,
        }
    }
}

/// DTLS record header
#[derive(Debug, Clone)]
pub struct RecordHeader {
    /// Content type
    pub content_type: ContentType,
    
    /// Protocol version
    pub version: DtlsVersion,
    
    /// Record epoch (incremented on cipher state changes)
    pub epoch: u16,
    
    /// Record sequence number
    pub sequence_number: u64,
    
    /// Record data length
    pub length: u16,
}

impl RecordHeader {
    /// Create a new record header
    pub fn new(
        content_type: ContentType,
        version: DtlsVersion,
        epoch: u16,
        sequence_number: u64,
        length: u16,
    ) -> Self {
        Self {
            content_type,
            version,
            epoch,
            sequence_number,
            length,
        }
    }
    
    /// Serialize the record header to bytes
    pub fn serialize(&self) -> Result<BytesMut> {
        let mut buf = BytesMut::with_capacity(13);
        
        // Content type (1 byte)
        buf.put_u8(self.content_type as u8);
        
        // Protocol version (2 bytes)
        buf.put_u16(self.version as u16);
        
        // Epoch (2 bytes)
        buf.put_u16(self.epoch);
        
        // Sequence number (6 bytes - truncated u48)
        buf.put_u16((self.sequence_number >> 32) as u16);
        buf.put_u32(self.sequence_number as u32);
        
        // Length (2 bytes)
        buf.put_u16(self.length);
        
        Ok(buf)
    }
    
    /// Parse a record header from bytes
    pub fn parse(data: &[u8]) -> Result<(Self, usize)> {
        if data.len() < 13 {
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        // Content type (1 byte)
        let content_type = ContentType::from(cursor.get_u8());
        
        // Protocol version (2 bytes)
        let version_raw = cursor.get_u16();
        let version = match version_raw {
            0xFEFF => DtlsVersion::Dtls10,
            0xFEFD => DtlsVersion::Dtls12,
            _ => {
                return Err(crate::error::Error::InvalidProtocolVersion(
                    format!("Invalid DTLS version: {:#x}", version_raw)
                ));
            }
        };
        
        // Epoch (2 bytes)
        let epoch = cursor.get_u16();
        
        // Sequence number (6 bytes - truncated u48)
        let seq_high = cursor.get_u16() as u64;
        let seq_low = cursor.get_u32() as u64;
        let sequence_number = (seq_high << 32) | seq_low;
        
        // Length (2 bytes)
        let length = cursor.get_u16();
        
        let header = Self {
            content_type,
            version,
            epoch,
            sequence_number,
            length,
        };
        
        Ok((header, 13))
    }
}

/// DTLS record
#[derive(Debug, Clone)]
pub struct Record {
    /// Record header
    pub header: RecordHeader,
    
    /// Record data
    pub data: Bytes,
}

impl Record {
    /// Create a new DTLS record
    pub fn new(
        content_type: ContentType,
        version: DtlsVersion,
        epoch: u16,
        sequence_number: u64,
        data: Bytes,
    ) -> Self {
        let header = RecordHeader::new(
            content_type,
            version,
            epoch,
            sequence_number,
            data.len() as u16,
        );
        
        Self {
            header,
            data,
        }
    }
    
    /// Serialize the record to bytes
    pub fn serialize(&self) -> Result<Bytes> {
        let mut header_buf = self.header.serialize()?;
        let mut buf = BytesMut::with_capacity(header_buf.len() + self.data.len());
        
        buf.extend_from_slice(&header_buf);
        buf.extend_from_slice(&self.data);
        
        Ok(buf.freeze())
    }
    
    /// Parse a DTLS record from bytes
    pub fn parse(data: &[u8]) -> Result<(Self, usize)> {
        tracing::trace!(data_len = data.len(), "Parsing DTLS record");

        if data.len() < 13 {
            tracing::trace!(data_len = data.len(), "Record too short");
            return Err(crate::error::Error::PacketTooShort);
        }
        
        let mut cursor = Cursor::new(data);
        
        // Content type (1 byte)
        let content_type = ContentType::from(cursor.get_u8());
        tracing::trace!(content_type = ?content_type, "Parsed content type");
        
        // Protocol version (2 bytes)
        let version_raw = cursor.get_u16();
        tracing::trace!(version_raw = format_args!("0x{:04x}", version_raw), "Parsed version");
        
        let version = match version_raw {
            0xFEFF => DtlsVersion::Dtls10,
            0xFEFD => DtlsVersion::Dtls12,
            _ => {
                tracing::trace!(version_raw = format_args!("{:#x}", version_raw), "Invalid DTLS version");
                return Err(crate::error::Error::InvalidProtocolVersion(
                    format!("Invalid DTLS version: {:#x}", version_raw)
                ));
            }
        };
        
        // Epoch (2 bytes)
        let epoch = cursor.get_u16();
        tracing::trace!(epoch, "Parsed epoch");
        
        // Sequence number (6 bytes)
        let seq_high = cursor.get_u16() as u64;
        let seq_low = cursor.get_u32() as u64;
        let sequence_number = (seq_high << 32) | seq_low;
        tracing::trace!(sequence_number, "Parsed sequence number");
        
        // Length (2 bytes)
        let length = cursor.get_u16() as usize;
        tracing::trace!(record_length = length, "Parsed record length");
        
        // Check that we have enough data
        if data.len() < 13 + length {
            tracing::trace!(have = data.len(), need = 13 + length, "Record data too short");
            return Err(crate::error::Error::PacketTooShort);
        }
        
        // Create header
        let header = RecordHeader {
            content_type,
            version,
            epoch,
            sequence_number,
            length: length as u16,
        };
        
        // Extract data
        let mut record_data = vec![0u8; length];
        cursor.copy_to_slice(&mut record_data);
        let record_data = Bytes::from(record_data);
        
        // Create record
        let record = Self {
            header,
            data: record_data,
        };
        
        tracing::trace!(content_type = ?content_type, epoch, sequence_number, length, "Successfully parsed DTLS record");
        
        Ok((record, 13 + length))
    }
    
    /// Parse multiple DTLS records from bytes
    pub fn parse_multiple(data: &[u8]) -> Result<Vec<Self>> {
        tracing::trace!(data_len = data.len(), "Parsing multiple DTLS records [raw bytes REDACTED]");
        
        let mut records = Vec::new();
        let mut offset = 0;
        
        while offset < data.len() {
            tracing::trace!(offset, "Parsing record at offset");
            match Self::parse(&data[offset..]) {
                Ok((record, size)) => {
                    tracing::trace!(size, "Successfully parsed record");
                    records.push(record);
                    offset += size;
                }
                Err(e) => {
                    tracing::trace!(error = ?e, "Error parsing record");
                    if records.is_empty() {
                        // Only return error if we couldn't parse any records
                        return Err(e);
                    } else {
                        // If we parsed at least one record, just stop
                        break;
                    }
                }
            }
        }
        
        tracing::trace!(count = records.len(), "Parsed DTLS records");
        Ok(records)
    }
} 