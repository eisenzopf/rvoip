use crate::error::{Error, Result};
use crate::types::{
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use crate::types::content_language::ContentLanguage;
use super::HeaderSetter;

/// Content-Language Header Builder for SIP Messages
///
/// This module provides builder methods for the Content-Language header in SIP messages,
/// which indicates the natural language(s) of the message body.
///
/// ## SIP Content-Language Header Overview
///
/// The Content-Language header is defined in [RFC 3261 Section 20.13](https://datatracker.ietf.org/doc/html/rfc3261#section-20.13)
/// as part of the core SIP protocol. It follows the syntax and semantics defined in 
/// [RFC 2616 Section 14.12](https://datatracker.ietf.org/doc/html/rfc2616#section-14.12) for HTTP
/// and [RFC 5646](https://datatracker.ietf.org/doc/html/rfc5646) for language tags.
///
/// ## Purpose of Content-Language Header
///
/// The Content-Language header serves several important purposes in SIP:
///
/// 1. It identifies the natural language(s) of the enclosed message body
/// 2. It enables recipients to process content appropriately based on language
/// 3. It supports internationalization and localization of SIP services
/// 4. It helps multilingual UAs select the appropriate rendering or processing
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
/// - **ja**: Japanese
///
/// ## Special Considerations
///
/// 1. **Multiple Languages**: A Content-Language header can list multiple language tags when content includes multiple languages
/// 2. **Language vs Locale**: Language tags can include region subtags (e.g., "en-US") for locale-specific content
/// 3. **Script Subtags**: Some languages may include script information (e.g., "zh-Hans" vs "zh-Hant")
/// 4. **Variants**: Specific variants can be specified (e.g., "sl-nedis" for Nedis dialect of Slovenian)
///
/// ## Relationship with other headers
///
/// - **Content-Language** vs **Accept-Language**: Content-Language specifies what's in the message, while Accept-Language indicates preference for responses
/// - **Content-Language** + **Content-Type**: Content-Type identifies format, Content-Language identifies natural language
/// - **Content-Language** + **Content-Disposition**: Work together for multilingual attachment handling
///
/// # Examples
///
/// ## Basic Usage with Message Text
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentLanguageExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a MESSAGE with Spanish text content
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:recipient@example.com").unwrap()
///     .content_type_text()
///     .content_language("es")  // Specifies that the content is in Spanish
///     .body("Hola, ¿cómo estás? Este es un mensaje de texto en español.")
///     .build();
/// ```
///
/// ## Multiple Languages in Content
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentLanguageExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a MESSAGE with bilingual content (English and French)
/// let bilingual_message = format!(
///     "This message contains both English and French content.\n\n\
///      Ce message contient du contenu en anglais et en français."
/// );
///     
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:support@example.com").unwrap()
///     .content_type_text()
///     .content_languages(&["en", "fr"])  // Message contains both languages
///     .body(bilingual_message)
///     .build();
/// ```
///
/// ## Internationalized Error Message
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::{ContentLanguageExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::StatusCode;
///
/// // Create a 486 Busy Here response with a localized message in Japanese
/// let response = SimpleResponseBuilder::new(StatusCode::BusyHere, Some("Busy Here"))
///     .content_type_text()
///     .content_language("ja")
///     .body("現在話し中です。後でおかけ直しください。")  // "Currently busy. Please call back later."
///     .build();
/// ```

/// Extension trait for adding Content-Language header building capabilities
///
/// This trait provides methods to add Content-Language headers to SIP messages, indicating
/// the natural language(s) of the message body content.
///
/// ## When to use Content-Language
///
/// Content-Language is particularly useful in the following scenarios:
///
/// 1. **Multilingual environments**: When serving users who speak different languages
/// 2. **Global SIP services**: For services operating across countries and regions
/// 3. **User-generated content**: When message bodies contain human-readable text
/// 4. **Error messages**: To provide localized error descriptions
/// 5. **Interactive voice response**: When providing text prompts in different languages
///
/// ## Best Practices
///
/// - Use correct [BCP 47](https://www.rfc-editor.org/info/bcp47) language tags
/// - Include region subtags when language variation is important (e.g., "en-US" vs "en-GB")
/// - When multiple languages are present, list all languages used in the content
/// - Consider including the Content-Language header for all messages with human-readable content
///
/// # More Examples
///
/// ## Customer Support MESSAGE with Language
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentLanguageExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a SIP MESSAGE in German for customer service
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:support@example.com").unwrap()
///     .from("German Customer", "sip:customer@example.de", Some("tag1234"))
///     .to("Support", "sip:support@example.com", None)
///     .content_type_text()
///     .content_language("de")  // Content is in German
///     .body("Ich habe eine Frage zu meiner Rechnung. Können Sie mir bitte helfen?")
///     .build();
/// ```
///
/// ## Multilingual IVR System Response
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::{ContentLanguageExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::StatusCode;
///
/// // Create a 200 OK response with multilingual content
/// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .content_type_text()
///     .content_languages(&["en", "es", "fr"])  // Content includes three languages
///     .body("Press 1 for service.\nPresione 1 para servicio.\nAppuyez sur 1 pour le service.")
///     .build();
/// ```
pub trait ContentLanguageExt {
    /// Add a Content-Language header with a single language
    ///
    /// This method specifies a single language tag for the message body content.
    /// Language tags follow [BCP 47](https://www.rfc-editor.org/info/bcp47) format,
    /// such as "en" for English, "fr" for French, or "en-US" for American English.
    ///
    /// # Arguments
    ///
    /// * `language` - The language tag to specify (e.g., "en", "en-US", "zh-Hans")
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentLanguageExt};
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a MESSAGE with Italian content
    /// let request = SimpleRequestBuilder::new(Method::Message, "sip:recipient@example.com").unwrap()
    ///     .from("Sender", "sip:sender@example.com", Some("tag1234"))
    ///     .to("Recipient", "sip:recipient@example.com", None)
    ///     .content_type_text()
    ///     .content_language("it")  // Content is in Italian
    ///     .body("Buongiorno! Come stai oggi?")
    ///     .build();
    /// ```
    ///
    /// # RFC Reference
    /// 
    /// As per [RFC 3261 Section 20.13](https://datatracker.ietf.org/doc/html/rfc3261#section-20.13),
    /// the Content-Language header field is used to specify the language of the message body.
    /// Language tags follow the format defined in [RFC 5646](https://datatracker.ietf.org/doc/html/rfc5646).
    fn content_language(self, language: &str) -> Self;

    /// Add a Content-Language header with multiple languages
    ///
    /// This method specifies multiple language tags for the message body,
    /// indicating that the content contains elements in multiple languages.
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
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a notification message with content in multiple languages
    /// let request = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
    ///     .from("Multilingual Service", "sip:service@example.com", Some("tag5678"))
    ///     .to("International User", "sip:user@example.com", None)
    ///     .content_type_text()
    ///     .content_languages(&["en-US", "fr-CA"])  // Content includes both English and French
    ///     .body("Your account has been updated.\n\nVotre compte a été mis à jour.")
    ///     .build();
    /// ```
    ///
    /// # Multiple Languages Explained
    ///
    /// When a message body contains text in multiple languages, all languages should be listed
    /// in the Content-Language header. This helps the recipient properly handle the content,
    /// for example:
    /// 
    /// - A multilingual SIP client might choose which portions of the text to display
    /// - A text-to-speech system could select the appropriate voice for each section
    /// - Automated translation services can identify parts that don't need translation
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