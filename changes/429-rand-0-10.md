---
section: Breaking changes
breaking: true
---
**rand 0.8 → 0.10.** `rvoip_rtp_core::RtcpIntervalParams::randomized`
takes any rand 0.10 `Rng` (the rand_core trait formerly called
`RngCore`). Migration: pass a rand 0.10 generator, such as `rand::rng()`.
