//! RFC 4568 SDES key-exchange wrapper used by the media adapter.
//!
//! session-core owns SDP negotiation (decision D16 in the
//! `STEP_2B_SRTP_INTEGRATION_PLAN.md`); rtp-core owns crypto
//! primitives. This module is the bridge — it consumes typed
//! `CryptoAttribute` values from sip-core, generates fresh master keys
//! per RFC 4568 §6.1, and produces the per-direction `SrtpContext`
//! pair that media-core's RTP transport will use.
//!
//! The SDES state-machine logic lives here (rather than in
//! `crates/rtp-core/src/security/sdes/mod.rs::Sdes`) because the
//! existing rtp-core wrapper is bytes-oriented (its `process_message`
//! takes `&[u8]` of `\r\n`-joined `a=crypto:` lines), which would
//! force an awkward typed↔string round-trip at the SDP boundary.
//! Implementing SDES directly on top of rtp-core's primitives —
//! `SrtpContext`, `SrtpCryptoKey`, `SrtpCryptoSuite` constants,
//! `OsRng`, base64 — keeps the path typed end-to-end.
//!
//! # RFC compliance
//!
//! - RFC 4568 §6.1 — master key length per suite (16+14 = 30 bytes
//!   for AES-128, base64-encoded as the `inline:` parameter).
//! - RFC 4568 §6.2.1 — `AES_CM_128_HMAC_SHA1_80` is MTI; default
//!   offer also includes `_32` for low-bandwidth carrier coverage.
//! - RFC 4568 §7.5 — answerer's chosen tag must reference an
//!   offered tag with the same suite; otherwise reject.
//! - RFC 4568 §6.1 — each side has its own master key (D4). We
//!   build *two* `SrtpContext`s per call: one keyed with our own
//!   master (outbound), one with the peer's (inbound).

use std::collections::HashMap;

use base64::{engine::general_purpose::STANDARD, Engine};
use rand::{rngs::OsRng, RngCore};
use rvoip_rtp_core::srtp::{
    SrtpContext, SrtpCryptoKey, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_32, SRTP_AES128_CM_SHA1_80,
    SRTP_AES256_CM_SHA1_32, SRTP_AES256_CM_SHA1_80,
};
use rvoip_sip_core::types::sdp::{CryptoAttribute, CryptoSuite};

use crate::errors::{Result, SessionError};

/// Salt length in bytes for SDES SDP inline keys (RFC 4568 §6.1).
/// Independent of the encryption-key length.
const SDES_SALT_LEN: usize = 14;

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

/// Generate a fresh random master key + salt for the given suite.
/// Returns `(key, salt, base64_inline)` — the first two for building
/// our local `SrtpContext`, the third to drop into the `inline:`
/// parameter of the outgoing `a=crypto:` SDP attribute.
fn generate_keysalt(suite: &SrtpCryptoSuite) -> (Vec<u8>, Vec<u8>, String) {
    let mut key = vec![0u8; suite.key_length];
    let mut salt = vec![0u8; SDES_SALT_LEN];
    OsRng.fill_bytes(&mut key);
    OsRng.fill_bytes(&mut salt);
    let mut combined = Vec::with_capacity(key.len() + salt.len());
    combined.extend_from_slice(&key);
    combined.extend_from_slice(&salt);
    let inline = STANDARD.encode(&combined);
    (key, salt, inline)
}

/// Decode an `a=crypto:` `inline=` base64 blob into `(key, salt)` parts
/// of suite-correct length. Tolerates the optional `|lifetime` and
/// `|MKI:LEN` suffixes by stripping anything after the first `|`
/// (RFC 4568 §6.1 — we don't honour rekeying or MKI today).
fn decode_keysalt(inline_b64: &str, suite: &SrtpCryptoSuite) -> Result<(Vec<u8>, Vec<u8>)> {
    let key_b64 = inline_b64.split('|').next().unwrap_or(inline_b64);
    let combined = STANDARD.decode(key_b64).map_err(|e| {
        SessionError::SDPNegotiationFailed(format!("invalid base64 in a=crypto inline: {}", e))
    })?;
    let expected = suite.key_length + SDES_SALT_LEN;
    if combined.len() < expected {
        return Err(SessionError::SDPNegotiationFailed(format!(
            "a=crypto inline too short: got {} bytes, need {}",
            combined.len(),
            expected
        )));
    }
    let key = combined[..suite.key_length].to_vec();
    let salt = combined[suite.key_length..suite.key_length + SDES_SALT_LEN].to_vec();
    Ok((key, salt))
}

/// State held by an offerer between sending the offer and receiving
/// the answer. Maps tag → our locally-generated key+salt for that
/// suite. Not part of the documented public API even though it appears
/// in the `Offerer` variant — the only intended interaction is
/// constructing it via [`SrtpNegotiator::new_offerer`] and consuming
/// it via [`SrtpNegotiator::accept_answer`].
#[doc(hidden)]
pub struct OfferedSlot {
    suite: CryptoSuite,
    rtp_suite: SrtpCryptoSuite,
    key: Vec<u8>,
    salt: Vec<u8>,
}

/// Output of a successful SDES exchange — the per-direction
/// `SrtpContext` pair the RTP transport will use to protect
/// outbound packets and unprotect inbound packets (D4).
pub struct SrtpPair {
    /// Outbound (us → peer); keyed with our master.
    pub send_ctx: SrtpContext,
    /// Inbound (peer → us); keyed with the peer's master.
    pub recv_ctx: SrtpContext,
    /// The negotiated suite (for telemetry / diagnostics).
    pub suite: CryptoSuite,
}

