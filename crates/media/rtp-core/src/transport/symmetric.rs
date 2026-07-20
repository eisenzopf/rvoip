//! Symmetric-RTP destination learning with bounded rebinding.
//!
//! A peer behind NAT may send RTP from a different socket than the address in
//! SDP.  The first valid RTP packet therefore latches the observed source.
//! Moving an established latch is deliberately stricter: the source must pass
//! a short same-SSRC/sequence probation and the number of moves is bounded.
//! This does not replace SRTP authentication, but it prevents a single spoofed
//! UDP packet from redirecting an established plain-RTP call.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

/// Policy for learning and rebinding a symmetric-RTP destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SymmetricRtpPolicy {
    /// Whether validated inbound RTP may update the outbound destination.
    pub enabled: bool,
    /// Permit a rebind to a different IP address. The default only permits a
    /// source-port change on the already-latched IP.
    pub allow_ip_change: bool,
    /// Consecutive packets required from a new source before it becomes the
    /// outbound destination.
    pub probation_packets: u8,
    /// Maximum accepted destination changes after the initial latch.
    pub max_rebindings: u8,
    /// Maximum time allowed to complete one probation sequence.
    pub rebind_window: Duration,
    /// Largest forward RTP sequence-number jump accepted during probation.
    pub max_sequence_jump: u16,
}

impl Default for SymmetricRtpPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_ip_change: false,
            probation_packets: 3,
            max_rebindings: 2,
            rebind_window: Duration::from_secs(2),
            max_sequence_jump: 512,
        }
    }
}

impl SymmetricRtpPolicy {
    /// A policy that never learns a destination from inbound RTP.
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            allow_ip_change: false,
            probation_packets: 1,
            max_rebindings: 0,
            rebind_window: Duration::from_secs(1),
            max_sequence_jump: 1,
        }
    }

    /// Validate bounds before a receive task is started.
    pub fn validate(self) -> Result<(), &'static str> {
        if self.probation_packets == 0 || self.probation_packets > 32 {
            return Err("symmetric RTP probation_packets must be between 1 and 32");
        }
        if self.rebind_window.is_zero() || self.rebind_window > Duration::from_secs(60) {
            return Err("symmetric RTP rebind_window must be greater than zero and at most 60s");
        }
        if self.max_sequence_jump == 0 || self.max_sequence_jump >= 0x8000 {
            return Err("symmetric RTP max_sequence_jump must be between 1 and 32767");
        }
        Ok(())
    }

    /// Permit authenticated deployments to opt into bounded IP changes.
    pub const fn with_ip_change(mut self, enabled: bool) -> Self {
        self.allow_ip_change = enabled;
        self
    }
}

/// Aggregate-only diagnostics. No peer address, SSRC, token, or session ID is
/// exposed, so this snapshot is safe to include in operational diagnostics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SymmetricRtpDiagnostics {
    pub destination_learned: bool,
    pub accepted_rebindings: u64,
    pub probation_packets: u64,
    pub rejected_packets: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SymmetricRtpDecision {
    Accept,
    LatchInitial,
    Probation,
    Rebind,
    Reject,
}

#[derive(Clone, Copy, Debug)]
struct LatchedSource {
    address: SocketAddr,
    ssrc: u32,
    last_sequence: u16,
}

#[derive(Clone, Copy, Debug)]
struct RebindCandidate {
    address: SocketAddr,
    ssrc: u32,
    last_sequence: u16,
    packets: u8,
    started_at: Instant,
}

pub(crate) struct SymmetricRtpLearner {
    policy: SymmetricRtpPolicy,
    latched: Option<LatchedSource>,
    candidate: Option<RebindCandidate>,
    rebindings: u8,
}

