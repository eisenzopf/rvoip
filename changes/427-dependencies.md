---
section: Changed
breaking: false
---
**reqwest** 0.12 → 0.13, **tokio-tungstenite** 0.29 → 0.30, **opus** 0.3 →
0.4 (its `opusic-sys` binding builds the bundled libopus and replaces the
unmaintained `audiopus_sys`), and `dtls` 0.17.2 → 0.17.3 in the lockfile.
reqwest 0.13's `rustls` feature turns on aws-lc-rs, which conflicts with the
workspace's ring-only rustls, so every rvoip crate uses `rustls-no-provider`
(plus `form` where it posts forms) and installs ring before building a
client; the tests that build their own clients do the same. The unused
`tower` 0.4, `tower-http` 0.5 and `mio` 0.8 workspace dependency entries are
removed.
