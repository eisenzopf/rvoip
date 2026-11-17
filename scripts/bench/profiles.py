"""Benchmark rows and lane profiles (crates/sip/rvoip-sip/docs/RFC7502_BENCHMARK.md §1, §7)."""

# E74 row numbering. `media` is the DUT's media mode; the caller sends media
# (and validates the echo) whenever media != 'none'.
ROWS = {
    'R1': {'id': 'R1', 'title': 'Session Establishment Rate, no media (RFC 7502 §6.2 + §6.5)',
           'transport': 'tls', 'media': 'none', 'hold_ms': 0},
    'R2': {'id': 'R2', 'title': 'Session Establishment Rate, with media (RFC 7502 §6.4 + §6.5)',
           'transport': 'tls', 'media': 'srtp', 'hold_ms': 10000},
    'R5': {'id': 'R5', 'title': 'Session Establishment Rate, no media, UDP (RFC 7502 §6.2)',
           'transport': 'udp', 'media': 'none', 'hold_ms': 0},
    'R6': {'id': 'R6', 'title': 'Session Establishment Rate, with media, UDP/RTP (RFC 7502 §6.4)',
           'transport': 'udp', 'media': 'rtp', 'hold_ms': 10000},
}
# R4: overload and recovery at the CPS ceiling C (admission on).
R4 = {'id': 'R4', 'title': 'Overload at the CPS ceiling and recovery (Vapi extension, admission on)',
      'transport': 'tls', 'media': 'srtp', 'hold_ms': 10000,
      'stages': [(0.5, 60), (1.0, 60), (1.5, 60), (2.0, 60), (0.5, 60), (1.0, 60)],
      'warmup_ramp_seconds': 120}

TRIAL = {
    'n': 50000,
    'quantum': 4,
    'minimum_trial_seconds': 180,
    'warmup_seconds': 30,          # at r/2, not scored
    'establishment_threshold_s': 10,
    'invalid_streak_stop': 3,
}

PROFILES = {
    # Every patch release: confirm the recorded R, check RFC 6076 latency.
    'release': {'rows': ['R1', 'R5', 'R2'], 'method': 'confirm',
                'confirm': [0.95, 0.95, 0.95], 'probe_up': 1.05, 'latency_at': 0.90,
                'latency_tolerance': {'relative': 0.25, 'floor_ms': 10.0}, 'budget_hours': 4.5},
    # Every minor release: bisect searches, overload, the IETF report.
    'minor': {'rows': ['R1', 'R2', 'R5', 'R6'], 'method': 'search', 'algorithm': 'bisect',
              'confirmations': 3, 'resolution': 0.02, 'overload': True, 'latency_at': 0.90,
              'report': True, 'budget_hours': 13, 'max_trials': 16, 'reserve_capacity': True},
    # Quarterly: the exact RFC 7502 §4.10 search.
    'full': {'rows': ['R1', 'R2', 'R5', 'R6'], 'method': 'search', 'algorithm': 'rfc7502',
             'confirmations': 10, 'overload': True, 'latency_at': 0.90, 'report': True,
             'budget_hours': 60, 'max_trials': 40, 'reserve_capacity': True},
    # Functional check of a lane: one short trial per row (never a gate; --r0 ROW=sps overrides).
    'smoke': {'rows': ['R1', 'R5', 'R2', 'R6'], 'method': 'smoke', 'seconds': 30, 'warmup_seconds': 10,
              'rates': {'R1': 400, 'R5': 400, 'R2': 200, 'R6': 200}, 'budget_hours': 1.5},
    # Tester diagnostics: smoke-style trials against ea-uas (no DUT), e.g. the callers' receive path.
    'diag': {'rows': ['R2'], 'method': 'smoke', 'seconds': 120, 'warmup_seconds': 30,
             'rates': {'R1': 9600, 'R5': 9600, 'R2': 2400, 'R6': 2400}, 'budget_hours': 1.5, 'ea_uas': True},
    # RFC 7502 §6.1 testbed baseline: the callers against ea-uas (no DUT).
    'baseline': {'rows': ['R1', 'R5', 'R2', 'R6'], 'method': 'search', 'algorithm': 'bisect',
                 'confirmations': 1, 'resolution': 0.05, 'report': True, 'budget_hours': 6,
                 'ea_uas': True, 'max_trials': 9,
                 # Starting points from E74's EA-to-EA baselines (2 callers: TLS >= 9,426, UDP >= 8,548;
                 # 4 callers: TLS/SRTP 10 s >= 4,992, UDP/RTP 10 s >= 5,488).
                 'r0': {'R1': 8000, 'R5': 7200, 'R2': 2400, 'R6': 2400}},
}

TESTER_CAP_FRACTION = 0.67

# On-demand us-west-2 prices (AWS Pricing API, E74 plan 2026-11-01).
PRICE_PER_HOUR = {'c8i.4xlarge': 0.74968, 'c8i.12xlarge': 2.24904, 'c8i.16xlarge': 2.99872,
                  'c8in.24xlarge': 6.53184, 'c8in.16xlarge': 4.35456,
                  'c8in.12xlarge': 3.26592}


def quantize(rate, quantum=TRIAL['quantum']):
    return max(quantum, int(rate) // quantum * quantum)
