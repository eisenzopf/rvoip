//! # Presence Information Data Format (PIDF)
//!
//! This module provides a minimal implementation of PIDF (Presence Information Data Format)
//! as defined in RFC 3863. This is a simplified version suitable for basic presence operations
//! in SIP SIMPLE.
//!
//! ## Structure
//!
//! A PIDF document consists of:
//! - A presence element with an entity attribute
//! - One or more tuple elements containing status information
//! - Optional note elements for human-readable text
//!
//! ## Example PIDF Document
//!
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <presence xmlns="urn:ietf:params:xml:ns:pidf"
//!           entity="pres:alice@example.com">
//!   <tuple id="t1">
//!     <status>
//!       <basic>open</basic>
//!     </status>
//!     <contact>sip:alice@192.168.1.10</contact>
//!     <timestamp>2024-01-15T14:00:00Z</timestamp>
//!   </tuple>
//!   <note>Available for calls</note>
//! </presence>
//! ```

use std::fmt;
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use crate::{Error, Result};

/// Basic presence status values as defined in RFC 3863
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BasicStatus {
    /// The principal is available for communication
    Open,
    /// The principal is not available for communication
    Closed,
}

impl fmt::Display for BasicStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BasicStatus::Open => write!(f, "open"),
            BasicStatus::Closed => write!(f, "closed"),
        }
    }
}

impl std::str::FromStr for BasicStatus {
    type Err = Error;
    
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "open" => Ok(BasicStatus::Open),
            "closed" => Ok(BasicStatus::Closed),
            _ => Err(Error::ParseError(format!("Invalid basic status: {}", s))),
        }
    }
}

/// Status information for a presence tuple
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Basic presence status (open/closed)
    pub basic: BasicStatus,
}

impl Status {
    /// Create a new Status with the given basic value
    pub fn new(basic: BasicStatus) -> Self {
        Self { basic }
    }
    
    /// Create an "open" status
    pub fn open() -> Self {
        Self::new(BasicStatus::Open)
    }
    
    /// Create a "closed" status
    pub fn closed() -> Self {
        Self::new(BasicStatus::Closed)
    }
}

/// A presence tuple representing a single device or endpoint
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tuple {
    /// Unique identifier for this tuple
    pub id: String,
    /// Status information
    pub status: Status,
    /// Optional contact URI for this tuple
    pub contact: Option<String>,
    /// Optional timestamp for when this status was set
    pub timestamp: Option<DateTime<Utc>>,
    /// Optional priority (higher values = higher priority)
    pub priority: Option<f32>,
}

impl Tuple {
    /// Create a new tuple with the given ID and status
    pub fn new(id: impl Into<String>, status: Status) -> Self {
        Self {
            id: id.into(),
            status,
            contact: None,
            timestamp: None,
            priority: None,
        }
    }
    
    /// Set the contact URI for this tuple
    pub fn with_contact(mut self, contact: impl Into<String>) -> Self {
        self.contact = Some(contact.into());
        self
    }
    
    /// Set the timestamp for this tuple
    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = Some(timestamp);
        self
    }
    
    /// Set the priority for this tuple
    pub fn with_priority(mut self, priority: f32) -> Self {
        self.priority = Some(priority);
        self
    }
}

/// A PIDF presence document
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PidfDocument {
    /// The entity this presence information is about (e.g., "pres:alice@example.com")
    pub entity: String,
    /// One or more tuples containing presence information
    pub tuples: Vec<Tuple>,
    /// Optional human-readable notes
    pub notes: Vec<String>,
}

impl PidfDocument {
    /// Create a new PIDF document for the given entity
    pub fn new(entity: impl Into<String>) -> Self {
        Self {
            entity: entity.into(),
            tuples: Vec::new(),
            notes: Vec::new(),
        }
    }
    
    /// Add a tuple to this document
    pub fn add_tuple(mut self, tuple: Tuple) -> Self {
        self.tuples.push(tuple);
        self
    }
    
    /// Add a note to this document
    pub fn add_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
    
    /// Create a simple "available" presence document
    pub fn available(entity: impl Into<String>) -> Self {
        let entity_str = entity.into();
        Self::new(entity_str.clone())
            .add_tuple(
                Tuple::new("t1", Status::open())
                    .with_timestamp(Utc::now())
            )
    }
    
