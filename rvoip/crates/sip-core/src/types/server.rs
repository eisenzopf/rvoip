//! # SIP Server Header
//!
//! This module provides an implementation of the SIP Server header as defined in
//! [RFC 3261 Section 20.35](https://datatracker.ietf.org/doc/html/rfc3261#section-20.35).
//!
//! The Server header contains information about the software used by the UAS (User Agent Server)
//! or proxy server that generated a response. This information is typically included in response 
//! messages to identify the server software.
//!
//! ## Purpose
//!
//! The Server header serves several purposes:
//!
//! - Provides information about the server software implementation
//! - Aids in debugging and troubleshooting
//! - Allows tracking of SIP implementation distribution and versions
//!
//! ## Format
//!
//! ```
//! Server: ProductName/1.0
//! Server: ProductName/1.0 (Comment)
//! Server: ProductName/1.0 (Comment) AnotherProduct/2.0
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//!
//! // Create a server info header with products and comments
//! let server = ServerInfo::new()
//!     .with_product("SIP-Server", Some("1.0"))
//!     .with_comment("SIP Core Library")
//!     .with_product("OS", Some("Unix"));
//!
//! assert_eq!(server.to_string(), "SIP-Server/1.0 (SIP Core Library) OS/Unix");
//! ```

// Server header type for SIP messages
// Format defined in RFC 3261 Section 20.35

use std::fmt;
use serde::{Serialize, Deserialize};

/// ServerInfo represents the software used by the server
/// Used in the Server header of SIP responses
///
/// The Server header identifies the software products used by the User Agent Server (UAS) or
/// proxy that processed the request. It contains a list of product names, versions,
/// and optional comments.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a server header with product and version
/// let server = ServerInfo::new()
///     .with_product("MyServer", Some("1.0"));
///
/// // Create a more detailed server header
/// let server = ServerInfo::new()
///     .with_product("SIPCore", Some("2.1"))
///     .with_comment("High Performance Edition")
///     .with_product("OS", Some("Linux"));
///
/// assert_eq!(
///     server.to_string(),
///     "SIPCore/2.1 (High Performance Edition) OS/Linux"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// List of product tokens and comments
    pub products: Vec<ServerProduct>,
}

/// ServerProduct represents a single product in the Server header
///
/// Each ServerProduct can be either a product with an optional version,
/// or a comment enclosed in parentheses. A Server header can contain
/// multiple products and comments in sequence.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Product with version
/// let product = ServerProduct::Product {
///     name: "MySIPServer".to_string(),
///     version: Some("1.0".to_string()),
/// };
///
/// // Comment
/// let comment = ServerProduct::Comment("Test Build".to_string());
/// ```
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
///
/// This is an internal representation used by the parser to construct
/// ServerProduct instances. It contains a product name and an optional version.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::server::Product;
///
/// // Product with version
/// let product = Product {
///     name: "MySIPServer".to_string(),
///     version: Some("1.0".to_string()),
/// };
///
/// // Product without version
/// let product = Product {
///     name: "MySIPServer".to_string(),
///     version: None,
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Product {
    /// The product name
    pub name: String,
    /// The optional version string
    pub version: Option<String>,
}

/// Represents a single component of Server/User-Agent header
/// Used by the parser module
///
/// This is an internal representation used by the parser to construct
/// ServerInfo instances. It can be either a Product or a Comment.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::server::{ServerVal, Product};
///
/// // Product value
/// let product_val = ServerVal::Product(Product {
///     name: "MySIPServer".to_string(),
///     version: Some("1.0".to_string()),
/// });
///
/// // Comment value
/// let comment_val = ServerVal::Comment("Test Build".to_string());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerVal {
    /// A product component
    Product(Product),
    /// A comment component
    Comment(String),
}

impl ServerInfo {
    /// Create a new empty ServerInfo
    ///
    /// Initializes a ServerInfo with an empty list of products.
    ///
    /// # Returns
    ///
    /// A new empty `ServerInfo` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let server = ServerInfo::new();
    /// assert!(server.products.is_empty());
    /// assert_eq!(server.to_string(), "");
    /// ```
    pub fn new() -> Self {
        ServerInfo {
            products: Vec::new(),
        }
    }

