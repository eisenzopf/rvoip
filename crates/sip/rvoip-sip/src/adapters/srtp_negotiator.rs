//! RFC 4568 SDES key-exchange wrapper used by the media adapter.
//!
//! This is a thin typed-boundary adapter over the canonical SDES engine
//! in `rvoip_rtp_core::security::sdes` — that module owns all the actual
//! negotiation logic (key generation, per-tag offerer state, answer
//! validation, key/salt length checks, rejecting unsupported lifetime/
//! MKI/session-param extensions). This file's only job is converting
//! between sip-core's typed SDP `CryptoAttribute`/`CryptoSuite` and
//! rtp-core's `SdesCryptoAttribute`/`SrtpCryptoSuite` at the boundary
//! between SIP/SDP concerns and the crypto engine.
//!
//! # RFC compliance
//!
//! See `rvoip_rtp_core::security::sdes` for the full RFC 4568 compliance
//! notes (master key length, lifetime/MKI rejection, answer tag/suite
//! validation) — this module inherits all of it unchanged.

use rvoip_rtp_core::security::sdes::{SdesCryptoAttribute, SdesNegotiator};
use rvoip_rtp_core::srtp::{
    SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80,
    SRTP_AES256_CM_SHA1_32, SRTP_AES256_CM_SHA1_80,
};
use rvoip_sip_core::types::sdp::{CryptoAttribute, CryptoSuite};

use crate::errors::{Result, SessionError};

/// Map a typed sip-core `CryptoSuite` to the matching rtp-core
/// `SrtpCryptoSuite` constant.
fn rtp_suite_for(suite: CryptoSuite) -> SrtpCryptoSuite {
    match suite {
        CryptoSuite::AesCm128HmacSha1_80 => SRTP_AES128_CM_SHA1_80,
        CryptoSuite::AesCm128HmacSha1_32 => SRTP_AES128_CM_SHA1_32,
        CryptoSuite::AesCm256HmacSha1_80 => SRTP_AES256_CM_SHA1_80,
        CryptoSuite::AesCm256HmacSha1_32 => SRTP_AES256_CM_SHA1_32,
    }
}

/// Map an rtp-core `SrtpCryptoSuite` back to the sip-core wire-name enum.
/// `_80`/`_32` only change the authentication tag length, not the
/// key/salt size, so the mapping is exact and total over the four suites
/// this crate supports.
fn sip_suite_for(suite: &SrtpCryptoSuite) -> Result<CryptoSuite> {
    match (suite.key_length, suite.tag_length) {
        (16, 10) => Ok(CryptoSuite::AesCm128HmacSha1_80),
        (16, 4) => Ok(CryptoSuite::AesCm128HmacSha1_32),
        (32, 10) => Ok(CryptoSuite::AesCm256HmacSha1_80),
        (32, 4) => Ok(CryptoSuite::AesCm256HmacSha1_32),
        _ => Err(SessionError::SDPNegotiationFailed(format!(
            "unsupported crypto suite: {suite:?}"
        ))),
    }
}

fn to_core_attr(attr: &CryptoAttribute) -> SdesCryptoAttribute {
    SdesCryptoAttribute {
        tag: attr.tag,
        suite: rtp_suite_for(attr.suite),
        key_inline: attr.key_inline.clone(),
        key_lifetime: attr.key_lifetime.clone(),
        key_mki: attr.key_mki,
        session_params: attr.session_params.clone(),
    }
}

fn from_core_attr(attr: &SdesCryptoAttribute) -> Result<CryptoAttribute> {
    Ok(CryptoAttribute::new(
        attr.tag,
        sip_suite_for(&attr.suite)?,
        attr.key_inline.clone(),
    ))
}

/// Output of a successful SDES exchange — the per-direction
/// `SrtpContext` pair the RTP transport will use to protect outbound
/// packets and unprotect inbound packets (D4).
pub struct SrtpPair {
    /// Outbound (us → peer); keyed with our master.
    pub send_ctx: SrtpContext,
    /// Inbound (peer → us); keyed with the peer's master.
    pub recv_ctx: SrtpContext,
    /// The negotiated suite (for telemetry / diagnostics).
    pub suite: CryptoSuite,
}

