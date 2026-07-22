//! SDES (Security Descriptions, RFC 4568) SDP key exchange.
//!
//! This is the canonical SDES implementation for the whole workspace —
//! extracted and generalized from `rvoip-sip`'s adapter (which had
//! already gotten this right and is proven by its own end-to-end call
//! test) to live here instead, so every consumer shares one correct
//! engine instead of each maintaining its own.
//!
//! # What the previous version of this module got wrong
//!
//! The old `Sdes`/`SdesConfig`/`SdesRole` byte-oriented state machine
//! (removed by this rewrite) had a real bug: the answerer never
//! generated its own master key. `create_answer` decoded the *offerer's*
//! key from the offer and stored it as `self.srtp_key`, then echoed the
//! offerer's own crypto attribute straight back in the answer. RFC 4568
//! §6.1 requires each side to use its own independently generated master
//! key; this module's tests only ever exercised a single shared key
//! encrypting and decrypting with itself, which trivially round-trips
//! regardless of directionality — it never actually proved two
//! independent keyed contexts worked, which is exactly why the bug went
//! unnoticed.
//!
//! # RFC compliance
//!
//! - RFC 4568 §6.1 — each side generates its own master key; the offerer
//!   keeps its locally-generated key per offered tag until the answer
//!   arrives, the answerer generates a fresh key of its own for the
//!   answer.
//! - RFC 4568 §6.1 — master key + salt length is exactly
//!   `suite.key_length + 14` bytes (112-bit salt for every suite here);
//!   anything short or long is rejected, not truncated or zero-padded.
//! - RFC 4568 §6.1 — `|lifetime` and `|MKI:length` key-parameter
//!   extensions, and any session parameters (`UNENCRYPTED_SRTP`, `KDR=N`,
//!   etc.), are explicitly rejected rather than silently ignored: silently
//!   dropping a parameter a peer considers load-bearing is a silent
//!   security downgrade, not a compatibility shim.
//! - RFC 4568 §7.5 — the answerer's chosen tag must reference a tag the
//!   offerer actually offered, with the same suite; otherwise reject.

use std::collections::HashMap;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::{rngs::OsRng, RngCore};
use zeroize::Zeroize;

use crate::error::Error;
use crate::srtp::{SrtpContext, SrtpCryptoKey, SrtpCryptoSuite};
use crate::Result;

/// Standard SRTP master salt length in bytes (RFC 3711 §8.2 / RFC 4568
/// §6.1) — independent of the suite's key length.
const SDES_SALT_LEN: usize = 14;

/// A single RFC 4568 `a=crypto:` attribute's negotiation-relevant fields.
/// Deliberately independent of any SDP text representation or of
/// sip-core's `CryptoSuite` wire-name enum — encoding/decoding the actual
/// `a=crypto:` line and mapping to a wire-format suite name is the
/// caller's job (e.g. `rvoip-sip`'s adapter, which already owns SDP
/// generation/parsing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdesCryptoAttribute {
    /// Tag: small positive integer, unique per `m=` section.
    pub tag: u32,
    /// The SRTP crypto suite this attribute proposes/selects.
    pub suite: SrtpCryptoSuite,
    /// Base64-encoded `key || salt` (RFC 4568 §6.1), with no `|lifetime`
    /// or `|MKI:length` suffix — those aren't supported, see
    /// [`SdesCryptoAttribute::key_lifetime`]/[`SdesCryptoAttribute::key_mki`].
    pub key_inline: String,
    /// Optional `|lifetime` key parameter. Always rejected if present —
    /// carried as a field (rather than just erroring at parse time) so a
    /// caller that already parsed the full attribute (e.g. from typed SDP
    /// types) can hand it over for this module to reject explicitly.
    pub key_lifetime: Option<String>,
    /// Optional `|MKI:length` key parameter. Always rejected if present.
    pub key_mki: Option<(u32, u32)>,
    /// Optional session parameters (`UNENCRYPTED_SRTP`, `KDR=N`, etc.).
    /// Always rejected if non-empty.
    pub session_params: Vec<String>,
}

impl SdesCryptoAttribute {
    /// Construct a minimal attribute with no lifetime/MKI/session
    /// parameters — the common case.
    pub fn new(tag: u32, suite: SrtpCryptoSuite, key_inline: impl Into<String>) -> Self {
        Self {
            tag,
            suite,
            key_inline: key_inline.into(),
            key_lifetime: None,
            key_mki: None,
            session_params: Vec::new(),
        }
    }

