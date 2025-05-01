//! # SIP Accept-Language Header
//! 
//! This module provides an implementation of the SIP Accept-Language header field as defined in
//! [RFC 3261 Section 20.3](https://datatracker.ietf.org/doc/html/rfc3261#section-20.3).
//!
//! The Accept-Language header field is used to indicate the preferred languages for reason phrases,
//! session descriptions, or status responses carried as message bodies in the response. The syntax
//! and semantics are identical to HTTP Accept-Language header field as defined in
//! [RFC 2616 Section 14.4](https://datatracker.ietf.org/doc/html/rfc2616#section-14.4).
//!
//! If the Accept-Language header field is not present, the server SHOULD assume all languages are
//! acceptable to the client.
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::types::AcceptLanguage;
//! use std::str::FromStr;
//!
//! // Parse an Accept-Language header
//! let header = AcceptLanguage::from_str("en-US;q=0.8, fr;q=1.0, de;q=0.7").unwrap();
//!
//! // Check if a language is acceptable
//! assert!(header.accepts("fr"));
//! assert!(header.accepts("en-us"));
//!
//! // Find the best match from available languages
//! let available = vec!["es", "de", "en-us"];
//! assert_eq!(header.best_match(available), Some("en-us"));
//!
//! // Format as a string (ordered by q-value)
//! assert_eq!(header.to_string(), "fr;q=1.000, en-us;q=0.800, de;q=0.700");
//! ```

use crate::parser::headers::accept_language::{LanguageInfo, parse_accept_language};
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Accept-Language header field (RFC 3261 Section 20.3).
///
/// The Accept-Language header indicates the preferred languages for message content in responses.
/// It contains a prioritized list of language tags, each potentially with a quality value ("q-value")
/// that indicates its relative preference (from 0.0 to 1.0, with 1.0 being the default and highest priority).
///
/// As per RFC 3261, if this header is not present in a request, the server should assume all languages 
/// are acceptable to the client.
///
/// # Language matching
///
/// This implementation follows the language matching rules outlined in RFC 2616:
///
/// - Languages are matched case-insensitively
/// - A wildcard (`*`) matches any language
/// - Languages with higher q-values are preferred over languages with lower q-values
/// - Languages with the same q-value are ordered by their original order in the header
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::AcceptLanguage;
/// use std::str::FromStr;
///
/// // Create from a header string
/// let header = AcceptLanguage::from_str("Accept-Language: en-US;q=0.8, fr, de;q=0.7").unwrap();
///
/// // Or just from the header value part
/// let header = AcceptLanguage::from_str("en-US;q=0.8, fr, de;q=0.7").unwrap();
///
/// // Create programmatically
/// use rvoip_sip_core::parser::headers::accept_language::LanguageInfo;
/// use ordered_float::NotNan;
///
/// let en = LanguageInfo {
///     range: "en".to_string(),
///     q: Some(NotNan::new(0.8).unwrap()),
///     params: vec![],
/// };
///
/// let fr = LanguageInfo {
///     range: "fr".to_string(),
///     q: None, // Default q-value is 1.0
///     params: vec![],
/// };
///
/// let header = AcceptLanguage::from_languages(vec![en, fr]);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptLanguage(pub Vec<LanguageInfo>);

impl AcceptLanguage {
    /// Creates an empty Accept-Language header.
    ///
    /// An empty Accept-Language header means all languages are acceptable,
    /// according to RFC 3261 Section 20.3.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptLanguage;
    ///
    /// let header = AcceptLanguage::new();
    /// assert!(header.accepts("any-language"));
    /// ```
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Creates an Accept-Language header with specified capacity.
    ///
    /// This is useful when you know approximately how many languages
    /// you'll be adding to avoid reallocations.
    ///
    /// # Parameters
    ///
    /// - `capacity`: The initial capacity for the languages vector
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptLanguage;
    ///
    /// let mut header = AcceptLanguage::with_capacity(3);
    /// // Can now add up to 3 languages without reallocation
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Creates an Accept-Language header from an iterator of language info items.
    ///
    /// # Parameters
    ///
    /// - `languages`: An iterator yielding `LanguageInfo` items
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptLanguage;
    /// use rvoip_sip_core::parser::headers::accept_language::LanguageInfo;
    /// use ordered_float::NotNan;
    ///
    /// let en = LanguageInfo {
    ///     range: "en".to_string(),
    ///     q: Some(NotNan::new(0.8).unwrap()),
    ///     params: vec![],
    /// };
    ///
    /// let fr = LanguageInfo {
    ///     range: "fr".to_string(),
    ///     q: None, // Default q-value is 1.0
    ///     params: vec![],
    /// };
    ///
    /// let header = AcceptLanguage::from_languages(vec![en, fr]);
    /// assert_eq!(header.languages().len(), 2);
    /// ```
    pub fn from_languages<I>(languages: I) -> Self
    where
        I: IntoIterator<Item = LanguageInfo>
    {
        Self(languages.into_iter().collect())
    }