fn to_sip_pair(pair: rvoip_rtp_core::security::sdes::SdesSrtpPair) -> Result<SrtpPair> {
    let suite = sip_suite_for(&pair.suite)?;
    Ok(SrtpPair {
        send_ctx: pair.send_ctx,
        recv_ctx: pair.recv_ctx,
        suite,
    })
}

/// SDES key-exchange wrapper. Constructed in one of two roles
/// (offerer / answerer) corresponding to the SIP UAC / UAS sides.
/// Delegates all actual negotiation to
/// `rvoip_rtp_core::security::sdes::SdesNegotiator`.
pub enum SrtpNegotiator {
    /// UAC awaiting an answer to its offered crypto attributes.
    Offerer(SdesNegotiator),
    /// UAS ready to receive an offer.
    Answerer(SdesNegotiator),
}

impl SrtpNegotiator {
    /// UAC side. Generate fresh master keys for each requested suite
    /// and return the typed `a=crypto:` lines to attach to the SDP
    /// offer. Suites are emitted with sequential tags (1, 2, ...) in
    /// the order supplied — the answerer is expected to pick the
    /// first tag whose suite it supports.
    pub fn new_offerer(suites: &[CryptoSuite]) -> Result<(Self, Vec<CryptoAttribute>)> {
        if suites.is_empty() {
            return Err(SessionError::SDPNegotiationFailed(
                "SrtpNegotiator::new_offerer requires at least one suite".into(),
            ));
        }
        let rtp_suites: Vec<SrtpCryptoSuite> = suites.iter().map(|s| rtp_suite_for(*s)).collect();
        let (inner, attrs) = SdesNegotiator::new_offerer(&rtp_suites)
            .map_err(|e| SessionError::SDPNegotiationFailed(e.to_string()))?;
        let sip_attrs = attrs
            .iter()
            .map(from_core_attr)
            .collect::<Result<Vec<_>>>()?;
        Ok((SrtpNegotiator::Offerer(inner), sip_attrs))
    }

    /// UAS side. Construct an answerer ready to receive an offer.
    pub fn new_answerer() -> Self {
        SrtpNegotiator::Answerer(SdesNegotiator::new_answerer())
    }

    /// UAC: peer's answer arrived. Validate it references one of our
    /// offered tags with the matching suite (RFC 4568 §7.5), decode
    /// the peer's master key, and build the `SrtpPair`.
    pub fn accept_answer(&self, attr: &CryptoAttribute) -> Result<SrtpPair> {
        let SrtpNegotiator::Offerer(inner) = self else {
            return Err(SessionError::SDPNegotiationFailed(
                "SrtpNegotiator::accept_answer called on non-offerer".into(),
            ));
        };
        let pair = inner
            .accept_answer(&to_core_attr(attr))
            .map_err(|e| SessionError::SDPNegotiationFailed(e.to_string()))?;
        to_sip_pair(pair)
    }

