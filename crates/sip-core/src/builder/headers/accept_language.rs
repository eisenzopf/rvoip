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

/// Accept-Language Header Builder for SIP Messages
///
/// This module provides builder methods for the Accept-Language header in SIP messages,
/// which indicates the natural languages that the User Agent prefers for message content.
///
/// ## SIP Accept-Language Header Overview
///
/// The Accept-Language header is defined in [RFC 3261 Section 20.3](https://datatracker.ietf.org/doc/html/rfc3261#section-20.3)
/// as part of the core SIP protocol. It follows the syntax and semantics defined in 
/// [RFC 2616 Section 14.4](https://datatracker.ietf.org/doc/html/rfc2616#section-14.4) for HTTP,
/// with language tags conforming to [RFC 5646](https://datatracker.ietf.org/doc/html/rfc5646) (BCP 47).
///
/// ## Purpose of Accept-Language Header
///
/// The Accept-Language header serves several important purposes in SIP:
///
/// 1. It indicates which natural languages the User Agent prefers for content
/// 2. It enables language negotiation between UAs from different regions
/// 3. It provides a mechanism to express language preferences via quality values (q-values)
/// 4. It helps servers deliver localized content based on user preferences
///
/// ## Common Language Tags in SIP
///
/// Language tags follow [BCP 47](https://www.rfc-editor.org/info/bcp47) format. Common examples include:
///
/// - **en**: English
/// - **en-US**: American English
/// - **en-GB**: British English
/// - **fr**: French
/// - **fr-CA**: Canadian French
/// - **es**: Spanish
/// - **zh-Hans**: Simplified Chinese
/// - **zh-Hant**: Traditional Chinese
/// - **de**: German
/// - **ja**: Japanese
/// - **ru**: Russian
/// - **pt-BR**: Brazilian Portuguese
///
/// ## Quality Values (q-values)
///
/// The Accept-Language header can include quality values (q-values) to indicate preference order:
///
/// - Values range from 0.0 to 1.0, with 1.0 being the highest priority
/// - Default value is 1.0 when not specified
/// - Multiple languages can have the same q-value, indicating equal preference
/// - A q-value of 0.0 explicitly indicates that the language is not acceptable
///
/// ## Special Considerations
///
/// 1. **Language Matching**: Servers should favor exact matches over partial matches
/// 2. **Language Fallbacks**: When no exact match is available, fallback to less specific tags
/// 3. **Default Behavior**: If no Accept-Language header is present, the server should use a default language
/// 4. **Multiple Headers**: The Accept-Language header can appear multiple times in a request
///
/// ## Relationship with other headers
///
/// - **Accept-Language** + **Content-Language**: Accept-Language specifies preferences, Content-Language specifies what is being sent
/// - **Accept-Language** vs **Accept**: Accept is for content types, Accept-Language is for natural languages
/// - **Accept-Language** + **User-Agent**: Together can give context about the client's regional settings
/// - **Accept-Language** in **Contact** headers: Can indicate language capabilities at registration time
///
/// # Examples
///
/// ## Basic Usage with Language Preference
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptLanguageExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with language preference
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:service@example.com").unwrap()
///     .accept_language("en-US", None)  // Prefer American English with default priority
///     .build();
/// ```
///
/// ## Multiple Languages with Priorities
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptLanguageExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with multiple language preferences
/// let languages = vec![
///     ("fr-CA", Some(1.0)),        // Canadian French (highest priority)
///     ("en-CA", Some(0.8)),        // Canadian English (second priority)
///     ("fr", Some(0.6)),           // Generic French (third priority)
///     ("en", Some(0.4)),           // Generic English (fourth priority)
/// ];
///
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
///     .accept_languages(languages)
///     .build();
/// ```
///
/// ## International Customer Support
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptLanguageExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request to contact a multilingual service
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:support@example.com").unwrap()
///     .accept_language("es", Some(1.0))       // Spanish (highest priority)
///     .accept_language("en", Some(0.5))       // English (fallback)
///     .build();
/// ```
///
/// ## When to use Accept-Language Headers
///
/// Accept-Language headers are particularly useful in the following scenarios:
///
/// 1. **Multilingual environments**: When communicating with services in multiple countries
/// 2. **Call centers and IVR systems**: To receive prompts in preferred languages
/// 3. **International services**: For global SIP deployments serving diverse user bases
/// 4. **Message content negotiation**: To receive text content in languages the user understands
/// 5. **User-level preferences**: To preserve individual language settings across sessions
///
/// ## Best Practices
///
/// - Use standard [BCP 47](https://www.rfc-editor.org/info/bcp47) language tags
/// - Include region subtags when language variation matters (e.g., "en-US" vs "en-GB")
/// - Order languages by preference using q-values
/// - Include more generic language tags as fallbacks (e.g., "en" after "en-US")
/// - Consider sending at least one widely supported language as a low-priority fallback
///
/// # Examples
///
/// ## Multilingual Call Center Routing
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptLanguageExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create an INVITE to a call center with language preferences for routing
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:customer-service@example.com").unwrap()
///     .from("Customer", "sip:customer@example.net", Some("tag1234"))
///     .to("Customer Service", "sip:customer-service@example.com", None)
///     // First try to get a German-speaking agent
///     .accept_language("de", Some(1.0))
///     // Can also understand Swiss German as second choice
///     .accept_language("de-CH", Some(0.9))
///     // English as fallback option
///     .accept_language("en", Some(0.7))
///     .build();
/// ```
///
/// ## Regional Content Delivery
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::AcceptLanguageExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a MESSAGE request to a news service with regional preferences
/// let languages = vec![
///     ("zh-Hans", Some(1.0)),      // Simplified Chinese
///     ("zh-Hant", Some(0.8)),      // Traditional Chinese as fallback
///     ("en", Some(0.5)),           // English as last resort
/// ];
///
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:news-service@example.com").unwrap()
///     .from("User", "sip:user@example.net", Some("msg123"))
///     .to("News Service", "sip:news-service@example.com", None)
///     .accept_languages(languages)
///     .body("Please send the latest headlines")
///     .build();
/// ```
pub trait AcceptLanguageExt {
    /// Add an Accept-Language header with a single language
    ///
    /// This method specifies a single language that the UA prefers for content,
    /// optionally with a quality value (q-value) to indicate preference when
    /// multiple Accept-Language headers are present.
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
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a request that indicates language preference
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:conference@example.com").unwrap()
    ///     .from("Participant", "sip:user@example.net", Some("call123"))
    ///     .to("Conference", "sip:conference@example.com", None)
    ///     .accept_language("ja", Some(0.8))  // Prefer Japanese content with high priority
    ///     .build();
    /// ```
    ///
    /// # RFC Reference
    /// 
    /// As per [RFC 3261 Section 20.3](https://datatracker.ietf.org/doc/html/rfc3261#section-20.3),
    /// the Accept-Language header field follows the syntax defined in 
    /// [RFC 2616 Section 14.4](https://datatracker.ietf.org/doc/html/rfc2616#section-14.4),
    /// with language tags as defined in [RFC 5646](https://datatracker.ietf.org/doc/html/rfc5646).
    fn accept_language(self, language: &str, q: Option<f32>) -> Self;