    /// Adds a language to the list.
    ///
    /// # Parameters
    ///
    /// - `language`: The language info to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptLanguage;
    /// use rvoip_sip_core::parser::headers::accept_language::LanguageInfo;
    ///
    /// let mut header = AcceptLanguage::new();
    /// let en = LanguageInfo {
    ///     range: "en".to_string(),
    ///     q: None,
    ///     params: vec![],
    /// };
    /// header.push(en);
    /// assert_eq!(header.languages().len(), 1);
    /// ```
    pub fn push(&mut self, language: LanguageInfo) {
        self.0.push(language);
    }

    /// Returns the list of languages in this header.
    ///
    /// # Returns
    ///
    /// A slice containing all language info items in this header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptLanguage;
    /// use std::str::FromStr;
    ///
    /// let header = AcceptLanguage::from_str("en;q=0.8, fr").unwrap();
    /// let languages = header.languages();
    /// assert_eq!(languages.len(), 2);
    /// assert_eq!(languages[0].range, "fr"); // Sorted by q-value
    /// assert_eq!(languages[1].range, "en");
    /// ```
    pub fn languages(&self) -> &[LanguageInfo] {
        &self.0
    }

    /// Checks if a specific language is acceptable.
    ///
    /// Performs a basic language tag match, respecting wildcards and case-insensitivity.
    /// According to RFC 3261, if the header is empty, all languages are acceptable.
    ///
    /// # Parameters
    ///
    /// - `language_tag`: The language tag to check (e.g., "en", "en-US", "fr")
    ///
    /// # Returns
    ///
    /// `true` if the language is acceptable, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptLanguage;
    /// use std::str::FromStr;
    ///
    /// let header = AcceptLanguage::from_str("en-US;q=0.8, fr, *;q=0.1").unwrap();
    ///
    /// assert!(header.accepts("en-us"));  // Case-insensitive
    /// assert!(header.accepts("fr"));
    /// assert!(header.accepts("de"));     // Matched by wildcard
    /// ```
    pub fn accepts(&self, language_tag: &str) -> bool {
        // If empty, accept everything (RFC 3261 Section 20.3)
        if self.0.is_empty() {
            return true;
        }

        // Check if language_tag matches any of the accepted language tags
        self.0.iter().any(|lang_info| {
            lang_info.range == "*" || lang_info.language_equals(language_tag)
        })
    }

    /// Find the best acceptable language from a list of available languages.
    ///
    /// This method determines the most preferred language based on q-values and
    /// the order of languages in the header.
    ///
    /// # Parameters
    ///
    /// - `available_languages`: An iterator yielding available language tags
    ///
    /// # Returns
    ///
    /// The best matching language if any is acceptable, or `None` if no language matches
    ///
    /// # Algorithm
    ///
    /// 1. If the AcceptLanguage header is empty, all languages are acceptable and
    ///    the first available language is returned
    /// 2. Languages are compared by their q-values, with higher values preferred
    /// 3. When q-values are equal, the order in the header determines preference
    /// 4. Wildcards match any language but are only used if no specific match is found
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::types::AcceptLanguage;
    /// use std::str::FromStr;
    ///
    /// let header = AcceptLanguage::from_str("en-US;q=0.8, fr;q=1.0, de;q=0.5").unwrap();
    ///
    /// // Available languages
    /// let available = vec!["es", "de", "en-us"];
    ///
    /// // The best match is "en-us" (higher q-value than "de")
    /// assert_eq!(header.best_match(available), Some("en-us"));
    ///
    /// // If "fr" is available, it will be preferred (highest q-value)
    /// let available = vec!["fr", "en-us", "de"];
    /// assert_eq!(header.best_match(available), Some("fr"));
    /// ```
    pub fn best_match<'a, I>(&self, available_languages: I) -> Option<&'a str>
    where
        I: IntoIterator<Item = &'a str>
    {
        // If no languages specified in Accept-Language, any language is acceptable
        if self.0.is_empty() {
            return available_languages.into_iter().next();
        }

        let available: Vec<&str> = available_languages.into_iter().collect();
        
        // Make sure languages are sorted by q-value (highest first)
        let mut sorted_languages = self.0.clone();
        sorted_languages.sort();
        
        // First check exact matches in q-value order (highest first)
        for lang_info in &sorted_languages {
            // Skip wildcards in this pass
            if lang_info.range == "*" {
                continue;
            }
            
            for available_lang in &available {
                if lang_info.language_equals(available_lang) {
                    return Some(available_lang);
                }
            }
        }
        
        // Then check for wildcard match
        if let Some(wildcard) = sorted_languages.iter().find(|lang| lang.range == "*") {
            // If there's a wildcard and it has a non-zero q-value, accept the first available language
            if wildcard.q_value() > 0.0 {
                return available.first().copied();
            }
        }
        
        None
    }
}

