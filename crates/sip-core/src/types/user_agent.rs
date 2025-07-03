//! # SIP User-Agent Header
//!
//! This module provides an implementation of the SIP User-Agent header as defined in
//! [RFC 3261 Section 20.41](https://datatracker.ietf.org/doc/html/rfc3261#section-20.41).
//!
//! The User-Agent header field contains information about the UAC originating the request.
//! The User-Agent header field contains a textual description of the software/hardware/product
//! involved in the transaction.
//!
//! ## Format
//!
//! ```text
//! User-Agent: Example-SIP-Client/1.0
//! User-Agent: Example-SIP-Client/1.0 (Platform/OS Version)
//! ```
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::user_agent::UserAgent;
//! use std::str::FromStr;
//!
//! // Create a User-Agent header
//! let mut user_agent = UserAgent::new();
//! user_agent.add_product("Example-SIP-Client/1.0");
//! user_agent.add_product("Platform/OS Version");
//!
//! // Parse from a string
//! let user_agent = UserAgent::from_str("Example-SIP-Client/1.0 (Platform/OS Version)").unwrap();
//! assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
//! assert_eq!(user_agent.products()[1], "(Platform/OS Version)");
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the User-Agent header field (RFC 3261 Section 20.41).
///
/// The User-Agent header field contains information about the UAC originating the request.
/// It contains product tokens and/or comments that identify the user agent software and
/// hardware.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::user_agent::UserAgent;
/// use std::str::FromStr;
///
/// // Create a User-Agent header
/// let mut user_agent = UserAgent::new();
/// user_agent.add_product("Example-SIP-Client/1.0");
/// user_agent.add_product("(Platform/OS Version)");
///
/// // Convert to a string
/// assert_eq!(user_agent.to_string(), "Example-SIP-Client/1.0 (Platform/OS Version)");
///
/// // Parse from a string
/// let user_agent = UserAgent::from_str("Example-SIP-Client/1.0 (Platform/OS Version)").unwrap();
/// assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
/// assert_eq!(user_agent.products()[1], "(Platform/OS Version)");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAgent {
    /// List of product tokens and comments
    products: Vec<String>,
}

impl UserAgent {
    /// Creates a new empty User-Agent header.
    ///
    /// # Returns
    ///
    /// A new empty `UserAgent` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::user_agent::UserAgent;
    ///
    /// let user_agent = UserAgent::new();
    /// assert!(user_agent.products().is_empty());
    /// ```
    pub fn new() -> Self {
        UserAgent {
            products: Vec::new(),
        }
    }

    /// Creates a User-Agent header with a single product token.
    ///
    /// # Parameters
    ///
    /// - `product`: The product token to include
    ///
    /// # Returns
    ///
    /// A new `UserAgent` instance with the specified product token
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::user_agent::UserAgent;
    ///
    /// let user_agent = UserAgent::single("Example-SIP-Client/1.0");
    /// assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
    /// ```
    pub fn single(product: &str) -> Self {
        UserAgent {
            products: vec![product.to_string()],
        }
    }

    /// Creates a User-Agent header with multiple product tokens.
    ///
    /// # Parameters
    ///
    /// - `products`: A slice of product tokens to include
    ///
    /// # Returns
    ///
    /// A new `UserAgent` instance with the specified product tokens
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::user_agent::UserAgent;
    ///
    /// let user_agent = UserAgent::with_products(&[
    ///     "Example-SIP-Client/1.0",
    ///     "(Platform/OS Version)"
    /// ]);
    /// assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
    /// assert_eq!(user_agent.products()[1], "(Platform/OS Version)");
    /// ```
    pub fn with_products<T: AsRef<str>>(products: &[T]) -> Self {
        UserAgent {
            products: products.iter().map(|p| p.as_ref().to_string()).collect(),
        }
    }

    /// Adds a product token to the list.
    ///
    /// # Parameters
    ///
    /// - `product`: The product token to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::user_agent::UserAgent;
    ///
    /// let mut user_agent = UserAgent::new();
    /// user_agent.add_product("Example-SIP-Client/1.0");
    /// assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
    /// ```
    pub fn add_product(&mut self, product: &str) {
        self.products.push(product.to_string());
    }