    /// Create a simple "unavailable" presence document
    pub fn unavailable(entity: impl Into<String>) -> Self {
        let entity_str = entity.into();
        Self::new(entity_str.clone())
            .add_tuple(
                Tuple::new("t1", Status::closed())
                    .with_timestamp(Utc::now())
            )
    }
    
    /// Serialize this PIDF document to XML string
    ///
    /// This produces a simplified but RFC-compliant PIDF XML document.
    /// For full XML features, consider using a dedicated XML library.
    pub fn to_xml(&self) -> String {
        let mut xml = String::new();
        
        // XML declaration
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        
        // Presence element with namespace and entity
        xml.push_str(&format!(
            "<presence xmlns=\"urn:ietf:params:xml:ns:pidf\" entity=\"{}\">\n",
            escape_xml(&self.entity)
        ));
        
        // Tuples
        for tuple in &self.tuples {
            xml.push_str(&format!("  <tuple id=\"{}\">\n", escape_xml(&tuple.id)));
            
            // Status
            xml.push_str("    <status>\n");
            xml.push_str(&format!("      <basic>{}</basic>\n", tuple.status.basic));
            xml.push_str("    </status>\n");
            
            // Optional contact
            if let Some(contact) = &tuple.contact {
                xml.push_str(&format!("    <contact>{}</contact>\n", escape_xml(contact)));
            }
            
            // Optional timestamp
            if let Some(timestamp) = &tuple.timestamp {
                xml.push_str(&format!(
                    "    <timestamp>{}</timestamp>\n",
                    timestamp.to_rfc3339()
                ));
            }
            
            // Optional priority
            if let Some(priority) = &tuple.priority {
                xml.push_str(&format!("    <priority>{}</priority>\n", priority));
            }
            
            xml.push_str("  </tuple>\n");
        }
        
        // Notes
        for note in &self.notes {
            xml.push_str(&format!("  <note>{}</note>\n", escape_xml(note)));
        }
        
        xml.push_str("</presence>");
        xml
    }
    
    /// Parse a PIDF document from XML string
    ///
    /// This is a minimal parser that handles basic PIDF documents.
    /// For full XML parsing, consider using a dedicated XML library.
    pub fn from_xml(xml: &str) -> Result<Self> {
        // This is a very basic parser for demonstration
        // In production, you would use a proper XML parser
        
        // Extract entity attribute
        let entity = extract_attribute(xml, "entity")?;
        
        let mut doc = Self::new(entity);
        
        // Extract tuples
        let mut tuple_start = 0;
        while let Some(start) = xml[tuple_start..].find("<tuple") {
            let start = tuple_start + start;
            if let Some(end) = xml[start..].find("</tuple>") {
                let end = start + end + 8; // Include </tuple>
                let tuple_xml = &xml[start..end];
                
                // Extract tuple ID
                let id = extract_attribute(tuple_xml, "id")?;
                
                // Extract basic status
                let basic = if tuple_xml.contains("<basic>open</basic>") {
                    BasicStatus::Open
                } else if tuple_xml.contains("<basic>closed</basic>") {
                    BasicStatus::Closed
                } else {
                    return Err(Error::ParseError("Missing or invalid basic status".to_string()));
                };
                
                let mut tuple = Tuple::new(id, Status::new(basic));
                
                // Extract optional contact
                if let Ok(contact) = extract_element(tuple_xml, "contact") {
                    tuple.contact = Some(contact);
                }
                
                // Extract optional timestamp
                if let Ok(timestamp_str) = extract_element(tuple_xml, "timestamp") {
                    if let Ok(timestamp) = DateTime::parse_from_rfc3339(&timestamp_str) {
                        tuple.timestamp = Some(timestamp.with_timezone(&Utc));
                    }
                }
                
                // Extract optional priority
                if let Ok(priority_str) = extract_element(tuple_xml, "priority") {
                    if let Ok(priority) = priority_str.parse::<f32>() {
                        tuple.priority = Some(priority);
                    }
                }
                
                doc.tuples.push(tuple);
                tuple_start = end;
            } else {
                break;
            }
        }
        
        // Extract notes
        let mut note_start = 0;
        while let Some(start) = xml[note_start..].find("<note>") {
            let start = note_start + start + 6; // Skip <note>
            if let Some(end) = xml[start..].find("</note>") {
                let note = xml[start..start + end].to_string();
                doc.notes.push(unescape_xml(&note));
                note_start = start + end + 7; // Skip </note>
            } else {
                break;
            }
        }
        
        Ok(doc)
    }
}

