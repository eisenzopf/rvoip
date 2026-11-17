//! New-call admission on rvoip's transaction-layer gate.
//!
//! rvoip's [`NewCallAdmission`] decides every new dialog-forming INVITE
//! before any transaction, dialog, session or media state exists, and
//! answers a rejection with a stateless `503 Service Unavailable` (on the TLS
//! connection reader for TLS, on the transaction dispatch path for UDP). Its
//! rate limit is a lock-free GCRA. The DUT uses it for both shedding rules of
//! the contract:
//!
//! - `max-cps`: a rate limit of `max_cps` with a one-second burst (`max_cps`
//!   calls back to back), its 503s carrying `Retry-After`.
//! - Drain: the gate's level closes, so every new INVITE gets a 503 with
//!   `Retry-After` while in-dialog requests (ACK, BYE) still pass.
//!
//! The gate always runs in [`AdmissionMode::Enforce`]; with `admission: off`
//! it has no rate limit and an open level, so it admits everything until the
//! drain closes it. Keeping it enforcing from the start is what attaches the
//! TLS ingress hook, so drain 503s over TLS are also answered on the reader.

use rvoip_sip_dialog::transaction::{
    AdmissionLevel, AdmissionMode, NewCallAdmission, NewCallAdmissionConfig, NewCallRateLimit,
};

use crate::config::Admission;

/// `Retry-After` on drain 503s when admission is `off`.
pub const DRAIN_RETRY_AFTER_SECS: u32 = 1;

/// The gate configuration for `admission`.
pub fn gate_config(admission: Admission) -> NewCallAdmissionConfig {
    let config = NewCallAdmissionConfig::new(AdmissionMode::Enforce);
    match admission {
        Admission::Off => config,
        Admission::MaxCps {
            max_cps,
            retry_after_secs,
        } => config.with_rate_limit(
            // A one-second burst: the GCRA tolerance is `max_cps` emission
            // intervals.
            NewCallRateLimit::new(max_cps, max_cps).with_retry_after_secs(retry_after_secs),
        ),
    }
}

/// `Retry-After` on the 503s sent while draining.
pub fn drain_retry_after(admission: Admission) -> u32 {
    match admission {
        Admission::Off => DRAIN_RETRY_AFTER_SECS,
        Admission::MaxCps {
            retry_after_secs, ..
        } => retry_after_secs,
    }
}

/// Stop accepting new calls: every new INVITE from now on gets a 503.
pub fn close_for_drain(gate: &NewCallAdmission, admission: Admission) {
    gate.set_mode(AdmissionMode::Enforce);
    gate.set_level(AdmissionLevel::closed(Some(drain_retry_after(admission))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_dialog::transaction::AdmissionDecision;
    use std::time::Duration;

    fn decide_many(gate: &NewCallAdmission, prefix: &str, count: usize) -> (usize, usize) {
        let mut admitted = 0;
        let mut rejected = 0;
        for index in 0..count {
            match gate.decide(&format!("{prefix}-{index}")) {
                AdmissionDecision::Admit => admitted += 1,
                AdmissionDecision::Reject { .. } => rejected += 1,
                _ => unreachable!("decisions are admit or reject"),
            }
        }
        (admitted, rejected)
    }

    #[test]
    fn off_admits_everything_until_the_drain_closes_the_gate() {
        let gate = NewCallAdmission::new(gate_config(Admission::Off));
        assert_eq!(gate.stats().mode, AdmissionMode::Enforce);
        assert_eq!(gate.stats().rate_limit, None);
        assert_eq!(decide_many(&gate, "off", 10_000), (10_000, 0));

        close_for_drain(&gate, Admission::Off);
        assert_eq!(
            gate.decide("late"),
            AdmissionDecision::Reject {
                retry_after_secs: Some(DRAIN_RETRY_AFTER_SECS)
            }
        );
        assert_eq!(gate.stats().rejected, 1);
    }

    #[test]
    fn max_cps_admits_a_one_second_burst_then_sheds_with_retry_after() {
        let admission = Admission::MaxCps {
            max_cps: 200.0,
            retry_after_secs: 3,
        };
        let gate = NewCallAdmission::new(gate_config(admission));
        // Back to back: the burst (max_cps, plus the GCRA's one emission
        // interval of slack) passes, the rest is shed.
        let (admitted, rejected) = decide_many(&gate, "burst", 1_000);
        assert!(
            (200..=202).contains(&admitted),
            "a one-second burst admits about max_cps calls, got {admitted}"
        );
        assert_eq!(admitted + rejected, 1_000);
        assert_eq!(
            gate.decide("shed"),
            AdmissionDecision::Reject {
                retry_after_secs: Some(3)
            }
        );
        let stats = gate.stats();
        assert_eq!(stats.rate_limited, stats.rejected);

        // Tokens return at max_cps: after 100 ms about 20 more calls pass.
        std::thread::sleep(Duration::from_millis(100));
        let (refilled, _) = decide_many(&gate, "refill", 200);
        assert!(
            (15..=40).contains(&refilled),
            "100 ms at 200 cps refills about 20, got {refilled}"
        );
    }

    #[test]
    fn max_cps_holds_the_sustained_rate() {
        let gate = NewCallAdmission::new(gate_config(Admission::MaxCps {
            max_cps: 1_000.0,
            retry_after_secs: 1,
        }));
        // Drain the burst, then offer 3,000 cps for 300 ms.
        decide_many(&gate, "warm", 2_000);
        let started = std::time::Instant::now();
        let mut admitted = 0;
        let mut offered = 0;
        while started.elapsed() < Duration::from_millis(300) {
            for _ in 0..3 {
                offered += 1;
                if gate.decide(&format!("s-{offered}")) == AdmissionDecision::Admit {
                    admitted += 1;
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let elapsed = started.elapsed().as_secs_f64();
        let rate = f64::from(admitted) / elapsed;
        assert!(
            (700.0..=1_300.0).contains(&rate),
            "sustained admitted rate {rate:.0}/s of {offered} offered"
        );
    }

    #[test]
    fn drain_keeps_the_max_cps_retry_after() {
        let admission = Admission::MaxCps {
            max_cps: 50.0,
            retry_after_secs: 9,
        };
        assert_eq!(drain_retry_after(admission), 9);
        let gate = NewCallAdmission::new(gate_config(admission));
        close_for_drain(&gate, admission);
        assert_eq!(
            gate.decide("draining"),
            AdmissionDecision::Reject {
                retry_after_secs: Some(9)
            }
        );
    }
}