    /// `Err` if this attribute uses a key-parameter extension or session
    /// parameter this module doesn't support (RFC 4568 §6.1 lifetime/MKI,
    /// or any session param) — reject explicitly rather than silently
    /// ignoring something a peer may consider load-bearing.
    fn reject_unsupported_extensions(&self) -> Result<()> {
        if self.key_lifetime.is_some() {
            return Err(Error::UnsupportedFeature(format!(
                "a=crypto tag {} uses a key lifetime parameter, which is not supported",
                self.tag
            )));
        }
        if self.key_mki.is_some() {
            return Err(Error::UnsupportedFeature(format!(
                "a=crypto tag {} uses an MKI parameter, which is not supported",
                self.tag
            )));
        }
        if !self.session_params.is_empty() {
            return Err(Error::UnsupportedFeature(format!(
                "a=crypto tag {} uses session parameters ({}), which are not supported",
                self.tag,
                self.session_params.join(",")
            )));
        }
        Ok(())
    }
}

/// Output of a successful SDES exchange — the per-direction
/// [`SrtpContext`] pair the RTP transport uses to protect outbound
/// packets and unprotect inbound packets. RFC 4568 §6.1 mandates each
/// side use its own independently generated master key, so this is never
/// built from a single shared key.
pub struct SdesSrtpPair {
    /// Outbound (us → peer); keyed with our own locally-generated master.
    pub send_ctx: SrtpContext,
    /// Inbound (peer → us); keyed with the peer's master.
    pub recv_ctx: SrtpContext,
    /// The negotiated suite.
    pub suite: SrtpCryptoSuite,
}

/// Local key material generated for one offered tag, held until the
/// answer arrives. Zeroized on drop like any other raw SRTP key.
#[derive(Zeroize)]
#[zeroize(drop)]
pub struct OfferedSlot {
    #[zeroize(skip)]
    suite: SrtpCryptoSuite,
    key: Vec<u8>,
    salt: Vec<u8>,
}

/// RFC 4568 SDES key-exchange engine. Constructed in one of two roles
/// (offerer / answerer) corresponding to the SDP offerer/answerer.
pub enum SdesNegotiator {
    /// Offerer, awaiting an answer to its offered crypto attributes.
    Offerer { offered: HashMap<u32, OfferedSlot> },
    /// Answerer, ready to receive an offer.
    Answerer,
}

/// Generate a fresh random master key + salt for the given suite.
/// Returns `(key, salt, base64_inline)`.
fn generate_keysalt(suite: &SrtpCryptoSuite) -> (Vec<u8>, Vec<u8>, String) {
    let mut key = vec![0u8; suite.key_length];
    let mut salt = vec![0u8; SDES_SALT_LEN];
    OsRng.fill_bytes(&mut key);
    OsRng.fill_bytes(&mut salt);
    let mut combined = Vec::with_capacity(key.len() + salt.len());
    combined.extend_from_slice(&key);
    combined.extend_from_slice(&salt);
    let inline = BASE64.encode(&combined);
    (key, salt, inline)
}

/// Decode an `a=crypto:` inline base64 blob into `(key, salt)`, validated
/// to be exactly `suite.key_length + 14` bytes — not truncated, not
/// zero-padded, not silently accepted if short or long.
fn decode_keysalt(inline_b64: &str, suite: &SrtpCryptoSuite) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut combined = BASE64
        .decode(inline_b64)
        .map_err(|e| Error::InvalidParameter(format!("invalid base64 in a=crypto inline: {e}")))?;
    let expected = suite.key_length + SDES_SALT_LEN;
    if combined.len() != expected {
        let got = combined.len();
        combined.zeroize();
        return Err(Error::InvalidParameter(format!(
            "a=crypto inline is {got} bytes, expected exactly {expected} ({} key + {SDES_SALT_LEN} salt) for this suite",
            suite.key_length
        )));
    }
    let key = combined[..suite.key_length].to_vec();
    let salt = combined[suite.key_length..].to_vec();
    combined.zeroize();
    Ok((key, salt))
}

