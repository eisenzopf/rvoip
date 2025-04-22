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
                // Convert Vec<u8> to String properly
                let disp_type_str = String::from_utf8(dtype_bytes.to_vec())?;
                let disp_type = match disp_type_str.to_lowercase().as_str() {
                    "session" => DispositionType::Session,
                    "render" => DispositionType::Render,
                    "icon" => DispositionType::Icon,
                    "alert" => DispositionType::Alert,
                    _ => DispositionType::Other(disp_type_str),
                };
                
                let params = params_vec.into_iter()
                    .filter_map(|p| {
                        if let crate::parser::headers::content_disposition::DispositionParam::Generic(Param::Other(k, v_opt)) = p {
                            let value_str = v_opt.map(|gv| gv.to_string()).unwrap_or_default();
                            Some((k.to_lowercase(), value_str))
                        } else {
                            None
                        }
                    })
                    .collect();
                Ok(ContentDisposition { disposition_type: disp_type, params })
            })
    }
}

// TODO: Implement methods, FromStr, Display 