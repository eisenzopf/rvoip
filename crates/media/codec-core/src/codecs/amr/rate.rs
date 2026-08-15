//! Rate adaptation: mode-change constraints and codec mode requests.
//!
//! Two separate concerns that both govern which mode an AMR stream uses:
//!
//! - [`ModeChangePolicy`] constrains *our encoder*, honouring the
//!   `mode-change-period` and `mode-change-neighbor` parameters the peer asked
//!   for. Ignoring these breaks interoperation with circuit-switched gateways,
//!   which is the reason RFC 4867 defines them.
//! - [`CmrDamper`] decides when to ask *the peer* to change mode, and — more
//!   importantly — when not to. An undamped CMR sender oscillates audibly.
//!
//! Both are driven in frame-blocks (20 ms) rather than wall-clock time, so they
//! are deterministic and need no clock.

use super::mode::{AmrMode, AmrModeSet};
use crate::error::{CodecError, Result};

/// Enforces the mode-change restrictions negotiated in SDP.
///
/// RFC 4867 defines two, and they compose:
///
/// - `mode-change-period=N`: "changes must be separated by a period of N
///   frame-blocks". N is 1 (unrestricted) or 2.
/// - `mode-change-neighbor=1`: "the sender SHOULD only perform mode changes to
///   the neighboring modes in the active codec mode set". Neighbours are
///   "the ones closest in bit rate to the current mode" — **within the active
///   mode set**, not adjacent mode indices. With a mode-set of `0,4,8`, the
///   neighbours of mode 4 are 0 and 8.
///
/// A request that the policy cannot grant immediately is not an error: the
/// current mode simply stays in effect, and repeating the request walks toward
/// the target one step at a time.
///
/// # Driving contract
///
/// Call [`advance`](Self::advance) exactly once per encoded frame-block, after
/// encoding it:
///
/// ```text
/// loop {
///     policy.request(desired);       // may or may not take effect
///     encode(policy.current());
///     policy.advance();
/// }
/// ```
///
/// Two changes cannot take effect within one frame-block — a frame carries one
/// mode — so a second `request` before the next `advance` is deferred. That is
/// the mechanism behind `mode-change-period`, not a separate restriction.
#[derive(Debug, Clone)]
pub struct ModeChangePolicy {
    mode_set: AmrModeSet,
    period: u8,
    neighbor_only: bool,
    current: AmrMode,
    /// Frame-blocks since the last change. Starts at `period` so the first
    /// request is not artificially delayed — RFC 4867 says "the initial phase
    /// of the interval is arbitrary".
    since_change: u32,
}

impl ModeChangePolicy {
    /// Create a policy.
    ///
    /// # Errors
    ///
    /// Returns an error when `period` is not 1 or 2, when `initial` is not in
    /// `mode_set`, or when the two disagree about the variant.
    pub fn new(
        mode_set: AmrModeSet,
        period: u8,
        neighbor_only: bool,
        initial: AmrMode,
    ) -> Result<Self> {
        if period != 1 && period != 2 {
            return Err(CodecError::invalid_config(format!(
                "AMR mode-change-period must be 1 or 2, got {period}"
            )));
        }
        if !mode_set.contains(initial) {
            return Err(CodecError::invalid_config(format!(
                "initial mode {} is outside the mode-set ({})",
                initial.index(),
                mode_set.to_sdp_value()
            )));
        }
        Ok(Self {
            mode_set,
            period,
            neighbor_only,
            current: initial,
            since_change: u32::from(period),
        })
    }

    /// The mode currently in effect.
    #[must_use]
    pub const fn current(&self) -> AmrMode {
        self.current
    }

    /// Whether a change would be permitted on the next frame-block.
    #[must_use]
    pub const fn can_change_now(&self) -> bool {
        self.since_change >= self.period as u32
    }

    /// Account for one encoded frame-block. Call once per 20 ms frame, after
    /// encoding it. See the type-level driving contract.
    pub const fn advance(&mut self) {
        self.since_change = self.since_change.saturating_add(1);
    }

    /// The mode that would take effect if `desired` were requested now, without
    /// changing any state.
    #[must_use]
    pub fn preview(&self, desired: AmrMode) -> AmrMode {
        // A mode outside the negotiated set must never be used: RFC 4867 says
        // frames in such modes "MUST NOT be sent in any RTP payload".
        if !self.mode_set.contains(desired) {
            return self.current;
        }
        if desired == self.current {
            return self.current;
        }
        if !self.can_change_now() {
            return self.current;
        }
        if self.neighbor_only {
            self.step_toward(desired)
        } else {
            desired
        }
    }

