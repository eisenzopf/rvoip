//! Advanced security fail-closed availability example.
//!
//! rvoip 0.3.5 disables automatic rotation and multi-stream derivation until a
//! standard, reviewed KDF is implemented. Built-in recovery policies contain
//! no unavailable DTLS, MIKEY, or ZRTP fallback. This example verifies those
//! properties instead of presenting placeholder derivation as production use.

use rvoip_rtp_core::api::common::{
    advanced_security::{
        error_recovery::FallbackConfig,
        key_management::{KeyManager, KeyRotationPolicy, KeySyndicationConfig, SecurityPolicy},
    },
    config::KeyExchangeMethod,
    SecurityError,
};
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let key_manager = KeyManager::new(
        KeyRotationPolicy::Manual,
        KeySyndicationConfig::multimedia(),
        SecurityPolicy::default(),
    );
    assert!(matches!(
        key_manager.initialize(vec![0x61; 32]).await,
        Err(SecurityError::UnsupportedFeature(_))
    ));
    assert!(matches!(
        key_manager.rotate_keys().await,
        Err(SecurityError::UnsupportedFeature(_))
    ));

    for fallback in [
        FallbackConfig::default(),
        FallbackConfig::enterprise(),
        FallbackConfig::peer_to_peer(),
        FallbackConfig::development(),
    ] {
        assert!(!fallback.method_priority.is_empty());
        assert!(fallback
            .method_priority
            .iter()
            .all(|method| *method == KeyExchangeMethod::Sdes));
    }

    info!("placeholder key management failed closed and recovery advertises only SDES");
}
