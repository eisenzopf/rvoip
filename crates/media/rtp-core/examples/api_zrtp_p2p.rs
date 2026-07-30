//! ZRTP fail-closed availability example.
//!
//! rvoip 0.3.5 retains the ZRTP configuration and packet types for source
//! compatibility, but does not implement a complete interoperable exchange.
//! This example demonstrates typed rejection and does not simulate SAS values,
//! negotiated keys, or a secure call.

use rvoip_rtp_core::{
    api::common::{SecurityConfig, SecurityError},
    security::{
        zrtp::{Zrtp, ZrtpConfig, ZrtpRole},
        SecurityKeyExchange,
    },
    Error,
};

fn main() {
    let zrtp_config = ZrtpConfig::default();

    let Err(error) = Zrtp::try_new(zrtp_config.clone(), ZrtpRole::Initiator) else {
        panic!("ZRTP must not construct while its state machine is incomplete");
    };
    assert!(matches!(error, Error::UnsupportedFeature(_)));

    // The infallible constructor remains only for source compatibility. Every
    // protocol operation still rejects the unavailable exchange.
    let mut compatibility_context = Zrtp::new(zrtp_config, ZrtpRole::Initiator);
    assert!(matches!(
        compatibility_context.init(),
        Err(Error::UnsupportedFeature(_))
    ));
    assert!(compatibility_context.get_srtp_key().is_none());

    let public_config = SecurityConfig::zrtp_p2p();
    assert!(matches!(
        public_config.validate(),
        Err(SecurityError::UnsupportedFeature(_))
    ));

    println!("ZRTP correctly failed closed; no secure session was established");
}
