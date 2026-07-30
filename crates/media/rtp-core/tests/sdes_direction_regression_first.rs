use rvoip_rtp_core::security::sdes::{Sdes, SdesConfig, SdesRole};
use rvoip_rtp_core::security::SecurityKeyExchange;

#[test]
fn sdes_answerer_generates_fresh_transmit_key_material() {
    let mut offerer = Sdes::new(SdesConfig::default(), SdesRole::Offerer);
    let offer = offerer.process_message(b"").unwrap().unwrap();
    let offerer_key = offerer.get_srtp_key().unwrap();

    let mut answerer = Sdes::new(SdesConfig::default(), SdesRole::Answerer);
    let _answer = answerer.process_message(&offer).unwrap().unwrap();
    let answerer_key = answerer.get_srtp_key().unwrap();

    assert_ne!(
        (offerer_key.key(), offerer_key.salt()),
        (answerer_key.key(), answerer_key.salt()),
        "an SDES answer must carry fresh local transmit key material"
    );
}