    /// Returns the list of product tokens.
    ///
    /// # Returns
    ///
    /// A slice containing all product tokens in this header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::user_agent::UserAgent;
    ///
    /// let user_agent = UserAgent::with_products(&[
    ///     "Example-SIP-Client/1.0",
    ///     "(Platform/OS Version)"
    /// ]);
    /// let products = user_agent.products();
    /// assert_eq!(products.len(), 2);
    /// assert_eq!(products[0], "Example-SIP-Client/1.0");
    /// assert_eq!(products[1], "(Platform/OS Version)");
    /// ```
    pub fn products(&self) -> &[String] {
        &self.products
    }

    /// Checks if the list is empty.
    ///
    /// # Returns
    ///
    /// `true` if the list contains no product tokens, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::user_agent::UserAgent;
    ///
    /// let user_agent = UserAgent::new();
    /// assert!(user_agent.is_empty());
    ///
    /// let user_agent = UserAgent::single("Example-SIP-Client/1.0");
    /// assert!(!user_agent.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.products.is_empty()
    }
}

impl fmt::Display for UserAgent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.products.join(" "))
    }
}

impl Default for UserAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl FromStr for UserAgent {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header value without the name
        let value_str = if s.contains(':') {
            // Strip the "User-Agent:" prefix
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(Error::ParseError("Invalid User-Agent header format".to_string()));
            }
            parts[1].trim()
        } else {
            s.trim()
        };
        
        // Empty string is a valid User-Agent (means no agent information)
        if value_str.is_empty() {
            return Ok(UserAgent::new());
        }
        
        // Parse product tokens (with special handling for tokens in parentheses)
        let mut products = Vec::new();
        let mut current_product = String::new();
        let mut in_parentheses = false;
        
        for ch in value_str.chars() {
            match ch {
                '(' => {
                    if in_parentheses {
                        // Nested parentheses, add to current product
                        current_product.push(ch);
                    } else {
                        // Start of parenthesized comment
                        if !current_product.is_empty() {
                            current_product = current_product.trim().to_string();
                            if !current_product.is_empty() {
                                products.push(current_product);
                            }
                            current_product = String::new();
                        }
                        in_parentheses = true;
                        current_product.push(ch);
                    }
                },
                ')' => {
                    current_product.push(ch);
                    if in_parentheses {
                        // End of parenthesized comment
                        in_parentheses = false;
                        if !current_product.is_empty() {
                            products.push(current_product);
                            current_product = String::new();
                        }
                    }
                },
                ' ' => {
                    if in_parentheses {
                        // Space inside parentheses, keep it
                        current_product.push(ch);
                    } else {
                        // Space outside parentheses, token delimiter
                        if !current_product.is_empty() {
                            current_product = current_product.trim().to_string();
                            if !current_product.is_empty() {
                                products.push(current_product);
                            }
                            current_product = String::new();
                        }
                    }
                },
                _ => {
                    // Add to current product
                    current_product.push(ch);
                }
            }
        }
        
        // Add the last product, if any
        if !current_product.is_empty() {
            current_product = current_product.trim().to_string();
            if !current_product.is_empty() {
                products.push(current_product);
            }
        }
            
        Ok(UserAgent { products })
    }
}