    /// UAS: process an inbound offer's `a=crypto:` attributes. Picks
    /// the first suite we support, generates our master key, returns
    /// `(chosen_attribute_to_emit_in_answer, SrtpPair)`. The answer
    /// echoes the offerer's chosen tag with our own inline key.
    pub fn process_offer(&self, attrs: &[CryptoAttribute]) -> Result<(CryptoAttribute, SrtpPair)> {
        let SrtpNegotiator::Answerer(inner) = self else {
            return Err(SessionError::SDPNegotiationFailed(
                "SrtpNegotiator::process_offer called on non-answerer".into(),
            ));
        };
        let core_attrs: Vec<SdesCryptoAttribute> = attrs.iter().map(to_core_attr).collect();
        let (chosen, pair) = inner
            .process_offer(&core_attrs)
            .map_err(|e| SessionError::SDPNegotiationFailed(e.to_string()))?;
        Ok((from_core_attr(&chosen)?, to_sip_pair(pair)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};

    fn default_offered() -> Vec<CryptoSuite> {
        vec![
            CryptoSuite::AesCm128HmacSha1_80,
            CryptoSuite::AesCm128HmacSha1_32,
        ]
    }

    #[test]
    fn offerer_emits_one_attribute_per_suite_with_sequential_tags() {
        let suites = default_offered();
        let (_, attrs) = SrtpNegotiator::new_offerer(&suites).unwrap();
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].tag, 1);
        assert_eq!(attrs[0].suite, CryptoSuite::AesCm128HmacSha1_80);
        assert_eq!(attrs[1].tag, 2);
        assert_eq!(attrs[1].suite, CryptoSuite::AesCm128HmacSha1_32);
        // Each offered key should be base64 of 30 bytes (AES-128: 16 key + 14 salt) → 40 chars no padding.
        assert!(!attrs[0].key_inline.is_empty());
        let decoded = STANDARD.decode(&attrs[0].key_inline).unwrap();
        assert_eq!(decoded.len(), 30);
    }

    #[test]
    fn full_offer_answer_round_trip_produces_compatible_contexts() {
        // UAC builds offer.
        let suites = default_offered();
        let (offerer, offer_attrs) = SrtpNegotiator::new_offerer(&suites).unwrap();

        // UAS processes offer, picks first supported suite.
        let answerer = SrtpNegotiator::new_answerer();
        let (answer_attr, mut answerer_pair) = answerer.process_offer(&offer_attrs).unwrap();
        assert_eq!(answer_attr.tag, 1, "first-supported wins");
        assert_eq!(answer_attr.suite, CryptoSuite::AesCm128HmacSha1_80);

        // UAC accepts answer.
        let mut offerer_pair = offerer.accept_answer(&answer_attr).unwrap();

        // Build a real RTP packet, encrypt with UAC's send_ctx, decrypt with UAS's recv_ctx.
        // (UAC→UAS direction uses UAC's master key for encryption.)
        use rvoip_rtp_core::packet::{RtpHeader, RtpPacket};
        let header = RtpHeader::new(0, 1, 12345, 0xdead_beef);
        let payload = bytes::Bytes::from_static(b"hello srtp world");
        let packet = RtpPacket::new(header, payload.clone());
        let protected = offerer_pair.send_ctx.protect(&packet).unwrap();
        let bytes = protected.serialize().unwrap();
        let decrypted = answerer_pair.recv_ctx.unprotect(&bytes).unwrap();
        assert_eq!(decrypted.payload, payload);

        // UAS→UAC direction uses UAS's master key.
        let header2 = RtpHeader::new(0, 1, 12345, 0xface_d00d);
        let payload2 = bytes::Bytes::from_static(b"hello back");
        let packet2 = RtpPacket::new(header2, payload2.clone());
        let protected2 = answerer_pair.send_ctx.protect(&packet2).unwrap();
        let bytes2 = protected2.serialize().unwrap();
        let decrypted2 = offerer_pair.recv_ctx.unprotect(&bytes2).unwrap();
        assert_eq!(decrypted2.payload, payload2);
    }

    #[test]
    fn accept_answer_rejects_unknown_tag() {
        let (offerer, _) = SrtpNegotiator::new_offerer(&default_offered()).unwrap();
        // Tag 99 was never offered.
        let bogus = CryptoAttribute::new(
            99,
            CryptoSuite::AesCm128HmacSha1_80,
            STANDARD.encode(vec![0u8; 30]),
        );
        let result = offerer.accept_answer(&bogus);
        assert!(matches!(&result, Err(e) if format!("{:?}", e).contains("was not offered")));
    }

