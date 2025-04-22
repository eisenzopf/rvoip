use std::collections::HashMap;
use crate::parser::headers::parse_content_disposition;
use crate::error::Result;
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;

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
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use crate::parser::headers::content_disposition::parse_content_disposition;
        use nom::combinator::all_consuming;

        match all_consuming(parse_content_disposition)(s.as_bytes()) {
            Ok((_, (dtype_bytes, params_vec))) => {
                let disp_type_str = String::from_utf8(dtype_bytes.to_vec())?;
                let disp_type = DispositionType::Other(disp_type_str);
                // Convert params Vec<Param> -> HashMap<String, String>
                // TODO: Refine this conversion, especially for handling
                let params = params_vec.into_iter()
                    .filter_map(|p| {
                        if let Param::Other(k, v_opt) = p {
                            Some((k.to_lowercase(), v_opt.unwrap_or_default()))
                        } else {
                            None // Ignore non-Other params for now
                        }
                    })
                    .collect();
                Ok(ContentDisposition { disposition_type: disp_type, params })
            },
            Err(e) => Err(Error::ParsingError{ 
                message: format!("Failed to parse Content-Disposition header: {:?}", e), 
                source: None 
            })
        }
    }
}

// TODO: Implement methods, FromStr, Display 