// Implement TypedHeaderTrait for UserAgent
impl TypedHeaderTrait for UserAgent {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::UserAgent
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.to_string().into_bytes()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    UserAgent::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::UserAgent(server_vals) => {
                // Convert the complex server values into strings
                let mut products = Vec::new();
                
                for val in server_vals {
                    let (product_opt, comment_opt) = val;
                    
                    // Handle product if present
                    if let Some(product) = product_opt {
                        let (name_bytes, version_opt) = product;
                        if let Ok(name) = std::str::from_utf8(name_bytes) {
                            let mut product_str = name.to_string();
                            
                            // Add version if present
                            if let Some(version_bytes) = version_opt {
                                if let Ok(version) = std::str::from_utf8(version_bytes) {
                                    product_str.push('/');
                                    product_str.push_str(version);
                                }
                            }
                            
                            products.push(product_str);
                        }
                    }
                    
                    // Handle comment if present
                    if let Some(comment_bytes) = comment_opt {
                        if let Ok(comment) = std::str::from_utf8(comment_bytes) {
                            products.push(format!("({})", comment));
                        }
                    }
                }
                
                Ok(UserAgent { products })
            },
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let user_agent = UserAgent::new();
        assert!(user_agent.is_empty());
        assert_eq!(user_agent.to_string(), "");
    }
    
    #[test]
    fn test_single() {
        let user_agent = UserAgent::single("Example-SIP-Client/1.0");
        assert_eq!(user_agent.products().len(), 1);
        assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
        assert_eq!(user_agent.to_string(), "Example-SIP-Client/1.0");
    }
    
    #[test]
    fn test_with_products() {
        let user_agent = UserAgent::with_products(&[
            "Example-SIP-Client/1.0",
            "(Platform/OS Version)"
        ]);
        assert_eq!(user_agent.products().len(), 2);
        assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
        assert_eq!(user_agent.products()[1], "(Platform/OS Version)");
        assert_eq!(user_agent.to_string(), "Example-SIP-Client/1.0 (Platform/OS Version)");
    }
    
    #[test]
    fn test_add_product() {
        let mut user_agent = UserAgent::new();
        
        // Add products
        user_agent.add_product("Example-SIP-Client/1.0");
        user_agent.add_product("(Platform/OS Version)");
        
        assert_eq!(user_agent.products().len(), 2);
        assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
        assert_eq!(user_agent.products()[1], "(Platform/OS Version)");
    }
    
    #[test]
    fn test_from_str() {
        // Simple case
        let user_agent: UserAgent = "Example-SIP-Client/1.0".parse().unwrap();
        assert_eq!(user_agent.products().len(), 1);
        assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
        
        // Multiple products
        let user_agent: UserAgent = "Example-SIP-Client/1.0 (Platform/OS Version)".parse().unwrap();
        assert_eq!(user_agent.products().len(), 2);
        assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
        assert_eq!(user_agent.products()[1], "(Platform/OS Version)");
        
        // With header name
        let user_agent: UserAgent = "User-Agent: Example-SIP-Client/1.0 (Platform/OS Version)".parse().unwrap();
        assert_eq!(user_agent.products().len(), 2);
        assert_eq!(user_agent.products()[0], "Example-SIP-Client/1.0");
        assert_eq!(user_agent.products()[1], "(Platform/OS Version)");
        
        // Complex with multiple products and comments
        let user_agent: UserAgent = "ExamplePhone/2.0 ExampleBrowser/1.5 (OS/2.6) ExampleLib/1.1".parse().unwrap();
        assert_eq!(user_agent.products().len(), 4);
        assert_eq!(user_agent.products()[0], "ExamplePhone/2.0");
        assert_eq!(user_agent.products()[1], "ExampleBrowser/1.5");
        assert_eq!(user_agent.products()[2], "(OS/2.6)");
        assert_eq!(user_agent.products()[3], "ExampleLib/1.1");
        
        // Empty
        let user_agent: UserAgent = "".parse().unwrap();
        assert!(user_agent.is_empty());
        
        // Empty with header name
        let user_agent: UserAgent = "User-Agent:".parse().unwrap();
        assert!(user_agent.is_empty());
    }
    
    #[test]
    fn test_typed_header_trait() {
        // Create a header
        let user_agent = UserAgent::with_products(&[
            "Example-SIP-Client/1.0",
            "(Platform/OS Version)"
        ]);
        let header = user_agent.to_header();
        
        assert_eq!(header.name, HeaderName::UserAgent);
        
        // Convert back from Header
        let user_agent2 = UserAgent::from_header(&header).unwrap();
        assert_eq!(user_agent.products().len(), user_agent2.products().len());
        assert_eq!(user_agent.products()[0], user_agent2.products()[0]);
        assert_eq!(user_agent.products()[1], user_agent2.products()[1]);
        
        // Test invalid header name
        let wrong_header = Header::text(HeaderName::ContentType, "text/plain");
        assert!(UserAgent::from_header(&wrong_header).is_err());
    }
} 