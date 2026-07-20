//! Property qualification for outbound originate header/auth line safety.
//!
//! The public SIP originate builders may reject arbitrary input. Whenever
//! they accept it and the authentication provider produces a wire value, the
//! real typed serializer must still be incapable of emitting another header
//! line.

use proptest::prelude::*;
use rvoip_sip::{SipInitialHeaders, SipOriginateContext, SipTransportSecurityContext};
use rvoip_sip_core::types::header::HeaderName;
use rvoip_sip_core::types::headers::{HeaderValue, TypedHeader};

fn assert_one_serialized_header_line(wire: &str) -> Result<(), TestCaseError> {
    prop_assert!(!wire.contains('\r'), "serialized header contained CR");
    prop_assert!(!wire.contains('\n'), "serialized header contained LF");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn every_accepted_initial_header_serializes_as_one_line(
        name in any::<String>(),
        value in any::<String>(),
    ) {
        let Ok(headers) = SipInitialHeaders::new([(name, value)]) else {
            return Ok(());
        };
        let (name, value) = headers.iter().next().expect("one accepted header");
        let header = TypedHeader::Other(
            name.clone(),
            HeaderValue::Raw(value.as_bytes().to_vec()),
        );
        assert_one_serialized_header_line(&header.to_string())?;
    }

    #[test]
    fn every_generated_auth_value_serializes_as_one_line(
        kind in 0_u8..3,
        username in any::<String>(),
        secret in any::<String>(),
    ) {
        let (auth, challenge) = match kind {
            0 => (
                rvoip_sip::auth::SipClientAuth::digest(username, secret),
                r#"Digest realm="property.test", nonce="n1", algorithm=MD5, qop="auth""#,
            ),
            1 => (
                rvoip_sip::auth::SipClientAuth::basic(username, secret)
                    .allow_basic_over_cleartext(true),
                r#"Basic realm="property.test""#,
            ),
            _ => (
                rvoip_sip::auth::SipClientAuth::bearer_token(secret)
                    .allow_bearer_over_cleartext(true),
                r#"Bearer realm="property.test""#,
            ),
        };
        let Ok(context) = SipOriginateContext::new().with_auth(auth) else {
            return Ok(());
        };
        let Ok(generated) = context
            .auth()
            .expect("accepted context retained auth")
            .authorization_for_challenge_with_transport_context(
                challenge,
                "INVITE",
                "sip:callee@127.0.0.1",
                1,
                None,
                &SipTransportSecurityContext::from_transport_name("TLS"),
            )
        else {
            return Ok(());
        };

        let header = rvoip_sip_core::validation::validated_authorization_header(
            HeaderName::Authorization,
            generated.value,
        )
        .expect("generated auth passed its final insertion validator");
        assert_one_serialized_header_line(&header.to_string())?;
    }

    #[test]
    fn every_accepted_precomputed_auth_value_serializes_as_one_line(
        proxy in any::<bool>(),
        value in any::<String>(),
    ) {
        let name = if proxy {
            HeaderName::ProxyAuthorization
        } else {
            HeaderName::Authorization
        };
        let Ok(header) = rvoip_sip_core::validation::validated_authorization_header(
            name,
            value,
        ) else {
            return Ok(());
        };
        assert_one_serialized_header_line(&header.to_string())?;
    }
}
