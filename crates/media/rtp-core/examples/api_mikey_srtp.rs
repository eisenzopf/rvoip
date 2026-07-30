//! MIKEY-SRTP fail-closed availability example.
//!
//! rvoip 0.3.5 retains the MIKEY public types for source compatibility, but
//! every MIKEY mode is unavailable. In particular, the previous PSK path did
//! not protect transported TEK and salt material. This example demonstrates
//! typed rejection; it never substitutes static SRTP keys or simulates a
//! successful exchange.

use rvoip_rtp_core::{
    api::common::{SecurityConfig, SecurityError},
    security::{
        mikey::{Mikey, MikeyConfig, MikeyKeyExchangeMethod, MikeyRole},
        SecurityKeyExchange,
    },
    Error,
};
use tracing::info;

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let mikey_config = MikeyConfig {
        method: MikeyKeyExchangeMethod::Psk,
        psk: Some(vec![0x42; 32]),
        ..Default::default()
    };

    let Err(error) = Mikey::try_new(mikey_config.clone(), MikeyRole::Initiator) else {
        panic!("MIKEY-PSK must not construct while key transport is incomplete");
    };
    assert!(matches!(error, Error::UnsupportedFeature(_)));

    // The infallible constructor remains only for source compatibility. Every
    // protocol operation still rejects the unavailable exchange.
    let mut compatibility_context = Mikey::new(mikey_config, MikeyRole::Initiator);
    assert!(matches!(
        compatibility_context.init(),
        Err(Error::UnsupportedFeature(_))
    ));
    assert!(compatibility_context.get_srtp_key().is_none());

    let public_config = SecurityConfig::mikey_psk();
    assert!(matches!(
        public_config.validate(),
        Err(SecurityError::UnsupportedFeature(_))
    ));

    info!("MIKEY correctly failed closed; use SDES or explicitly provisioned direct SRTP");
}