fn build_pair(
    suite: SrtpCryptoSuite,
    our_key: &[u8],
    our_salt: &[u8],
    peer_key: &[u8],
    peer_salt: &[u8],
) -> Result<SdesSrtpPair> {
    let send_ctx = SrtpContext::new(
        suite.clone(),
        SrtpCryptoKey::new(our_key.to_vec(), our_salt.to_vec()),
    )
    .map_err(|e| Error::InvalidParameter(format!("failed to build outbound SrtpContext: {e}")))?;
    let recv_ctx = SrtpContext::new(
        suite.clone(),
        SrtpCryptoKey::new(peer_key.to_vec(), peer_salt.to_vec()),
    )
    .map_err(|e| Error::InvalidParameter(format!("failed to build inbound SrtpContext: {e}")))?;
    Ok(SdesSrtpPair {
        send_ctx,
        recv_ctx,
        suite,
    })
}

impl SdesNegotiator {
    /// Offerer side. Generate a fresh master key for each requested
    /// suite and return the attributes to attach to the SDP offer, one
    /// per suite, with sequential tags (1, 2, ...) in the order supplied.
    pub fn new_offerer(suites: &[SrtpCryptoSuite]) -> Result<(Self, Vec<SdesCryptoAttribute>)> {
        if suites.is_empty() {
            return Err(Error::InvalidParameter(
                "SdesNegotiator::new_offerer requires at least one suite".into(),
            ));
        }
        let mut offered = HashMap::with_capacity(suites.len());
        let mut attrs = Vec::with_capacity(suites.len());
        for (i, suite) in suites.iter().enumerate() {
            let tag = (i + 1) as u32;
            let (key, salt, inline) = generate_keysalt(suite);
            attrs.push(SdesCryptoAttribute::new(tag, suite.clone(), inline));
            offered.insert(
                tag,
                OfferedSlot {
                    suite: suite.clone(),
                    key,
                    salt,
                },
            );
        }
        Ok((SdesNegotiator::Offerer { offered }, attrs))
    }

    /// Answerer side. Construct a negotiator ready to receive an offer.
    pub fn new_answerer() -> Self {
        SdesNegotiator::Answerer
    }

    /// Offerer: the peer's answer arrived. Validate it references one of
    /// our offered tags with the matching suite (RFC 4568 §7.5), decode
    /// the peer's master key, and build the [`SdesSrtpPair`].
    pub fn accept_answer(&self, attr: &SdesCryptoAttribute) -> Result<SdesSrtpPair> {
        attr.reject_unsupported_extensions()?;
        let offered = match self {
            SdesNegotiator::Offerer { offered } => offered,
            SdesNegotiator::Answerer => {
                return Err(Error::InvalidParameter(
                    "SdesNegotiator::accept_answer called on an answerer".into(),
                ))
            }
        };
        let slot = offered.get(&attr.tag).ok_or_else(|| {
            Error::InvalidParameter(format!(
                "answer's a=crypto tag {} was not offered",
                attr.tag
            ))
        })?;
        if slot.suite != attr.suite {
            return Err(Error::InvalidParameter(format!(
                "answer's a=crypto suite {:?} does not match offered tag {} suite {:?}",
                attr.suite, attr.tag, slot.suite
            )));
        }
        let (peer_key, peer_salt) = decode_keysalt(&attr.key_inline, &slot.suite)?;
        build_pair(
            slot.suite.clone(),
            &slot.key,
            &slot.salt,
            &peer_key,
            &peer_salt,
        )
    }

    /// Answerer: process an inbound offer's attributes. Picks the first
    /// suite we support (honouring the offerer's preference order),
    /// preserves its tag, and generates a fresh local master key for the
    /// answer — never the offerer's own key. Returns
    /// `(attribute_to_emit_in_the_answer, pair)`.
    pub fn process_offer(
        &self,
        attrs: &[SdesCryptoAttribute],
    ) -> Result<(SdesCryptoAttribute, SdesSrtpPair)> {
        if !matches!(self, SdesNegotiator::Answerer) {
            return Err(Error::InvalidParameter(
                "SdesNegotiator::process_offer called on an offerer".into(),
            ));
        }
        let chosen = attrs.first().ok_or_else(|| {
            Error::InvalidParameter(
                "no offered a=crypto attribute is supported by this responder".into(),
            )
        })?;
        chosen.reject_unsupported_extensions()?;

        let (peer_key, peer_salt) = decode_keysalt(&chosen.key_inline, &chosen.suite)?;
        let (our_key, our_salt, our_inline) = generate_keysalt(&chosen.suite);

        let pair = build_pair(
            chosen.suite.clone(),
            &our_key,
            &our_salt,
            &peer_key,
            &peer_salt,
        )?;
        Ok((
            SdesCryptoAttribute::new(chosen.tag, chosen.suite.clone(), our_inline),
            pair,
        ))
    }
}

#[cfg(test)]
mod tests;
