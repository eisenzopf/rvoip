use bytes::Bytes;
use rvoip_rtp_core::srtp::{SrtpContext, SRTP_AES128_CM_SHA1_80};
use rvoip_rtp_core::RtpPacket;

#[test]
fn directional_constructor_uses_remote_key_material_for_inbound_packets() {
    let alice_key = vec![0x11; 16];
    let alice_salt = vec![0x22; 14];
    let bob_key = vec![0x33; 16];
    let bob_salt = vec![0x44; 14];

    let mut alice = SrtpContext::new_from_keys(
        alice_key.clone(),
        bob_key.clone(),
        alice_salt.clone(),
        bob_salt.clone(),
        SRTP_AES128_CM_SHA1_80,
    )
    .unwrap();
    let mut bob = SrtpContext::new_from_keys(
        bob_key,
        alice_key,
        bob_salt,
        alice_salt,
        SRTP_AES128_CM_SHA1_80,
    )
    .unwrap();

    let packet = RtpPacket::new_with_payload(
        0,
        1,
        160,
        0x1020_3040,
        Bytes::from_static(b"directional-key-test"),
    );
    let wire = alice.protect(&packet).unwrap().serialize().unwrap();
    let recovered = bob
        .unprotect(&wire)
        .expect("the receiver must authenticate with the sender's remote key");

    assert_eq!(recovered.payload, packet.payload);
}
