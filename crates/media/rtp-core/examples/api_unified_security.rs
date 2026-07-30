//! Unified security API availability example for rvoip 0.3.5.
//!
//! The manager exposes only methods that are implemented and provisioned.
//! Retained DTLS-SRTP, MIKEY, and ZRTP constructors fail validation instead of
//! being advertised or used as fallback methods.

use rvoip_rtp_core::api::common::{
    config::{KeyExchangeMethod, SecurityConfig},
    security_manager::SecurityContextManager,
    SecurityError,
};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let direct_srtp = SecurityConfig::srtp_with_key(vec![0x51; 30]);
    direct_srtp.validate()?;
    SecurityConfig::sdes_srtp().validate()?;

    for unavailable in [
        SecurityConfig::webrtc_compatible(),
        SecurityConfig::mikey_psk(),
        SecurityConfig::mikey_pke(),
        SecurityConfig::zrtp_p2p(),
    ] {
        assert!(matches!(
            unavailable.validate(),
            Err(SecurityError::UnsupportedFeature(_))
        ));
    }

    let manager = SecurityContextManager::new(direct_srtp);
    manager.initialize().await?;
    let available = manager.list_available_methods().await;
    assert_eq!(available, vec![KeyExchangeMethod::PreSharedKey]);

    let capabilities = manager.get_capabilities().await;
    assert_eq!(
        capabilities.supported_methods,
        vec![KeyExchangeMethod::PreSharedKey]
    );
    assert!(
        !capabilities.supported_methods.iter().any(|method| matches!(
            method,
            KeyExchangeMethod::DtlsSrtp | KeyExchangeMethod::Mikey | KeyExchangeMethod::Zrtp
        ))
    );

    info!(methods = ?available, "only implemented, provisioned methods are advertised");
    Ok(())
}
