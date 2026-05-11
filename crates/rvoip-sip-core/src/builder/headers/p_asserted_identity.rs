use std::str::FromStr;

use crate::error::Result;
use crate::types::{
    address::Address,
    p_asserted_identity::{PAssertedIdentity, PPreferredIdentity},
    uri::Uri,
};

use super::HeaderSetter;

/// Builder extension for the `P-Asserted-Identity` header (RFC 3325 §9.1).
///
/// Used by trusted intermediaries (carriers, PBX trunks) to convey the
/// verified identity of the originating user. Carriers commonly require this
/// on outbound trunk INVITEs for caller-ID assertion; without it the call is
/// often hard-rejected or stripped of caller ID.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PAssertedIdentityBuilderExt};
///
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@trunk.example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@trunk.example.com", None)
///     .p_asserted_identity("sip:alice@example.com").unwrap()
///     .build();
/// ```
pub trait PAssertedIdentityBuilderExt {
    /// Add a `P-Asserted-Identity` header carrying a single URI (no display
    /// name).
    fn p_asserted_identity(self, uri: impl AsRef<str>) -> Result<Self>
    where
        Self: Sized;

    /// Add a `P-Asserted-Identity` header with a display name and URI.
    fn p_asserted_identity_with_display(
        self,
        display_name: impl AsRef<str>,
        uri: impl AsRef<str>,
    ) -> Result<Self>
    where
        Self: Sized;

    /// Add a `P-Asserted-Identity` header carrying multiple URIs (e.g. a
    /// `sip:` plus a matching `tel:`).
    fn p_asserted_identity_uris(self, uris: Vec<impl AsRef<str>>) -> Result<Self>
    where
        Self: Sized;
}

impl<T> PAssertedIdentityBuilderExt for T
where
    T: HeaderSetter,
{
    fn p_asserted_identity(self, uri: impl AsRef<str>) -> Result<Self> {
        let parsed = Uri::from_str(uri.as_ref())?;
        Ok(self.set_header(PAssertedIdentity::with_uri(parsed)))
    }

    fn p_asserted_identity_with_display(
        self,
        display_name: impl AsRef<str>,
        uri: impl AsRef<str>,
    ) -> Result<Self> {
        let parsed = Uri::from_str(uri.as_ref())?;
        let address = Address::new_with_display_name(display_name.as_ref(), parsed);
        Ok(self.set_header(PAssertedIdentity::with_address(address)))
    }

    fn p_asserted_identity_uris(self, uris: Vec<impl AsRef<str>>) -> Result<Self> {
        let mut pai = PAssertedIdentity::empty();
        for uri_str in uris {
            let parsed = Uri::from_str(uri_str.as_ref())?;
            pai.add_uri(parsed);
        }
        Ok(self.set_header(pai))
    }
}

/// Builder extension for the `P-Preferred-Identity` header (RFC 3325 §9.2).
///
/// Sent by a UAC towards a trusted intermediary to express the identity it
/// would prefer the intermediary to assert on the outbound leg. The
/// intermediary either honours it (emitting a matching `P-Asserted-Identity`)
/// or rejects with `403 Forbidden`.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::PPreferredIdentityBuilderExt};
///
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@trunk.example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@trunk.example.com", None)
///     .p_preferred_identity("sip:alice.assistant@example.com").unwrap()
///     .build();
/// ```
pub trait PPreferredIdentityBuilderExt {
    /// Add a `P-Preferred-Identity` header carrying a single URI.
    fn p_preferred_identity(self, uri: impl AsRef<str>) -> Result<Self>
    where
        Self: Sized;

    /// Add a `P-Preferred-Identity` header with a display name and URI.
    fn p_preferred_identity_with_display(
        self,
        display_name: impl AsRef<str>,
        uri: impl AsRef<str>,
    ) -> Result<Self>
    where
        Self: Sized;

    /// Add a `P-Preferred-Identity` header carrying multiple URIs.
    fn p_preferred_identity_uris(self, uris: Vec<impl AsRef<str>>) -> Result<Self>
    where
        Self: Sized;
}