    #[test]
    fn accept_answer_rejects_suite_mismatch_for_known_tag() {
        let (offerer, _) = SrtpNegotiator::new_offerer(&default_offered()).unwrap();
        // Tag 1 was offered as `_80`, answerer claims `_32`.
        let mismatch = CryptoAttribute::new(
            1,
            CryptoSuite::AesCm128HmacSha1_32,
            STANDARD.encode(vec![0u8; 30]),
        );
        let result = offerer.accept_answer(&mismatch);
        assert!(matches!(&result, Err(e) if format!("{:?}", e).contains("does not match")));
    }

    #[test]
    fn process_offer_errors_when_no_crypto_suites_are_available() {
        let answerer = SrtpNegotiator::new_answerer();
        let result = answerer.process_offer(&[]);
        assert!(
            matches!(&result, Err(e) if format!("{:?}", e).contains("no offered a=crypto attribute"))
        );
    }

    #[test]
    fn process_offer_accepts_aes256_when_offered_alone() {
        let attrs = vec![CryptoAttribute::new(
            1,
            CryptoSuite::AesCm256HmacSha1_80,
            STANDARD.encode(vec![0u8; 46]),
        )];
        let answerer = SrtpNegotiator::new_answerer();
        let (chosen, pair) = answerer.process_offer(&attrs).unwrap();
        assert_eq!(chosen.tag, 1);
        assert_eq!(chosen.suite, CryptoSuite::AesCm256HmacSha1_80);
        assert_eq!(pair.suite, CryptoSuite::AesCm256HmacSha1_80);
    }

    #[test]
    fn process_offer_honors_asterisk_default_order_with_aes256_second() {
        let attrs = vec![
            CryptoAttribute::new(
                1,
                CryptoSuite::AesCm128HmacSha1_80,
                STANDARD.encode(vec![0u8; 30]),
            ),
            CryptoAttribute::new(
                2,
                CryptoSuite::AesCm256HmacSha1_80,
                STANDARD.encode(vec![0u8; 46]),
            ),
        ];

        let answerer = SrtpNegotiator::new_answerer();
        let (chosen, _) = answerer.process_offer(&attrs).unwrap();
        assert_eq!(chosen.tag, 1, "answerer should honor offerer order");
        assert_eq!(chosen.suite, CryptoSuite::AesCm128HmacSha1_80);
    }

    #[test]
    fn process_offer_picks_aes256_when_it_is_first_supported() {
        let attrs = vec![
            CryptoAttribute::new(
                1,
                CryptoSuite::AesCm256HmacSha1_80,
                STANDARD.encode(vec![0u8; 46]),
            ),
            CryptoAttribute::new(
                2,
                CryptoSuite::AesCm128HmacSha1_80,
                STANDARD.encode(vec![0u8; 30]),
            ),
        ];

        let answerer = SrtpNegotiator::new_answerer();
        let (chosen, pair) = answerer.process_offer(&attrs).unwrap();
        assert_eq!(chosen.tag, 1);
        assert_eq!(chosen.suite, CryptoSuite::AesCm256HmacSha1_80);
        assert_eq!(pair.suite, CryptoSuite::AesCm256HmacSha1_80);
    }

    #[test]
    fn process_offer_rejects_key_lifetime_extension() {
        let mut attr = CryptoAttribute::new(
            1,
            CryptoSuite::AesCm128HmacSha1_80,
            STANDARD.encode(vec![0u8; 30]),
        );
        attr.key_lifetime = Some("2^20".to_string());
        let answerer = SrtpNegotiator::new_answerer();
        let result = answerer.process_offer(&[attr]);
        assert!(matches!(&result, Err(e) if format!("{:?}", e).contains("lifetime")));
    }

    #[test]
    fn process_offer_rejects_session_parameters() {
        let mut attr = CryptoAttribute::new(
            1,
            CryptoSuite::AesCm128HmacSha1_80,
            STANDARD.encode(vec![0u8; 30]),
        );
        attr.session_params = vec!["UNENCRYPTED_SRTP".to_string()];
        let answerer = SrtpNegotiator::new_answerer();
        let result = answerer.process_offer(&[attr]);
        assert!(matches!(&result, Err(e) if format!("{:?}", e).contains("session parameters")));
    }
}
