//! `rvoip-bench-uas`: the Device Under Test of rvoip's RFC 7501/7502 SIP
//! benchmark (`RFC7502_BENCHMARK.md` §3).
//!
//! A terminating SIP UAS and media endpoint built on rvoip's public API: it
//! answers every INVITE with 200 OK (UDP or TLS; SDP with plain RTP, SDES-SRTP
//! or signaling only), echoes RTP back to its source through rvoip's media
//! plane, sheds new calls with a stateless 503 under `max-cps` admission,
//! reports its state in `ready.json`, `stats.jsonl`, `summary.json` and on
//! Prometheus `/metrics`, and drains when the orchestrator creates the
//! `finish` file. See the crate README for the files and metrics.

pub mod admission;
pub mod calls;
pub mod config;
pub mod drain;
pub mod dut;
pub mod echo;
pub mod metrics_http;
pub mod process;
pub mod stats;