impl SymmetricRtpLearner {
    pub(crate) fn new(policy: SymmetricRtpPolicy) -> Self {
        Self {
            policy,
            latched: None,
            candidate: None,
            rebindings: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.latched = None;
        self.candidate = None;
        self.rebindings = 0;
    }

    pub(crate) fn observe(
        &mut self,
        source: SocketAddr,
        ssrc: u32,
        sequence: u16,
        now: Instant,
    ) -> SymmetricRtpDecision {
        if !self.policy.enabled {
            return SymmetricRtpDecision::Accept;
        }

        let Some(mut latched) = self.latched else {
            self.latched = Some(LatchedSource {
                address: source,
                ssrc,
                last_sequence: sequence,
            });
            return SymmetricRtpDecision::LatchInitial;
        };

        if source == latched.address {
            // A packet from the established source wins over an in-progress
            // candidate. Legitimate SSRC changes are learned only on that
            // established tuple.
            latched.ssrc = ssrc;
            latched.last_sequence = sequence;
            self.latched = Some(latched);
            self.candidate = None;
            return SymmetricRtpDecision::Accept;
        }

        if self.rebindings >= self.policy.max_rebindings
            || (!self.policy.allow_ip_change && source.ip() != latched.address.ip())
            || ssrc != latched.ssrc
        {
            self.candidate = None;
            return SymmetricRtpDecision::Reject;
        }

        let candidate = match self.candidate {
            Some(candidate)
                if candidate.address == source
                    && candidate.ssrc == ssrc
                    && now.duration_since(candidate.started_at) <= self.policy.rebind_window =>
            {
                candidate
            }
            _ => {
                let delta = sequence.wrapping_sub(latched.last_sequence);
                if delta == 0 || delta > self.policy.max_sequence_jump {
                    self.candidate = None;
                    return SymmetricRtpDecision::Reject;
                }
                let candidate = RebindCandidate {
                    address: source,
                    ssrc,
                    last_sequence: sequence,
                    packets: 1,
                    started_at: now,
                };
                if self.policy.probation_packets == 1 {
                    self.accept_candidate(candidate);
                    return SymmetricRtpDecision::Rebind;
                }
                self.candidate = Some(candidate);
                return SymmetricRtpDecision::Probation;
            }
        };

        let delta = sequence.wrapping_sub(candidate.last_sequence);
        if delta == 0 || delta > self.policy.max_sequence_jump {
            self.candidate = None;
            return SymmetricRtpDecision::Reject;
        }

        let candidate = RebindCandidate {
            last_sequence: sequence,
            packets: candidate.packets.saturating_add(1),
            ..candidate
        };
        if candidate.packets >= self.policy.probation_packets {
            self.accept_candidate(candidate);
            SymmetricRtpDecision::Rebind
        } else {
            self.candidate = Some(candidate);
            SymmetricRtpDecision::Probation
        }
    }

    fn accept_candidate(&mut self, candidate: RebindCandidate) {
        self.latched = Some(LatchedSource {
            address: candidate.address,
            ssrc: candidate.ssrc,
            last_sequence: candidate.last_sequence,
        });
        self.candidate = None;
        self.rebindings = self.rebindings.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(ip: [u8; 4], port: u16) -> SocketAddr {
        SocketAddr::from((ip, port))
    }

    #[test]
    fn rebind_requires_probation_and_preserves_same_ip_by_default() {
        let now = Instant::now();
        let mut learner = SymmetricRtpLearner::new(SymmetricRtpPolicy::default());
        let original = addr([192, 0, 2, 10], 20_000);
        let rebound = addr([192, 0, 2, 10], 30_000);

        assert_eq!(
            learner.observe(original, 7, 100, now),
            SymmetricRtpDecision::LatchInitial
        );
        assert_eq!(
            learner.observe(rebound, 7, 101, now),
            SymmetricRtpDecision::Probation
        );
        assert_eq!(
            learner.observe(rebound, 7, 102, now),
            SymmetricRtpDecision::Probation
        );
        assert_eq!(
            learner.observe(rebound, 7, 103, now),
            SymmetricRtpDecision::Rebind
        );
        assert_eq!(
            learner.observe(rebound, 7, 104, now),
            SymmetricRtpDecision::Accept
        );
    }

    #[test]
    fn different_ip_ssrc_and_large_jump_are_rejected() {
        let now = Instant::now();
        let mut learner = SymmetricRtpLearner::new(SymmetricRtpPolicy::default());
        let original = addr([192, 0, 2, 10], 20_000);
        assert_eq!(
            learner.observe(original, 7, 100, now),
            SymmetricRtpDecision::LatchInitial
        );
        assert_eq!(
            learner.observe(addr([198, 51, 100, 1], 20_001), 7, 101, now),
            SymmetricRtpDecision::Reject
        );
        assert_eq!(
            learner.observe(addr([192, 0, 2, 10], 20_001), 8, 101, now),
            SymmetricRtpDecision::Reject
        );
        assert_eq!(
            learner.observe(addr([192, 0, 2, 10], 20_001), 7, 10_000, now),
            SymmetricRtpDecision::Reject
        );
    }

    #[test]
    fn established_source_cancels_spoof_candidate() {
        let now = Instant::now();
        let mut learner = SymmetricRtpLearner::new(SymmetricRtpPolicy::default());
        let original = addr([192, 0, 2, 10], 20_000);
        let candidate = addr([192, 0, 2, 10], 20_001);
        learner.observe(original, 7, 100, now);
        assert_eq!(
            learner.observe(candidate, 7, 101, now),
            SymmetricRtpDecision::Probation
        );
        assert_eq!(
            learner.observe(original, 7, 101, now),
            SymmetricRtpDecision::Accept
        );
        assert_eq!(
            learner.observe(candidate, 7, 102, now),
            SymmetricRtpDecision::Probation
        );
    }
}
