//! Security availability showcase for rvoip 0.3.5.
//!
//! The working paths shown here are exact-suite direct SRTP and SDES. Retained
//! DTLS-SRTP, MIKEY, and ZRTP configuration constructors are checked only to
//! demonstrate that validation rejects them with a typed unsupported error.

use bytes::Bytes;
use rvoip_rtp_core::{
    api::common::{SecurityConfig, SecurityError},
    packet::{RtpHeader, RtpPacket},
    srtp::{SrtpContext, SrtpCryptoKey, SRTP_AES128_CM_SHA1_80},
};
use tracing::info;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let direct_srtp = SecurityConfig::srtp_with_key(vec![0x31; 30]);
    direct_srtp.validate()?;
    SecurityConfig::sdes_srtp().validate()?;

    let packet = RtpPacket::new(
        RtpHeader::new(0, 1, 160, 0x1122_3344),
        Bytes::from_static(b"authenticated media"),
    );
    let key = SrtpCryptoKey::new(vec![0x31; 16], vec![0x42; 14]);
    let mut sender = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key.clone())?;
    let mut receiver = SrtpContext::new(SRTP_AES128_CM_SHA1_80, key)?;
    let protected = sender.protect(&packet)?.serialize()?;
    let recovered = receiver.unprotect(&protected)?;
    assert_eq!(recovered.payload, packet.payload);

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

    info!("direct SRTP and SDES validated; DTLS, MIKEY, and ZRTP failed closed");
    Ok(())
}
