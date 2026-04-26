//! SDP negotiation and media tracking
//!
//! This module handles SDP offer/answer negotiation within dialogs.

pub mod negotiation;
pub mod offer_answer;
pub mod media_tracking;

// Sprint 3 C2 — RFC 3264 §6 matcher.
pub use negotiation::{
    match_offer, AnswerCapabilities, MatchError, MediaLineMatch, OfferAnswerMatch,
};