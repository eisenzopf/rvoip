use bytes::Bytes;
use crate::types::header::{Header, HeaderName};
use crate::types::content_type::ContentType;
use crate::error::{Error, Result};
use crate::sdp::SdpSession;
use std::fmt;

/// Represents a parsed MIME part.
#[derive(Debug, Clone, PartialEq)]
pub struct MimePart {
    /// Headers associated with this part.
    pub headers: Vec<Header>,
    /// The raw content bytes of this part.
    pub raw_content: Bytes,
    /// Optionally parsed content based on Content-Type.
    pub parsed_content: Option<ParsedBody>,
}

impl MimePart {
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
            raw_content: Bytes::new(),
            parsed_content: None,
        }
    }

    pub fn content_type(&self) -> Option<String> {
         self.headers.iter()
            .find(|h| h.name == HeaderName::ContentType)
            .and_then(|h| h.value.as_text())
    }
}

impl Default for MimePart {
    fn default() -> Self {
        Self::new()
    }
}

/// Represents the different types of parsed body content.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedBody {
    /// Session Description Protocol data.
    Sdp(SdpSession),
    /// Plain text content.
    Text(String),
    /// Other content types stored as raw bytes.
    Other(Bytes),
}

/// Represents a parsed multipart MIME body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MultipartBody {
    /// The boundary string used to separate parts.
    pub boundary: String,
    /// The MIME parts contained in the body.
    pub parts: Vec<MimePart>,
}

impl MultipartBody {
    /// Creates a new MultipartBody with a given boundary.
    pub fn new(boundary: impl Into<String>) -> Self {
        Self {
            boundary: boundary.into(),
            parts: Vec::new(),
        }
    }

    /// Adds a MIME part to the body.
    pub fn add_part(&mut self, part: MimePart) {
        self.parts.push(part);
    }
}

// TODO: Add methods for serialization if needed. 