use rvoip_rtp_core::dtls::{create_connection, DtlsConfig};
use rvoip_rtp_core::srtp::{SrtpContext, SrtpCryptoKey, SRTP_AEAD_AES_128_GCM};

#[test]
fn aes_gcm_context_construction_fails_closed() {
    let key = SrtpCryptoKey::new(vec![0x11; 16], vec![0x22; 14]);
    assert!(
        SrtpContext::new(SRTP_AEAD_AES_128_GCM, key).is_err(),
        "the placeholder AES-GCM profile must not construct as AES-CM"
    );
}

#[tokio::test]
async fn dtls_construction_returns_an_error_instead_of_panicking() {
    assert!(
        create_connection(DtlsConfig::default()).await.is_err(),
        "incomplete DTLS construction must return a typed error"
    );
}
