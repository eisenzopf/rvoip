use std::collections::HashMap;
use crate::parser::headers::parse_content_disposition;
use crate::error::Result;
use std::fmt;
use std::str::FromStr;

/// Content Disposition Type (session, render, icon, alert, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispositionType {
    Session,
    Render,
    Icon,
    Alert,
    Other(String),
}

impl fmt::Display for DispositionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DispositionType::Session => write!(f, "session"),
            DispositionType::Render => write!(f, "render"),
            DispositionType::Icon => write!(f, "icon"),
            DispositionType::Alert => write!(f, "alert"),
            DispositionType::Other(s) => write!(f, "{}", s),
        }
    }
}

/// Typed Content-Disposition header.
#[derive(Debug, Clone, PartialEq, Eq)] // Add derives as needed
pub struct ContentDisposition {
    pub disposition_type: DispositionType,
    pub params: HashMap<String, String>,
}

impl fmt::Display for ContentDisposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.disposition_type)?;
        for (key, value) in &self.params {
            // Remove internal quote escaping for now
            if value.chars().any(|c| !c.is_ascii_alphanumeric() && !matches!(c, '!' | '#' | '$' | '%' | '&' | '\'' | '^' | '_' | '`' | '{' | '}' | '~' | '-')) {
                write!(f, ";{}=\"{}\"", key, value)?;
            } else {
                write!(f, ";{}={}", key, value)?;
            }
        }
        Ok(())
    }
}

impl FromStr for ContentDisposition {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        parse_content_disposition(s)
    }
}

// TODO: Implement methods, FromStr, Display 