/// Escape special XML characters
fn escape_xml(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '&' => "&amp;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// Unescape XML entities
fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Extract an attribute value from XML
fn extract_attribute(xml: &str, attr: &str) -> Result<String> {
    let pattern = format!("{}=\"", attr);
    if let Some(start) = xml.find(&pattern) {
        let start = start + pattern.len();
        if let Some(end) = xml[start..].find('"') {
            return Ok(xml[start..start + end].to_string());
        }
    }
    Err(Error::ParseError(format!("Attribute {} not found", attr)))
}

/// Extract an element's text content from XML
fn extract_element(xml: &str, element: &str) -> Result<String> {
    let start_tag = format!("<{}>", element);
    let end_tag = format!("</{}>", element);
    
    if let Some(start) = xml.find(&start_tag) {
        let start = start + start_tag.len();
        if let Some(end) = xml[start..].find(&end_tag) {
            return Ok(xml[start..start + end].to_string());
        }
    }
    Err(Error::ParseError(format!("Element {} not found", element)))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_status() {
        assert_eq!(BasicStatus::Open.to_string(), "open");
        assert_eq!(BasicStatus::Closed.to_string(), "closed");
        
        assert_eq!("open".parse::<BasicStatus>().unwrap(), BasicStatus::Open);
        assert_eq!("closed".parse::<BasicStatus>().unwrap(), BasicStatus::Closed);
        assert!("invalid".parse::<BasicStatus>().is_err());
    }
    
    #[test]
    fn test_pidf_available() {
        let doc = PidfDocument::available("pres:alice@example.com");
        assert_eq!(doc.entity, "pres:alice@example.com");
        assert_eq!(doc.tuples.len(), 1);
        assert_eq!(doc.tuples[0].status.basic, BasicStatus::Open);
    }
    
    #[test]
    fn test_pidf_unavailable() {
        let doc = PidfDocument::unavailable("pres:bob@example.com");
        assert_eq!(doc.entity, "pres:bob@example.com");
        assert_eq!(doc.tuples.len(), 1);
        assert_eq!(doc.tuples[0].status.basic, BasicStatus::Closed);
    }
    
    #[test]
    fn test_pidf_to_xml() {
        let doc = PidfDocument::new("pres:alice@example.com")
            .add_tuple(
                Tuple::new("t1", Status::open())
                    .with_contact("sip:alice@192.168.1.10")
            )
            .add_note("Available for calls");
        
        let xml = doc.to_xml();
        assert!(xml.contains("entity=\"pres:alice@example.com\""));
        assert!(xml.contains("<tuple id=\"t1\">"));
        assert!(xml.contains("<basic>open</basic>"));
        assert!(xml.contains("<contact>sip:alice@192.168.1.10</contact>"));
        assert!(xml.contains("<note>Available for calls</note>"));
    }
    
    #[test]
    fn test_pidf_from_xml() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<presence xmlns="urn:ietf:params:xml:ns:pidf" entity="pres:alice@example.com">
  <tuple id="t1">
    <status>
      <basic>open</basic>
    </status>
    <contact>sip:alice@192.168.1.10</contact>
  </tuple>
  <note>Available</note>
</presence>"#;
        
        let doc = PidfDocument::from_xml(xml).unwrap();
        assert_eq!(doc.entity, "pres:alice@example.com");
        assert_eq!(doc.tuples.len(), 1);
        assert_eq!(doc.tuples[0].id, "t1");
        assert_eq!(doc.tuples[0].status.basic, BasicStatus::Open);
        assert_eq!(doc.tuples[0].contact, Some("sip:alice@192.168.1.10".to_string()));
        assert_eq!(doc.notes, vec!["Available"]);
    }
    
    #[test]
    fn test_xml_escaping() {
        let doc = PidfDocument::new("pres:alice@example.com")
            .add_note("Status: <available> & busy");
        
        let xml = doc.to_xml();
        assert!(xml.contains("&lt;available&gt;"));
        assert!(xml.contains("&amp;"));
    }
}

/// Type alias for convenience and compatibility
pub type Presence = PidfDocument;