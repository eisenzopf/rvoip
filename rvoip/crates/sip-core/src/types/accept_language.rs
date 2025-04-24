use crate::parser::headers::accept_language::{LanguageInfo, parse_accept_language};
use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Deserialize, Serialize};

/// Represents the Accept-Language header field (RFC 3261 Section 20.3).
/// Indicates the preferred languages for the response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptLanguage(pub Vec<LanguageInfo>);

impl AcceptLanguage {
    /// Creates an empty Accept-Language header.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Creates an Accept-Language header with specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Creates an Accept-Language header from an iterator of language info items.
    pub fn from_languages<I>(languages: I) -> Self
    where
        I: IntoIterator<Item = LanguageInfo>
    {
        Self(languages.into_iter().collect())
    }

    /// Adds a language to the list.
    pub fn push(&mut self, language: LanguageInfo) {
        self.0.push(language);
    }

    /// Returns the list of languages in this header.
    pub fn languages(&self) -> &[LanguageInfo] {
        &self.0
    }

    /// Checks if a specific language is acceptable.
    /// Performs a basic language tag match, respecting wildcards.
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
    /// Returns None if no language is acceptable.
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