    /// Request `desired` and return the mode now in effect.
    ///
    /// Under `mode-change-neighbor` this moves one step toward the target, so
    /// reaching a distant mode takes several calls — which is exactly the
    /// gradual transition the parameter exists to produce.
    pub fn request(&mut self, desired: AmrMode) -> AmrMode {
        let next = self.preview(desired);
        if next != self.current {
            self.current = next;
            self.since_change = 0;
        }
        next
    }

    /// One step from the current mode toward `desired`, staying inside the
    /// active mode set.
    fn step_toward(&self, desired: AmrMode) -> AmrMode {
        let modes = self.mode_set.modes();
        let Some(current_pos) = modes.iter().position(|&m| m == self.current) else {
            return self.current;
        };
        let Some(target_pos) = modes.iter().position(|&m| m == desired) else {
            return self.current;
        };
        let next_pos = if target_pos > current_pos {
            current_pos + 1
        } else {
            current_pos - 1
        };
        modes.get(next_pos).copied().unwrap_or(self.current)
    }
}

/// Decides when to send a codec mode request, and damps how often.
///
/// Modelled on the behaviour rtpengine documents, which exists because naive
/// CMR senders oscillate: over each interval it records which modes actually
/// arrived, and at the end asks for **at most one** mode change — the
/// next-highest allowed mode that was never seen. Asking for one step rather
/// than the maximum avoids demanding a rate the peer may have good reason not
/// to use.
///
/// Driven in frame-blocks so it needs no clock: `interval_frames` of 250 is
/// five seconds at 20 ms per frame.
#[derive(Debug, Clone)]
pub struct CmrDamper {
    mode_set: AmrModeSet,
    interval_frames: u32,
    elapsed: u32,
    /// Bit `n` set means mode `n` arrived during this interval.
    seen: u16,
}

impl CmrDamper {
    /// Create a damper.
    ///
    /// # Errors
    ///
    /// Returns an error when `interval_frames` is zero, which would emit a
    /// request on every frame — the opposite of damping.
    pub fn new(mode_set: AmrModeSet, interval_frames: u32) -> Result<Self> {
        if interval_frames == 0 {
            return Err(CodecError::invalid_config(
                "CMR interval must be at least one frame-block",
            ));
        }
        Ok(Self {
            mode_set,
            interval_frames,
            elapsed: 0,
            seen: 0,
        })
    }

    /// Record a speech mode received from the peer.
    pub const fn observe(&mut self, mode: AmrMode) {
        if self.mode_set.contains(mode) {
            self.seen |= 1u16 << mode.index();
        }
    }

    /// Account for one received frame-block, returning a CMR to send if the
    /// interval just closed and a change is worth requesting.
    ///
    /// Returns `None` for most calls. A `Some(mode)` should be placed in the
    /// CMR field of the next outgoing payload.
    pub fn advance(&mut self) -> Option<AmrMode> {
        self.elapsed += 1;
        if self.elapsed < self.interval_frames {
            return None;
        }
        self.elapsed = 0;
        let request = self.next_unused_mode();
        self.seen = 0;
        request
    }

