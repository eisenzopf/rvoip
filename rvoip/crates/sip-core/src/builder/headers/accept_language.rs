use crate::error::{Error, Result};
use ordered_float::NotNan;
use std::str::FromStr;
use crate::types::{
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use crate::types::accept_language::AcceptLanguage;
use crate::parser::headers::accept_language::LanguageInfo;
use crate::types::param::Param;
use super::HeaderSetter;

/// Extension trait for adding Accept-Language header building capabilities
pub trait AcceptLanguageExt {
    /// Add an Accept-Language header with a single language
    ///
    /// # Arguments
    ///
    /// * `language` - The language tag (e.g., "en", "en-US", "fr")
    /// * `q` - Optional quality value (0.0 to 1.0, where 1.0 is highest priority)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AcceptLanguageExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .accept_language("en-US", Some(0.8))
    ///     .build();
    /// ```
    fn accept_language(self, language: &str, q: Option<f32>) -> Self;

    /// Add an Accept-Language header with multiple languages
    ///
    /// # Arguments
    ///
    /// * `languages` - A vector of tuples containing (language_tag, q_value)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AcceptLanguageExt};
    /// 
    /// let languages = vec![
    ///     ("en-US", Some(0.8)),
    ///     ("fr", Some(1.0)),
    ///     ("de", Some(0.7)),
    /// ];
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .accept_languages(languages)
    ///     .build();
    /// ```
    fn accept_languages(self, languages: Vec<(&str, Option<f32>)>) -> Self;
}

impl<T> AcceptLanguageExt for T 
where 
    T: HeaderSetter,
{
    fn accept_language(
        self, 
        language: &str, 
        q: Option<f32>
    ) -> Self {
        // Create language tag
        let language_tag = language.to_string();
        
        // Create language info with optional q value
        let params = Vec::new(); // No params - q goes in the q field
        
        let language_info = LanguageInfo {
            range: language_tag,
            q: q.and_then(|v| NotNan::new(v).ok()),
            params,
        };
        
        // Create the Accept-Language header
        let header_value = AcceptLanguage(vec![language_info]);
        self.set_header(header_value)
    }
    
    fn accept_languages(
        self, 
        languages: Vec<(&str, Option<f32>)>
    ) -> Self {
        // Create language infos
        let mut language_infos = Vec::with_capacity(languages.len());
        
        for (language, q) in languages {
            // Create language tag
            let language_tag = language.to_string();
            
            // Create language info with optional q value
            let params = Vec::new(); // No params - q goes in the q field
            
            let language_info = LanguageInfo {
                range: language_tag,
                q: q.and_then(|v| NotNan::new(v).ok()),
                params,
            };
            
            language_infos.push(language_info);
        }
        
        // If no valid languages, return self unchanged
        if language_infos.is_empty() {
            return self;
        }
        
        // Create the Accept-Language header
        let header_value = AcceptLanguage(language_infos);
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::header::HeaderName;
    
    #[test]
    fn test_accept_language_single() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .accept_language("en-US", Some(0.8))
            .build();
            
        // Check if Accept-Language header exists with the correct value
        let header = request.header(&HeaderName::AcceptLanguage);
        assert!(header.is_some(), "Accept-Language header not found");
        
        if let Some(TypedHeader::AcceptLanguage(AcceptLanguage(languages))) = header {
            assert_eq!(languages.len(), 1, "Expected 1 language, got {}", languages.len());
            
            // Check language tag, case-insensitive
            let language = &languages[0].range;
            assert!(language.eq_ignore_ascii_case("en-US"), 
                   "Expected language 'en-US', got '{}'", language);
            
            // Check q parameter directly through q field
            let has_q = languages[0].q.map(|q| (q.into_inner() - 0.8).abs() < 0.00001).unwrap_or(false);
            assert!(has_q, "Q value 0.8 not found");
        } else {
            panic!("Expected Accept-Language header");
        }
    }
    
    #[test]
    fn test_accept_languages_multiple() {
        let languages = vec![
            ("en-US", Some(0.8)),
            ("fr", Some(1.0)),
            ("de", Some(0.7)),
        ];
        
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .accept_languages(languages)
            .build();
            
        // Check if Accept-Language header exists with the correct values
        let header = request.header(&HeaderName::AcceptLanguage);
        assert!(header.is_some(), "Accept-Language header not found");
        
        if let Some(TypedHeader::AcceptLanguage(AcceptLanguage(languages))) = header {
            assert_eq!(languages.len(), 3, "Expected 3 languages, got {}", languages.len());
            
            // Check for en-US with q=0.8, case-insensitive
            let has_en_us = languages.iter().any(|lang| {
                lang.range.eq_ignore_ascii_case("en-US") && 
                lang.q.map(|q| (q.into_inner() - 0.8).abs() < 0.00001).unwrap_or(false)
            });
            assert!(has_en_us, "en-US language not found with q=0.8");
            
            // Check for fr with q=1.0
            let has_fr = languages.iter().any(|lang| {
                lang.range == "fr" && 
                lang.q.map(|q| (q.into_inner() - 1.0).abs() < 0.00001).unwrap_or(false)
            });
            assert!(has_fr, "fr language not found with q=1.0");
            
            // Check for de with q=0.7
            let has_de = languages.iter().any(|lang| {
                lang.range == "de" && 
                lang.q.map(|q| (q.into_inner() - 0.7).abs() < 0.00001).unwrap_or(false)
            });
            assert!(has_de, "de language not found with q=0.7");
        } else {
            panic!("Expected Accept-Language header");
        }
    }
} 