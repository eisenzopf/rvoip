use crate::error::Error;
use crate::{Result, RtpSsrc};
use bytes::{BufMut, BytesMut};

/// RTCP Source Description (SDES) item types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RtcpSdesItemType {
    /// End of SDES item list
    End = 0,

    /// Canonical name (CNAME)
    CName = 1,

    /// User name (NAME)
    Name = 2,

    /// E-mail address (EMAIL)
    Email = 3,

    /// Phone number (PHONE)
    Phone = 4,

    /// Geographic location (LOC)
    Location = 5,

    /// Application or tool name (TOOL)
    Tool = 6,

    /// Notice/status (NOTE)
    Note = 7,

    /// Private extensions (PRIV)
    Private = 8,
}

impl TryFrom<u8> for RtcpSdesItemType {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(RtcpSdesItemType::End),
            1 => Ok(RtcpSdesItemType::CName),
            2 => Ok(RtcpSdesItemType::Name),
            3 => Ok(RtcpSdesItemType::Email),
            4 => Ok(RtcpSdesItemType::Phone),
            5 => Ok(RtcpSdesItemType::Location),
            6 => Ok(RtcpSdesItemType::Tool),
            7 => Ok(RtcpSdesItemType::Note),
            8 => Ok(RtcpSdesItemType::Private),
            _ => Err(Error::RtcpError(format!(
                "Unknown SDES item type: {}",
                value
            ))),
        }
    }
}

/// RTCP Source Description (SDES) item
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSdesItem {
    /// Item type
    pub item_type: RtcpSdesItemType,

    /// Item value
    pub value: String,
}

impl RtcpSdesItem {
    /// Create a new SDES item
    pub fn new(item_type: RtcpSdesItemType, value: String) -> Self {
        Self { item_type, value }
    }

    /// Create a new CNAME item
    pub fn cname(value: String) -> Self {
        Self::new(RtcpSdesItemType::CName, value)
    }

    /// Create a new NAME item
    pub fn name(value: String) -> Self {
        Self::new(RtcpSdesItemType::Name, value)
    }

    /// Create a new TOOL item
    pub fn tool(value: String) -> Self {
        Self::new(RtcpSdesItemType::Tool, value)
    }
}

/// RTCP Source Description (SDES) chunk
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSdesChunk {
    /// SSRC/CSRC identifier
    pub ssrc: RtpSsrc,

    /// SDES items
    pub items: Vec<RtcpSdesItem>,
}

impl RtcpSdesChunk {
    /// Create a new SDES chunk
    pub fn new(ssrc: RtpSsrc) -> Self {
        Self {
            ssrc,
            items: Vec::new(),
        }
    }

    /// Add an SDES item
    pub fn add_item(&mut self, item: RtcpSdesItem) {
        self.items.push(item);
    }
}

/// RTCP Source Description (SDES) packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSourceDescription {
    /// SDES chunks
    pub chunks: Vec<RtcpSdesChunk>,
}

impl RtcpSourceDescription {
    /// Create a new SDES packet
    pub fn new() -> Self {
        Self { chunks: Vec::new() }
    }

    /// Add an SDES chunk
    pub fn add_chunk(&mut self, chunk: RtcpSdesChunk) {
        self.chunks.push(chunk);
    }

    /// Add a source with optional CNAME
    pub fn add_source(&mut self, ssrc: RtpSsrc, cname: Option<String>) {
        let mut chunk = RtcpSdesChunk::new(ssrc);
        if let Some(cname_value) = cname {
            chunk.add_item(RtcpSdesItem::cname(cname_value));
        }
        self.add_chunk(chunk);
    }

    /// Find a chunk by SSRC
    pub fn find_chunk(&self, ssrc: RtpSsrc) -> Option<&RtcpSdesChunk> {
        self.chunks.iter().find(|chunk| chunk.ssrc == ssrc)
    }