    /// The lowest allowed mode above everything observed this interval.
    ///
    /// `None` when nothing was received (there is no evidence to act on) or
    /// when the peer already used the highest allowed mode.
    fn next_unused_mode(&self) -> Option<AmrMode> {
        if self.seen == 0 {
            // Silence, or a stream we could not interpret. Requesting a change
            // on no evidence is how oscillation starts.
            return None;
        }
        // `seen` is non-zero here, so leading_zeros() is at most 15 and the
        // subtraction cannot underflow.
        let highest_seen =
            15u8.saturating_sub(u8::try_from(self.seen.leading_zeros()).unwrap_or(15));
        self.mode_set
            .modes()
            .into_iter()
            .find(|mode| mode.index() > highest_seen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::AmrVariant;

    const WB: AmrVariant = AmrVariant::WideBand;

    fn mode(index: u8) -> AmrMode {
        AmrMode::new(WB, index).unwrap()
    }

    fn policy(modes: &[u8], period: u8, neighbor: bool, initial: u8) -> ModeChangePolicy {
        ModeChangePolicy::new(
            AmrModeSet::from_indices(WB, modes).unwrap(),
            period,
            neighbor,
            mode(initial),
        )
        .unwrap()
    }

    #[test]
    fn unrestricted_policy_changes_on_every_frame_block() {
        let mut p = policy(&[0, 1, 2, 3, 4], 1, false, 0);
        assert_eq!(p.request(mode(4)), mode(4));
        assert_eq!(p.current(), mode(4));
        // And again on the very next frame.
        p.advance();
        assert_eq!(p.request(mode(1)), mode(1));
    }

    #[test]
    fn a_second_change_within_one_frame_block_is_deferred() {
        // A frame carries exactly one mode, so changing twice before emitting
        // anything is meaningless regardless of mode-change-period.
        let mut p = policy(&[0, 1, 2], 1, false, 0);
        assert_eq!(p.request(mode(2)), mode(2));
        assert_eq!(p.request(mode(1)), mode(2), "no frame boundary has passed");
        p.advance();
        assert_eq!(p.request(mode(1)), mode(1));
    }

    #[test]
    fn period_two_separates_changes_by_two_frame_blocks() {
        // "a value of 2 allows the sender to change mode every second
        // frame-block": a change at frame t means the next is allowed at t+2.
        let mut p = policy(&[0, 1, 2], 2, false, 0);
        // The initial phase is arbitrary, so the first change is allowed.
        assert_eq!(p.request(mode(2)), mode(2));

        p.advance(); // frame t+1
        assert!(!p.can_change_now());
        assert_eq!(p.request(mode(0)), mode(2), "only one frame-block elapsed");

        p.advance(); // frame t+2
        assert!(p.can_change_now());
        assert_eq!(p.request(mode(0)), mode(0));
    }

    #[test]
    fn period_one_allows_a_change_on_every_frame_block() {
        let mut p = policy(&[0, 1, 2], 1, false, 0);
        for target in [2u8, 0, 2, 1, 0] {
            assert_eq!(p.request(mode(target)), mode(target));
            p.advance();
        }
    }

    #[test]
    fn period_two_permits_half_as_many_changes_as_period_one() {
        // The property that matters, stated directly rather than by counting
        // frames in one hand-written trace.
        let changes = |period: u8| {
            let mut p = policy(&[0, 1], period, false, 0);
            let mut changes = 0;
            let mut want = mode(1);
            for _ in 0..40 {
                let before = p.current();
                if p.request(want) != before {
                    changes += 1;
                    want = if want == mode(1) { mode(0) } else { mode(1) };
                }
                p.advance();
            }
            changes
        };
        assert_eq!(changes(1), 40);
        assert_eq!(changes(2), 20);
    }

    #[test]
    fn neighbor_restriction_steps_through_the_active_set_not_mode_indices() {
        // Mode-set 0,4,8: the neighbours of 4 are 0 and 8, not 3 and 5.
        // Getting this wrong produces modes outside the negotiated set.
        let mut p = policy(&[0, 4, 8], 1, true, 0);
        assert_eq!(p.request(mode(8)), mode(4), "one step, to the next in set");
        p.advance();
        assert_eq!(p.request(mode(8)), mode(8), "second step reaches it");
        p.advance();
        assert_eq!(p.request(mode(0)), mode(4), "and back down one step");
    }

    #[test]
    fn neighbor_restriction_walks_to_a_distant_target_over_several_requests() {
        let mut p = policy(&[0, 1, 2, 3, 4, 5, 6, 7, 8], 1, true, 0);
        let target = mode(8);
        for expected in 1..=8u8 {
            p.advance();
            assert_eq!(p.request(target), mode(expected));
        }
        // Having arrived, further requests are stable.
        p.advance();
        assert_eq!(p.request(target), mode(8));
    }

    #[test]
    fn modes_outside_the_negotiated_set_are_never_selected() {
        // RFC 4867: frames in modes outside the set "MUST NOT be sent in any
        // RTP payload".
        let mut p = policy(&[0, 2, 4], 1, false, 0);
        assert_eq!(p.request(mode(3)), mode(0), "mode 3 is not in the set");
        assert_eq!(p.current(), mode(0));
        assert_eq!(p.request(mode(4)), mode(4));
    }

    #[test]
    fn preview_does_not_mutate() {
        let p = policy(&[0, 1, 2], 1, false, 0);
        assert_eq!(p.preview(mode(2)), mode(2));
        assert_eq!(p.current(), mode(0), "preview must not change state");
    }

    #[test]
    fn policy_construction_validates_its_inputs() {
        let set = AmrModeSet::from_indices(WB, &[0, 1]).unwrap();
        assert!(ModeChangePolicy::new(set.clone(), 3, false, mode(0)).is_err());
        assert!(ModeChangePolicy::new(set.clone(), 0, false, mode(0)).is_err());
        // Initial mode must be in the set.
        assert!(ModeChangePolicy::new(set, 1, false, mode(5)).is_err());
    }

    #[test]
    fn damper_emits_at_most_one_request_per_interval() {
        let set = AmrModeSet::from_indices(WB, &[0, 1, 2]).unwrap();
        let mut d = CmrDamper::new(set, 5).unwrap();

        // Peer only ever sends mode 0.
        let mut requests = Vec::new();
        for _ in 0..20 {
            d.observe(mode(0));
            if let Some(request) = d.advance() {
                requests.push(request.index());
            }
        }
        // Four intervals of five frames, one request each, always the
        // next-highest unused mode.
        assert_eq!(requests, vec![1, 1, 1, 1]);
    }

    #[test]
    fn damper_asks_only_for_the_next_step_up() {
        // "Only the next-highest bitrate mode that was not received will ever
        // be requested" — asking for the maximum would be a demand the peer may
        // have declined for a reason.
        let set = AmrModeSet::from_indices(WB, &[0, 3, 6, 8]).unwrap();
        let mut d = CmrDamper::new(set, 2).unwrap();
        d.observe(mode(0));
        d.observe(mode(3));
        assert_eq!(d.advance(), None);
        assert_eq!(d.advance().map(AmrMode::index), Some(6));
    }

    #[test]
    fn damper_is_silent_when_the_peer_already_uses_the_top_mode() {
        let set = AmrModeSet::from_indices(WB, &[0, 1, 2]).unwrap();
        let mut d = CmrDamper::new(set, 2).unwrap();
        d.observe(mode(2));
        assert_eq!(d.advance(), None);
        assert_eq!(d.advance(), None);
    }

    #[test]
    fn damper_is_silent_with_no_evidence() {
        // A silent interval — DTX, or total loss — is not a reason to demand a
        // rate change. Acting on no evidence is how CMR oscillation starts.
        let set = AmrModeSet::all(WB);
        let mut d = CmrDamper::new(set, 3).unwrap();
        assert_eq!(d.advance(), None);
        assert_eq!(d.advance(), None);
        assert_eq!(d.advance(), None, "interval closed with nothing observed");
    }

    #[test]
    fn damper_ignores_modes_outside_the_negotiated_set() {
        // A peer sending an out-of-set mode is misbehaving; it must not skew
        // our view of what it can do.
        let set = AmrModeSet::from_indices(WB, &[0, 1]).unwrap();
        let mut d = CmrDamper::new(set, 2).unwrap();
        d.observe(mode(8));
        assert_eq!(d.advance(), None);
        // Nothing valid was seen, so no request.
        assert_eq!(d.advance(), None);
    }

    #[test]
    fn damper_resets_its_observations_each_interval() {
        let set = AmrModeSet::from_indices(WB, &[0, 1, 2]).unwrap();
        let mut d = CmrDamper::new(set, 2).unwrap();

        // Interval 1: peer reaches mode 2, nothing to ask for.
        d.observe(mode(2));
        d.advance();
        assert_eq!(d.advance(), None);

        // Interval 2: peer drops to mode 0. The earlier observation must not
        // linger, or we would never notice the regression.
        d.observe(mode(0));
        d.advance();
        assert_eq!(d.advance().map(AmrMode::index), Some(1));
    }

    #[test]
    fn damper_rejects_a_zero_interval() {
        assert!(CmrDamper::new(AmrModeSet::all(WB), 0).is_err());
    }

    #[test]
    fn damper_and_policy_compose_without_oscillating() {
        // End to end: the peer's requests drive our encoder through the policy,
        // and a neighbour restriction makes the walk gradual rather than jumpy.
        let set = AmrModeSet::from_indices(WB, &[0, 2, 4, 6]).unwrap();
        let mut p = ModeChangePolicy::new(set.clone(), 2, true, mode(0)).unwrap();
        let mut d = CmrDamper::new(set, 4).unwrap();

        let mut trajectory = vec![p.current().index()];
        for _ in 0..40 {
            d.observe(p.current());
            if let Some(request) = d.advance() {
                p.request(request);
            }
            p.advance();
            trajectory.push(p.current().index());
        }

        // Monotone, one step at a time, ending at the top of the set.
        assert_eq!(*trajectory.last().unwrap(), 6);
        for pair in trajectory.windows(2) {
            let step = i32::from(pair[1]) - i32::from(pair[0]);
            assert!(
                step == 0 || step == 2,
                "jumped from {} to {}",
                pair[0],
                pair[1]
            );
        }
    }
}
