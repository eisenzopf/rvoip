---
section: Changed
breaking: false
---
Unlocked by the Rust 1.95 minimum: **sqlx** 0.8 → 0.9 (runtime SQL built from
fixed fragments is now marked with `AssertSqlSafe`; vcon-postgres keeps the
ring rustls provider through `tls-rustls-ring-webpki`), **serial_test** 3 → 4
(dev only), **parking_lot** 0.12 → 0.13, **sysinfo** 0.31 → 0.40 (perf
harness only), and the AWS SDK pins above, which take `aws-config` off the
`crypto-bigint` 0.5 path (RUSTSEC-2026-0387). New Rust 1.95 Clippy lints are
fixed in `rvoip-codec-core` and the perf harness.