impl<T> PPreferredIdentityBuilderExt for T
where
    T: HeaderSetter,
{
    fn p_preferred_identity(self, uri: impl AsRef<str>) -> Result<Self> {
        let parsed = Uri::from_str(uri.as_ref())?;
        Ok(self.set_header(PPreferredIdentity::with_uri(parsed)))
    }

    fn p_preferred_identity_with_display(
        self,
        display_name: impl AsRef<str>,
        uri: impl AsRef<str>,
    ) -> Result<Self> {
        let parsed = Uri::from_str(uri.as_ref())?;
        let address = Address::new_with_display_name(display_name.as_ref(), parsed);
        Ok(self.set_header(PPreferredIdentity::with_address(address)))
    }

    fn p_preferred_identity_uris(self, uris: Vec<impl AsRef<str>>) -> Result<Self> {
        let mut ppi = PPreferredIdentity::empty();
        for uri_str in uris {
            let parsed = Uri::from_str(uri_str.as_ref())?;
            ppi.add_uri(parsed);
        }
        Ok(self.set_header(ppi))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::{
        headers::{HeaderName, TypedHeader},
        method::Method,
    };

    #[test]
    fn pai_single_uri_lands_on_request() {
        let req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@trunk.example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag1"))
            .to("Bob", "sip:bob@trunk.example.com", None)
            .p_asserted_identity("sip:alice@example.com")
            .unwrap()
            .build();

        let header = req
            .headers
            .iter()
            .find(|h| matches!(h, TypedHeader::PAssertedIdentity(_)))
            .expect("PAI header missing");
        if let TypedHeader::PAssertedIdentity(pai) = header {
            assert_eq!(pai.len(), 1);
            assert_eq!(pai[0].uri.to_string(), "sip:alice@example.com");
        }
    }

    #[test]
    fn pai_with_display_name_carries_display() {
        let req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@trunk.example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag1"))
            .to("Bob", "sip:bob@trunk.example.com", None)
            .p_asserted_identity_with_display("Alice Smith", "sip:alice@example.com")
            .unwrap()
            .build();

        let header = req
            .headers
            .iter()
            .find(|h| matches!(h, TypedHeader::PAssertedIdentity(_)))
            .expect("PAI header missing");
        if let TypedHeader::PAssertedIdentity(pai) = header {
            assert_eq!(pai[0].display_name(), Some("Alice Smith"));
        }
    }

    #[test]
    fn pai_two_uris_emit_both() {
        let req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@trunk.example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag1"))
            .to("Bob", "sip:bob@trunk.example.com", None)
            .p_asserted_identity_uris(vec!["sip:alice@example.com", "tel:+14155551234"])
            .unwrap()
            .build();

        let header = req
            .headers
            .iter()
            .find(|h| matches!(h, TypedHeader::PAssertedIdentity(_)))
            .expect("PAI header missing");
        if let TypedHeader::PAssertedIdentity(pai) = header {
            assert_eq!(pai.len(), 2);
        }
    }

    #[test]
    fn ppi_single_uri_lands_on_request() {
        let req = SimpleRequestBuilder::new(Method::Invite, "sip:bob@trunk.example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag1"))
            .to("Bob", "sip:bob@trunk.example.com", None)
            .p_preferred_identity("sip:alice.assistant@example.com")
            .unwrap()
            .build();

        let header = req
            .headers
            .iter()
            .find(|h| matches!(h, TypedHeader::PPreferredIdentity(_)))
            .expect("PPI header missing");
        if let TypedHeader::PPreferredIdentity(ppi) = header {
            assert_eq!(ppi[0].uri.to_string(), "sip:alice.assistant@example.com");
        }
    }

    #[test]
    fn invalid_uri_errors() {
        let result = SimpleRequestBuilder::new(Method::Invite, "sip:bob@trunk.example.com")
            .unwrap()
            .p_asserted_identity("not a uri");
        assert!(result.is_err());
    }
}