    /// Find a CNAME for a source
    pub fn find_cname(&self, ssrc: RtpSsrc) -> Option<&str> {
        if let Some(chunk) = self.find_chunk(ssrc) {
            for item in &chunk.items {
                if item.item_type == RtcpSdesItemType::CName {
                    return Some(&item.value);
                }
            }
        }
        None
    }

    /// Serialize the SDES packet to bytes
    pub fn serialize(&self) -> Result<BytesMut> {
        // Calculate total size
        let mut total_size = 0;

        // Calculate size for each chunk
        for chunk in &self.chunks {
            // SSRC (4 bytes)
            total_size += 4;

            // Calculate size for each item
            for item in &chunk.items {
                // Type (1 byte) + Length (1 byte) + Value
                total_size += 2 + item.value.len();
            }

            // END item (1 byte) + padding to 32-bit boundary
            total_size += 1;
            if total_size % 4 != 0 {
                total_size += 4 - (total_size % 4);
            }
        }

        let mut buf = BytesMut::with_capacity(total_size);

        // Serialize each chunk
        for chunk in &self.chunks {
            // SSRC
            buf.put_u32(chunk.ssrc);

            // Serialize items
            for item in &chunk.items {
                // Item type
                buf.put_u8(item.item_type as u8);

                // Item length
                buf.put_u8(item.value.len() as u8);

                // Item value
                buf.put_slice(item.value.as_bytes());
            }

            // End marker
            buf.put_u8(RtcpSdesItemType::End as u8);

            // Pad to 32-bit boundary if needed
            let padding_bytes = (4 - (buf.len() % 4)) % 4;
            for _ in 0..padding_bytes {
                buf.put_u8(0);
            }
        }

        Ok(buf)
    }
}

/// Serialize an SDES packet
pub fn serialize_sdes(sdes: &RtcpSourceDescription) -> Result<BytesMut> {
    sdes.serialize()
}

