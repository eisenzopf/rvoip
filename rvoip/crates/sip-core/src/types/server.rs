// Server header type for SIP messages
// Format defined in RFC 3261 Section 20.35

use std::fmt;
use serde::{Serialize, Deserialize};

/// ServerInfo represents the software used by the server
/// Used in the Server header of SIP responses
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// List of product tokens and comments
    pub products: Vec<ServerProduct>,
}

/// ServerProduct represents a single product in the Server header
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerProduct {
    /// A product with optional version
    Product {
        /// Product name
        name: String,
        /// Optional version string
        version: Option<String>,
    },
    /// A comment (in parentheses)
    Comment(String),
}

/// Represents a product token with optional version used in Server/User-Agent headers
/// Used by the parser module
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Product {
    pub name: String,
    pub version: Option<String>,
}

/// Represents a single component of Server/User-Agent header
/// Used by the parser module
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerVal {
    Product(Product),
    Comment(String),
}

impl ServerInfo {
    /// Create a new empty ServerInfo
    pub fn new() -> Self {
        ServerInfo {
            products: Vec::new(),
        }
    }

    /// Add a product to the server info
    pub fn add_product(&mut self, name: &str, version: Option<&str>) {
        self.products.push(ServerProduct::Product {
            name: name.to_string(),
            version: version.map(|v| v.to_string()),
        });
    }

    /// Add a comment to the server info
    pub fn add_comment(&mut self, comment: &str) {
        self.products.push(ServerProduct::Comment(comment.to_string()));
    }

    /// Create a builder method for adding products
    pub fn with_product(mut self, name: &str, version: Option<&str>) -> Self {
        self.add_product(name, version);
        self
    }

    /// Create a builder method for adding comments
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.add_comment(comment);
        self
    }
}

impl fmt::Display for ServerInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        
        for product in &self.products {
            if !first {
                write!(f, " ")?;
            }
            
            match product {
                ServerProduct::Product { name, version } => {
                    write!(f, "{}", name)?;
                    if let Some(ver) = version {
                        write!(f, "/{}", ver)?;
                    }
                },
                ServerProduct::Comment(comment) => {
                    write!(f, "({})", comment)?;
                }
            }
            
            first = false;
        }
        
        Ok(())
    }
}

impl Default for ServerInfo {
    fn default() -> Self {
        Self::new()
    }
}

// Conversion from parser representation to header representation
impl From<Vec<ServerVal>> for ServerInfo {
    fn from(vals: Vec<ServerVal>) -> Self {
        let mut info = ServerInfo::new();
        for val in vals {
            match val {
                ServerVal::Product(p) => {
                    info.add_product(&p.name, p.version.as_deref());
                },
                ServerVal::Comment(c) => {
                    info.add_comment(&c);
                }
            }
        }
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_server_info_display() {
        let server = ServerInfo::new()
            .with_product("SIP-Server", Some("1.0"))
            .with_comment("SIP Core Library")
            .with_product("OS", Some("Unix"));
            
        assert_eq!(server.to_string(), "SIP-Server/1.0 (SIP Core Library) OS/Unix");
    }
    
    #[test]
    fn test_server_info_no_version() {
        let server = ServerInfo::new()
            .with_product("MyServer", None);
            
        assert_eq!(server.to_string(), "MyServer");
    }
    
    #[test]
    fn test_server_info_just_comment() {
        let server = ServerInfo::new()
            .with_comment("Test Server");
            
        assert_eq!(server.to_string(), "(Test Server)");
    }
    
    #[test]
    fn test_server_info_empty() {
        let server = ServerInfo::new();
        assert_eq!(server.to_string(), "");
    }
    
    #[test]
    fn test_from_server_val() {
        let vals = vec![
            ServerVal::Product(Product {
                name: "MyProduct".to_string(),
                version: Some("1.0".to_string())
            }),
            ServerVal::Comment("Test Build".to_string())
        ];
        
        let info = ServerInfo::from(vals);
        assert_eq!(info.products.len(), 2);
        assert_eq!(info.to_string(), "MyProduct/1.0 (Test Build)");
    }
} 