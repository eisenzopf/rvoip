use crate::error::{Error, Result};
use crate::types::{
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use crate::types::content_language::ContentLanguage;
use super::HeaderSetter;

/// Extension trait for adding Content-Language header building capabilities
pub trait ContentLanguageExt {
    /// Add a Content-Language header with a single language
    ///
    /// # Arguments
    ///
    /// * `language` - The language tag to specify (e.g., "en", "en-US")
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentLanguageExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .content_language("en-US")
    ///     .build();
    /// ```
    fn content_language(self, language: &str) -> Self;

    /// Add a Content-Language header with multiple languages
    ///
    /// # Arguments
    ///
    /// * `languages` - A slice of language tags to specify
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentLanguageExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .content_languages(&["en-US", "fr-CA"])
    ///     .build();
    /// ```
    fn content_languages<T: AsRef<str>>(self, languages: &[T]) -> Self;
}

impl<T> ContentLanguageExt for T 
where 
    T: HeaderSetter,
{
    fn content_language(self, language: &str) -> Self {
        let header_value = ContentLanguage::single(language);
        self.set_header(header_value)
    }

    fn content_languages<S: AsRef<str>>(self, languages: &[S]) -> Self {
        let header_value = ContentLanguage::with_languages(languages);
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::header::HeaderName;
    use crate::types::ContentLanguage; // Import the actual type
    
    #[test]
    fn test_content_language_single() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_language("en-US")
            .build();
            
        // Check if Content-Language header exists with the correct value
        let header = request.header(&HeaderName::ContentLanguage);
        assert!(header.is_some(), "Content-Language header not found");
        
        if let Some(TypedHeader::ContentLanguage(content_language)) = header {
            // Check if the content language includes "en-US"
            assert!(content_language.has_language("en-US"), "en-US language not found");
            assert_eq!(content_language.languages().len(), 1);
        } else {
            panic!("Expected Content-Language header");
        }
    }
    
    #[test]
    fn test_content_languages_multiple() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_languages(&["en-US", "fr-CA"])
            .build();
            
        // Check if Content-Language header exists with the correct values
        let header = request.header(&HeaderName::ContentLanguage);
        assert!(header.is_some(), "Content-Language header not found");
        
        if let Some(TypedHeader::ContentLanguage(content_language)) = header {
            // Check if the content language includes both "en-US" and "fr-CA"
            assert!(content_language.has_language("en-US"), "en-US language not found");
            assert!(content_language.has_language("fr-CA"), "fr-CA language not found");
            assert_eq!(content_language.languages().len(), 2);
        } else {
            panic!("Expected Content-Language header");
        }
    }
} 