/// SDES key-exchange wrapper. Constructed in one of two roles
/// (offerer / answerer) corresponding to the SIP UAC / UAS sides.
pub enum SrtpNegotiator {
    /// UAC awaiting an answer to its offered crypto attributes.
    Offerer { offered: HashMap<u32, OfferedSlot> },
    /// UAS ready to receive an offer.
    Answerer,
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
        let mut offered = HashMap::with_capacity(suites.len());
        let mut attrs = Vec::with_capacity(suites.len());
        for (i, &suite) in suites.iter().enumerate() {
            let tag = (i + 1) as u32;
            let rtp_suite = rtp_suite_for(suite);
            let (key, salt, inline) = generate_keysalt(&rtp_suite);
            attrs.push(CryptoAttribute::new(tag, suite, inline));
            offered.insert(
                tag,
                OfferedSlot {
                    suite,
                    rtp_suite,
                    key,
                    salt,
                },
            );
        }
        Ok((SrtpNegotiator::Offerer { offered }, attrs))
    }

    /// UAS side. Construct an answerer ready to receive an offer.
    pub fn new_answerer() -> Self {
        SrtpNegotiator::Answerer
    }

    /// UAC: peer's answer arrived. Validate it references one of our
    /// offered tags with the matching suite (RFC 4568 §7.5), decode
    /// the peer's master key, and build the `SrtpPair`.
    pub fn accept_answer(&self, attr: &CryptoAttribute) -> Result<SrtpPair> {
        let offered = match self {
            SrtpNegotiator::Offerer { offered } => offered,
            _ => {
                return Err(SessionError::SDPNegotiationFailed(
                    "SrtpNegotiator::accept_answer called on non-offerer".into(),
                ))
            }
        };
        let slot = offered.get(&attr.tag).ok_or_else(|| {
            SessionError::SDPNegotiationFailed(format!(
                "answer's a=crypto tag {} was not offered",
                attr.tag
            ))
        })?;
        if slot.suite != attr.suite {
            return Err(SessionError::SDPNegotiationFailed(format!(
                "answer's a=crypto suite {:?} does not match offered tag {} suite {:?}",
                attr.suite, attr.tag, slot.suite
            )));
        }
        let (peer_key, peer_salt) = decode_keysalt(&attr.key_inline, &slot.rtp_suite)?;
        build_pair(
            slot.rtp_suite.clone(),
            &slot.key,
            &slot.salt,
            &peer_key,
            &peer_salt,
            slot.suite,
        )
    }

    /// UAS: process an inbound offer's `a=crypto:` attributes. Picks
    /// the first suite we support, generates our master key, returns
    /// `(chosen_attribute_to_emit_in_answer, SrtpPair)`. The answer
    /// echoes the offerer's chosen tag with our own inline key.
    pub fn process_offer(&self, attrs: &[CryptoAttribute]) -> Result<(CryptoAttribute, SrtpPair)> {
        if !matches!(self, SrtpNegotiator::Answerer) {
            return Err(SessionError::SDPNegotiationFailed(
                "SrtpNegotiator::process_offer called on non-answerer".into(),
            ));
        }
        // First-supported wins (D2 — offerer ranked, we honour their preference).
        let chosen = attrs.first().ok_or_else(|| {
            SessionError::SDPNegotiationFailed(
                "no offered a=crypto suite is supported by this responder".into(),
            )
        })?;
        let rtp_suite = rtp_suite_for(chosen.suite);
        let (peer_key, peer_salt) = decode_keysalt(&chosen.key_inline, &rtp_suite)?;
        let (our_key, our_salt, our_inline) = generate_keysalt(&rtp_suite);

        let pair = build_pair(
            rtp_suite,
            &our_key,
            &our_salt,
            &peer_key,
            &peer_salt,
            chosen.suite,
        )?;
        Ok((
            CryptoAttribute::new(chosen.tag, chosen.suite, our_inline),
            pair,
        ))
    }
}

fn build_pair(
    rtp_suite: SrtpCryptoSuite,
    our_key: &[u8],
    our_salt: &[u8],
    peer_key: &[u8],
    peer_salt: &[u8],
    suite: CryptoSuite,
) -> Result<SrtpPair> {
    let send_ctx = SrtpContext::new(
        rtp_suite.clone(),
        SrtpCryptoKey::new(our_key.to_vec(), our_salt.to_vec()),
    )
    .map_err(|e| {
        SessionError::SDPNegotiationFailed(format!("failed to build outbound SrtpContext: {}", e))
    })?;
    let recv_ctx = SrtpContext::new(
        rtp_suite,
        SrtpCryptoKey::new(peer_key.to_vec(), peer_salt.to_vec()),
    )
    .map_err(|e| {
        SessionError::SDPNegotiationFailed(format!("failed to build inbound SrtpContext: {}", e))
    })?;
    Ok(SrtpPair {
        send_ctx,
        recv_ctx,
        suite,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_rtp_core::packet::{RtpHeader, RtpPacket};

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
            matches!(&result, Err(e) if format!("{:?}", e).contains("no offered a=crypto suite"))
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
    fn decode_keysalt_strips_lifetime_and_mki_suffixes() {
        let suite = SRTP_AES128_CM_SHA1_80;
        let raw = STANDARD.encode(vec![0u8; 30]);
        // Add the optional suffixes the spec allows.
        let inline = format!("{}|2^31|1:4", raw);
        let (key, salt) = decode_keysalt(&inline, &suite).unwrap();
        assert_eq!(key.len(), 16);
        assert_eq!(salt.len(), 14);
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
}