impl fmt::Display for AcceptLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Create a sorted copy of the languages
        let mut sorted_languages = self.0.clone();
        sorted_languages.sort();
        
        let language_strings: Vec<String> = sorted_languages.iter().map(|lang| lang.to_string()).collect();
        write!(f, "{}", language_strings.join(", "))
    }
}

// Helper function to parse from owned bytes
fn parse_from_owned_bytes(bytes: Vec<u8>) -> Result<Vec<LanguageInfo>> {
    match all_consuming(parse_accept_language)(bytes.as_slice()) {
        Ok((_, languages)) => Ok(languages),
        Err(e) => Err(Error::ParseError(
            format!("Failed to parse Accept-Language header: {:?}", e)
        ))
    }
}

impl FromStr for AcceptLanguage {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Handle the case of just the header without the name
        // (e.g., "en-US;q=0.8, fr" instead of "Accept-Language: en-US;q=0.8, fr")
        let input_bytes = if !s.contains(':') {
            format!("Accept-Language: {}", s).into_bytes()
        } else {
            s.as_bytes().to_vec()
        };
        
        // Parse using our helper function that takes ownership of the bytes
        parse_from_owned_bytes(input_bytes).map(AcceptLanguage)
    }
}

// Implement TypedHeaderTrait for AcceptLanguage
impl TypedHeaderTrait for AcceptLanguage {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::AcceptLanguage
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
                    AcceptLanguage::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
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
    use ordered_float::NotNan;
    use crate::types::param::Param;

    #[test]
    fn test_from_str() {
        // Test with header name
        let header_str = "Accept-Language: en-US;q=0.8, fr, de;q=0.7";
        let accept_lang: AcceptLanguage = header_str.parse().unwrap();
        
        assert_eq!(accept_lang.0.len(), 3);
        assert_eq!(accept_lang.0[0].range, "fr");  // Highest q-value first (1.0 implicit)
        assert_eq!(accept_lang.0[1].range, "en-us");  // Should be lowercase
        assert_eq!(accept_lang.0[2].range, "de");
        
        // Test without header name
        let value_str = "en;q=0.5, fr;q=0.8, *;q=0.1";
        let accept_lang2: AcceptLanguage = value_str.parse().unwrap();
        
        assert_eq!(accept_lang2.0.len(), 3);
        assert_eq!(accept_lang2.0[0].range, "fr");  // q=0.8
        assert_eq!(accept_lang2.0[1].range, "en");  // q=0.5
        assert_eq!(accept_lang2.0[2].range, "*");   // q=0.1
    }
    
    #[test]
    fn test_accepts() {
        // Create test languages
        let mut en = LanguageInfo {
            range: "en".to_string(),
            q: Some(NotNan::new(0.8).unwrap()),
            params: vec![],
        };
        
        let fr = LanguageInfo {
            range: "fr".to_string(),
            q: None, // Default 1.0
            params: vec![],
        };
        
        let wildcard = LanguageInfo {
            range: "*".to_string(),
            q: Some(NotNan::new(0.1).unwrap()),
            params: vec![],
        };
        
        // Test with languages
        let accept_lang = AcceptLanguage(vec![en.clone(), fr.clone()]);
        
        assert!(accept_lang.accepts("en"), "Should accept exact match");
        assert!(accept_lang.accepts("fr"), "Should accept exact match");
        assert!(!accept_lang.accepts("de"), "Should not accept non-matching language");
        
        // Test with wildcard
        let accept_lang_wildcard = AcceptLanguage(vec![en.clone(), wildcard]);
        
        assert!(accept_lang_wildcard.accepts("de"), "Should accept any language with wildcard");
        
        // Test empty Accept-Language (should accept everything per RFC)
        let empty_accept_lang = AcceptLanguage::new();
        assert!(empty_accept_lang.accepts("any-language"), "Empty Accept-Language should accept any language");
    }
    