    /// Add an Accept-Language header with multiple languages
    ///
    /// This method specifies multiple languages that the UA prefers for content,
    /// each with an optional quality value to indicate preference order.
    /// This is more efficient than adding multiple individual Accept-Language headers.
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
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a request with comprehensive language preferences
    /// let languages = vec![
    ///     ("fr-CA", Some(1.0)),        // Canadian French (highest priority)
    ///     ("en-CA", Some(0.9)),        // Canadian English (high priority)
    ///     ("fr", Some(0.7)),           // Generic French (medium priority)
    ///     ("en", Some(0.6)),           // Generic English (medium-low priority)
    ///     ("es", Some(0.3)),           // Spanish (low priority)
    /// ];
    /// 
    /// let request = SimpleRequestBuilder::new(Method::Options, "sip:service@example.com").unwrap()
    ///     .from("User", "sip:user@example.net", Some("options321"))
    ///     .to("Service", "sip:service@example.com", None)
    ///     .accept_languages(languages)  // Set all language preferences at once
    ///     .build();
    /// ```
    ///
    /// # Language Tag Format
    ///
    /// Language tags follow [BCP 47](https://www.rfc-editor.org/info/bcp47) format, which includes:
    ///
    /// - **Language**: Two or three letter ISO language code (e.g., "en", "fr")
    /// - **Region/Country**: Optional subtag separated by hyphen (e.g., "en-US", "fr-CA")
    /// - **Script**: Optional script subtag (e.g., "zh-Hans", "zh-Hant")
    /// - **Variant**: Optional dialect or variant (e.g., "de-CH-1901")
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