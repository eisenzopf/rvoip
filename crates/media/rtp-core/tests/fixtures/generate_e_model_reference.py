#!/usr/bin/env python3
"""Regenerate E-model vectors from the cited ITU-T equations."""

R_FACTOR_MAX = 93.2
CODECS = {
    "g711": (0.0, 25.1),
    "g722_approx": (13.0, 21.0),
    "g729a_vad": (11.0, 19.0),
    "g723_1_63k_vad": (15.0, 16.1),
    "opus_24k_approx": (7.0, 24.0),
}
CASES = (
    ("g711", 0.0, 0.0),
    ("g711", 25.0, 1.0),
    ("g711", 25.0, 5.0),
    ("g711", 100.0, 10.0),
    ("g722_approx", 25.0, 1.0),
    ("g729a_vad", 25.0, 1.0),
    ("g723_1_63k_vad", 150.0, 5.0),
    ("opus_24k_approx", 50.0, 2.0),
)


def mos(r_factor):
    """Apply ITU-T G.107 Annex B equation B-4."""
    r_factor = min(max(r_factor, 0.0), R_FACTOR_MAX)
    if r_factor < 6.52:
        return 1.0

    score = 1.0 + 0.035 * r_factor
    score += 0.000_007 * r_factor * (r_factor - 60.0) * (100.0 - r_factor)
    return min(max(score, 1.0), 4.5)


def evaluate(codec_name, delay_ms, loss_percent):
    """Apply G.107 7-29 and the documented reduced delay approximation."""
    ie, bpl = CODECS[codec_name]
    ie_eff = ie + (95.0 - ie) * loss_percent / (loss_percent + bpl)
    listening_r = min(max(R_FACTOR_MAX - ie_eff, 0.0), R_FACTOR_MAX)
    delay_impairment = 0.024 * delay_ms + 0.11 * max(delay_ms - 177.3, 0.0)
    conversational_r = min(max(listening_r - delay_impairment, 0.0), R_FACTOR_MAX)
    return conversational_r, mos(listening_r), mos(conversational_r)


print("codec,one_way_delay_ms,loss_percent,r_factor,mos_lq,mos_cq")
for codec, delay, loss in CASES:
    r_factor, mos_lq, mos_cq = evaluate(codec, delay, loss)
    print(f"{codec},{delay:.1f},{loss:.1f},{r_factor:.4f},{mos_lq:.4f},{mos_cq:.4f}")