    #[test]
    fn test_best_match() {
        // Create test languages
        let en = LanguageInfo {
            range: "en".to_string(),
            q: Some(NotNan::new(0.8).unwrap()),
            params: vec![],
        };
        
        let fr = LanguageInfo {
            range: "fr".to_string(),
            q: None, // Default 1.0
            params: vec![],
        };
        
        let de = LanguageInfo {
            range: "de".to_string(),
            q: Some(NotNan::new(0.5).unwrap()),
            params: vec![],
        };
        
        let wildcard = LanguageInfo {
            range: "*".to_string(),
            q: Some(NotNan::new(0.1).unwrap()),
            params: vec![],
        };
        
        // Test exact matches
        let accept_lang = AcceptLanguage(vec![en.clone(), fr.clone(), de.clone()]);
        let available = vec!["es", "de", "it"];
        
        assert_eq!(accept_lang.best_match(available), Some("de"), 
                  "Should choose the available language with highest q-value");
        
        // Test wildcard fallback
        let accept_lang_wildcard = AcceptLanguage(vec![en.clone(), wildcard]);
        let available_no_match = vec!["es", "it"];
        
        assert_eq!(accept_lang_wildcard.best_match(available_no_match), Some("es"), 
                  "Should fall back to wildcard and choose first available");
        
        // Test empty available languages
        assert_eq!(accept_lang.best_match(Vec::<&str>::new()), None, 
                  "Should return None when no languages are available");
        
        // Test empty Accept-Language header
        let empty_accept_lang = AcceptLanguage::new();
        let available_langs = vec!["en", "fr", "de"];
        
        assert_eq!(empty_accept_lang.best_match(available_langs), Some("en"), 
                  "Empty Accept-Language should accept first available language");
    }
    
    #[test]
    fn test_display() {
        // Create test languages
        let en = LanguageInfo {
            range: "en".to_string(),
            q: Some(NotNan::new(0.8).unwrap()),
            params: vec![],
        };
        
        let fr = LanguageInfo {
            range: "fr".to_string(),
            q: None, // Default 1.0
            params: vec![],
        };
        
        let de = LanguageInfo {
            range: "de".to_string(),
            q: Some(NotNan::new(0.5).unwrap()),
            params: vec![Param::Other("custom".to_string(), None)],
        };
        
        // Test display
        let accept_lang = AcceptLanguage(vec![fr.clone(), en.clone(), de.clone()]);
        let display_str = accept_lang.to_string();
        
        assert!(display_str.contains("fr"), "Should contain fr language");
        assert!(display_str.contains("en;q=0.800"), "Should contain en with q-value");
        assert!(display_str.contains("de;q=0.500;custom"), "Should contain de with q-value and params");
    }
    
    #[test]
    fn test_basic_functionality() {
        // Create a simple AcceptLanguage header
        let languages = vec![
            LanguageInfo {
                range: "en".to_string(),
                q: Some(NotNan::new(0.8).unwrap()),
                params: vec![],
            },
            LanguageInfo {
                range: "fr".to_string(),
                q: None, // Default q=1.0
                params: vec![],
            },
        ];
        
        let accept_lang = AcceptLanguage(languages);
        
        // Test the display implementation - should be in order of q-value (highest first)
        assert_eq!(accept_lang.to_string(), "fr, en;q=0.800");
        
        // Test the accepts method
        assert!(accept_lang.accepts("en"));
        assert!(accept_lang.accepts("fr"));
        assert!(!accept_lang.accepts("de"));
        
        // Test the best_match method
        let available = vec!["de", "en", "fr"];
        assert_eq!(accept_lang.best_match(available), Some("fr"));
        
        // Test with only non-matched languages
        let non_matching = vec!["de", "es", "it"];
        assert_eq!(accept_lang.best_match(non_matching), None);
    }
} 