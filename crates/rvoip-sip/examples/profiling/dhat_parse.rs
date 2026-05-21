//! Heap profile for `parse_message` + `to_bytes`.
//!
//! Run with:
//!
//! ```bash
//! cargo run --release --features dhat -p rvoip-sip --example profiling_dhat_parse
//! ```
//!
//! Emits `dhat-heap.json` in the CWD. Open at
//! <https://nnethercote.github.io/dh_view/dh_view.html>.
//!
//! Gated behind `--features dhat` so the global allocator swap never
//! leaks into `cargo test` runs.

#![cfg(feature = "dhat")]

use rvoip_sip_core::parse_message;
use std::hint::black_box;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

const ITERATIONS: usize = 50_000;

const INVITE: &[u8] = b"INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bKdhat\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=dhat\r\n\
Call-ID: dhat-invite@pc33.atlanta.example.com\r\n\
CSeq: 1 INVITE\r\n\
Contact: <sip:alice@pc33.atlanta.example.com>\r\n\
Content-Length: 0\r\n\r\n";

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    println!(
        "[dhat_parse] running {} parse+to_bytes cycles...",
        ITERATIONS
    );
    for _ in 0..ITERATIONS {
        let parsed = parse_message(black_box(INVITE)).expect("parse");
        let bytes = parsed.to_bytes();
        black_box(bytes);
    }
    println!("[dhat_parse] done — dhat-heap.json written to CWD");
}