    /// Add a product to the server info
    ///
    /// Adds a product with an optional version to the list of products.
    ///
    /// # Parameters
    ///
    /// - `name`: The product name
    /// - `version`: Optional version string
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut server = ServerInfo::new();
    ///
    /// // Add a product with version
    /// server.add_product("MySIPServer", Some("1.0"));
    /// assert_eq!(server.to_string(), "MySIPServer/1.0");
    ///
    /// // Add a product without version
    /// server.add_product("Gateway", None);
    /// assert_eq!(server.to_string(), "MySIPServer/1.0 Gateway");
    /// ```
    pub fn add_product(&mut self, name: &str, version: Option<&str>) {
        self.products.push(ServerProduct::Product {
            name: name.to_string(),
            version: version.map(|v| v.to_string()),
        });
    }

    /// Add a comment to the server info
    ///
    /// Adds a comment to the list of products. In the formatted Server
    /// header, comments are enclosed in parentheses.
    ///
    /// # Parameters
    ///
    /// - `comment`: The comment text
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut server = ServerInfo::new();
    ///
    /// // Add a comment
    /// server.add_comment("Debug Build");
    /// assert_eq!(server.to_string(), "(Debug Build)");
    ///
    /// // Add a product after the comment
    /// server.add_product("SIPCore", Some("2.0"));
    /// assert_eq!(server.to_string(), "(Debug Build) SIPCore/2.0");
    /// ```
    pub fn add_comment(&mut self, comment: &str) {
        self.products.push(ServerProduct::Comment(comment.to_string()));
    }

    /// Create a builder method for adding products
    ///
    /// Adds a product with an optional version and returns the modified
    /// ServerInfo for method chaining.
    ///
    /// # Parameters
    ///
    /// - `name`: The product name
    /// - `version`: Optional version string
    ///
    /// # Returns
    ///
    /// The modified `ServerInfo` instance (builder pattern)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let server = ServerInfo::new()
    ///     .with_product("SIPCore", Some("1.0"))
    ///     .with_product("Gateway", None);
    ///
    /// assert_eq!(server.to_string(), "SIPCore/1.0 Gateway");
    /// ```
    pub fn with_product(mut self, name: &str, version: Option<&str>) -> Self {
        self.add_product(name, version);
        self
    }

    /// Create a builder method for adding comments
    ///
    /// Adds a comment and returns the modified ServerInfo for method chaining.
    ///
    /// # Parameters
    ///
    /// - `comment`: The comment text
    ///
    /// # Returns
    ///
    /// The modified `ServerInfo` instance (builder pattern)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let server = ServerInfo::new()
    ///     .with_product("SIPCore", Some("1.0"))
    ///     .with_comment("Release Build")
    ///     .with_product("OS", Some("Unix"));
    ///
    /// assert_eq!(server.to_string(), "SIPCore/1.0 (Release Build) OS/Unix");
    /// ```
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.add_comment(comment);
        self
    }
}

impl fmt::Display for ServerInfo {
    /// Formats the ServerInfo as a string.
    ///
    /// Formats the server header according to RFC 3261, with products
    /// and their versions, and comments in parentheses.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    ///
    /// let server = ServerInfo::new()
    ///     .with_product("SIPCore", Some("1.0"))
    ///     .with_comment("Test Build")
    ///     .with_product("OS", Some("Unix"));
    ///
    /// assert_eq!(server.to_string(), "SIPCore/1.0 (Test Build) OS/Unix");
    /// ```
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
    /// Provides a default instance of ServerInfo.
    ///
    /// The default instance is an empty server info with no products or comments.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let server = ServerInfo::default();
    /// assert!(server.products.is_empty());
    /// assert_eq!(server.to_string(), "");
    /// ```
    fn default() -> Self {
        Self::new()
    }
}

// Conversion from parser representation to header representation
impl From<Vec<ServerVal>> for ServerInfo {
    /// Creates a ServerInfo from a vector of ServerVal.
    ///
    /// Converts the parser's internal representation to the public API representation.
    ///
    /// # Parameters
    ///
    /// - `vals`: Vector of ServerVal instances from the parser
    ///
    /// # Returns
    ///
    /// A new ServerInfo constructed from the provided values
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::server::{ServerInfo, ServerVal, Product};
    ///
    /// let vals = vec![
    ///     ServerVal::Product(Product {
    ///         name: "SIPCore".to_string(),
    ///         version: Some("1.0".to_string()),
    ///     }),
    ///     ServerVal::Comment("Test Build".to_string()),
    /// ];
    ///
    /// let server_info = ServerInfo::from(vals);
    /// assert_eq!(server_info.to_string(), "SIPCore/1.0 (Test Build)");
    /// ```
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