/// Parse an SDES body using the source count from the RTCP common header.
///
/// Each declared chunk must contain an SSRC, a terminating END item, and
/// zero-valued alignment octets. Trailing or truncated data is rejected so a
/// malformed known member cannot be hidden inside an otherwise valid compound
/// packet.
pub fn parse_sdes(data: &[u8], source_count: u8) -> Result<RtcpSourceDescription> {
    let mut offset = 0usize;
    let mut description = RtcpSourceDescription::new();

    for _ in 0..source_count {
        if data.len().saturating_sub(offset) < 4 {
            return Err(Error::RtcpError(
                "SDES chunk is truncated before its SSRC".to_string(),
            ));
        }

        let ssrc = u32::from_be_bytes(
            data[offset..offset + 4]
                .try_into()
                .expect("the SDES SSRC length was checked"),
        );
        offset += 4;
        let mut chunk = RtcpSdesChunk::new(ssrc);

        loop {
            let Some(&item_type) = data.get(offset) else {
                return Err(Error::RtcpError(
                    "SDES chunk is missing its END item".to_string(),
                ));
            };
            offset += 1;

            if item_type == RtcpSdesItemType::End as u8 {
                break;
            }

            let Some(&item_length) = data.get(offset) else {
                return Err(Error::RtcpError(
                    "SDES item is truncated before its length".to_string(),
                ));
            };
            offset += 1;
            let item_length = usize::from(item_length);
            if data.len().saturating_sub(offset) < item_length {
                return Err(Error::RtcpError(
                    "SDES item value is shorter than its declared length".to_string(),
                ));
            }

            let value_bytes = &data[offset..offset + item_length];
            offset += item_length;

            // RFC 3550 deliberately leaves the SDES item registry open to
            // later assignments. Keep parsing the chunk when this crate does
            // not model a registered or future item type; otherwise valid
            // types such as MID (15) would make the entire compound packet
            // unusable. Modeled text items still receive UTF-8 validation.
            if let Ok(item_type) = RtcpSdesItemType::try_from(item_type) {
                if item_type == RtcpSdesItemType::Private {
                    // PRIV starts with a binary prefix-length octet, followed
                    // by separate UTF-8 prefix and value strings. The legacy
                    // `RtcpSdesItem { value: String }` cannot preserve that
                    // boundary, so validate and ignore it instead of either
                    // rejecting valid prefix lengths or inventing a lossy
                    // representation.
                    let (&prefix_length, text) = value_bytes.split_first().ok_or_else(|| {
                        Error::RtcpError("SDES PRIV item has no prefix length".to_string())
                    })?;
                    let prefix_length = usize::from(prefix_length);
                    if prefix_length > text.len() {
                        return Err(Error::RtcpError(
                            "SDES PRIV prefix exceeds its item length".to_string(),
                        ));
                    }
                    std::str::from_utf8(&text[..prefix_length]).map_err(|_| {
                        Error::RtcpError("SDES PRIV prefix is not valid UTF-8".to_string())
                    })?;
                    std::str::from_utf8(&text[prefix_length..]).map_err(|_| {
                        Error::RtcpError("SDES PRIV value is not valid UTF-8".to_string())
                    })?;
                } else {
                    let value = std::str::from_utf8(value_bytes)
                        .map_err(|_| Error::RtcpError("SDES item is not valid UTF-8".to_string()))?
                        .to_string();
                    chunk.add_item(RtcpSdesItem::new(item_type, value));
                }
            }
        }

        while offset % 4 != 0 {
            let Some(&padding) = data.get(offset) else {
                return Err(Error::RtcpError(
                    "SDES chunk alignment padding is truncated".to_string(),
                ));
            };
            if padding != 0 {
                return Err(Error::RtcpError(
                    "SDES chunk alignment padding must be zero".to_string(),
                ));
            }
            offset += 1;
        }

        description.add_chunk(chunk);
    }

    if offset != data.len() {
        return Err(Error::RtcpError(format!(
            "SDES source count leaves {} unexpected body bytes",
            data.len() - offset
        )));
    }

    Ok(description)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdes_item_creation() {
        let item = RtcpSdesItem::cname("user@example.com".to_string());
        assert_eq!(item.item_type, RtcpSdesItemType::CName);
        assert_eq!(item.value, "user@example.com");

        let item = RtcpSdesItem::name("Test User".to_string());
        assert_eq!(item.item_type, RtcpSdesItemType::Name);
        assert_eq!(item.value, "Test User");

        let item = RtcpSdesItem::tool("rVOIP RTP".to_string());
        assert_eq!(item.item_type, RtcpSdesItemType::Tool);
        assert_eq!(item.value, "rVOIP RTP");
    }

    #[test]
    fn test_sdes_chunk() {
        let mut chunk = RtcpSdesChunk::new(0x12345678);
        chunk.add_item(RtcpSdesItem::cname("user@example.com".to_string()));
        chunk.add_item(RtcpSdesItem::tool("rVOIP RTP".to_string()));

        assert_eq!(chunk.ssrc, 0x12345678);
        assert_eq!(chunk.items.len(), 2);
        assert_eq!(chunk.items[0].item_type, RtcpSdesItemType::CName);
        assert_eq!(chunk.items[0].value, "user@example.com");
        assert_eq!(chunk.items[1].item_type, RtcpSdesItemType::Tool);
        assert_eq!(chunk.items[1].value, "rVOIP RTP");
    }

    #[test]
    fn test_sdes_packet() {
        let mut sdes = RtcpSourceDescription::new();

        // Add a chunk with CNAME and TOOL
        let mut chunk1 = RtcpSdesChunk::new(0x12345678);
        chunk1.add_item(RtcpSdesItem::cname("user1@example.com".to_string()));
        chunk1.add_item(RtcpSdesItem::tool("rVOIP RTP".to_string()));
        sdes.add_chunk(chunk1);

        // Add a source with just CNAME
        sdes.add_source(0xabcdef01, Some("user2@example.com".to_string()));

        // Verify chunks were added
        assert_eq!(sdes.chunks.len(), 2);

        // Verify first chunk
        assert_eq!(sdes.chunks[0].ssrc, 0x12345678);
        assert_eq!(sdes.chunks[0].items.len(), 2);

        // Verify second chunk
        assert_eq!(sdes.chunks[1].ssrc, 0xabcdef01);
        assert_eq!(sdes.chunks[1].items.len(), 1);
        assert_eq!(sdes.chunks[1].items[0].item_type, RtcpSdesItemType::CName);
        assert_eq!(sdes.chunks[1].items[0].value, "user2@example.com");

        // Test finding chunks and CNAMEs
        let chunk = sdes.find_chunk(0x12345678);
        assert!(chunk.is_some());
        assert_eq!(chunk.unwrap().ssrc, 0x12345678);

        let cname = sdes.find_cname(0xabcdef01);
        assert!(cname.is_some());
        assert_eq!(cname.unwrap(), "user2@example.com");

        // Test for non-existent SSRC
        let chunk = sdes.find_chunk(0x99999999);
        assert!(chunk.is_none());

        let cname = sdes.find_cname(0x99999999);
        assert!(cname.is_none());
    }

    #[test]
    fn serialized_sdes_round_trips_with_exact_source_count() {
        let mut sdes = RtcpSourceDescription::new();
        sdes.add_source(0x1234_5678, Some("alice@example.test".to_string()));
        sdes.add_source(0x90ab_cdef, None);

        let bytes = sdes.serialize().unwrap();
        assert_eq!(parse_sdes(&bytes, 2).unwrap(), sdes);
    }

    #[test]
    fn source_count_and_chunk_termination_are_enforced() {
        // One declared chunk but no SSRC.
        assert!(parse_sdes(&[], 1).is_err());
        // SSRC without the mandatory END item.
        assert!(parse_sdes(&[0x12, 0x34, 0x56, 0x78], 1).is_err());
        // A second body word cannot be ignored when the source count is zero.
        assert!(parse_sdes(&[0, 0, 0, 0], 0).is_err());
        // Alignment bytes after END must be present and zero.
        assert!(parse_sdes(&[0x12, 0x34, 0x56, 0x78, 0, 0, 1, 0], 1).is_err());
    }

    #[test]
    fn registered_unmodeled_item_does_not_hide_following_known_items() {
        let body = [
            0x12, 0x34, 0x56, 0x78, // SSRC
            15, 3, b'm', b'i', b'd', // IANA-assigned MID, not modeled here
            1, 5, b'a', b'l', b'i', b'c', b'e', // CNAME
            0, 0, 0, 0, // END and chunk alignment
        ];

        let parsed = parse_sdes(&body, 1).unwrap();
        assert_eq!(parsed.chunks.len(), 1);
        assert_eq!(parsed.chunks[0].items.len(), 1);
        assert_eq!(parsed.find_cname(0x12345678), Some("alice"));
    }

    #[test]
    fn private_item_uses_its_rfc_length_prefixed_shape() {
        let body = [
            0x12, 0x34, 0x56, 0x78, // SSRC
            8, 4, 2, b'i', b'd', b'v', // PRIV: prefix "id", value "v"
            1, 1, b'a', // CNAME
            0, 0, 0, // END and chunk alignment
        ];
        let parsed = parse_sdes(&body, 1).unwrap();
        assert_eq!(parsed.find_cname(0x12345678), Some("a"));
        assert_eq!(parsed.chunks[0].items.len(), 1);

        let malformed = [
            0x12, 0x34, 0x56, 0x78, // SSRC
            8, 2, 3, b'x', // prefix length exceeds item body
            0,    // END; already aligned
        ];
        assert!(parse_sdes(&malformed, 1).is_err());
    }
}
