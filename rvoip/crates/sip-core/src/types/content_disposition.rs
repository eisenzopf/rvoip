use std::collections::HashMap;
use crate::parser::headers::parse_content_disposition;
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::types::param::Param;
use serde::{Serialize, Deserialize};

/// Content Disposition Type (session, render, icon, alert, etc.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl FromStr for DispositionType {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "session" => Ok(DispositionType::Session),
            "render" => Ok(DispositionType::Render),
            "icon" => Ok(DispositionType::Icon),
            "alert" => Ok(DispositionType::Alert),
            _ => Ok(DispositionType::Other(s.to_string())),
        }
    }
}

/// Typed Content-Disposition header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::content_disposition::parse_content_disposition;
        use nom::combinator::all_consuming;

        all_consuming(parse_content_disposition)(s.as_bytes())
            .map_err(Error::from)
            .and_then(|(_, (dtype_bytes, params_vec))| {
                // String is already a String type, so we don't need to_vec()
                let disp_type = match dtype_bytes.as_str() {
                    "session" => DispositionType::Session,
                    "render" => DispositionType::Render,
                    "icon" => DispositionType::Icon,
                    "alert" => DispositionType::Alert,
                    _ => DispositionType::Other(dtype_bytes),
                };
                
                // Convert params to HashMap
                let mut params = HashMap::new();
                for param in params_vec {
                    // Use a match statement instead of pattern matching directly
                    match param {
                        // Skip handling-params for now - consider adding them separately
                        param => {
                            // Try to extract generic params
                            if let Ok(param_str) = std::str::from_utf8(format!("{:?}", param).as_bytes()) {
                                if param_str.contains("Generic(Param::Other") {
                                    // Extract key and value from debug output as a fallback
                                    if let Some(start) = param_str.find("\"") {
                                        if let Some(end) = param_str[start+1..].find("\"") {
                                            let key = param_str[start+1..start+1+end].to_lowercase();
                                            
                                            // Try to extract value
                                            if let Some(v_start) = param_str[start+1+end..].find("\"") {
                                                if let Some(v_end) = param_str[start+1+end+v_start+1..].find("\"") {
                                                    let value = param_str[start+1+end+v_start+1..start+1+end+v_start+1+v_end].to_string();
                                                    params.insert(key, value);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                
                Ok(ContentDisposition { disposition_type: disp_type, params })
            })
    }
}

// TODO: Implement methods, FromStr, Display 