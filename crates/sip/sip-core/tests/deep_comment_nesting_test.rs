//! Regression test: deeply nested comments in eagerly parsed headers
//! (`Server`, `User-Agent`, `Retry-After`) must not overflow the stack.
//!
//! A single ~6 KB datagram carrying `Server:` followed by thousands of `(`,
//! folded across continuation lines so each physical line stays under the
//! framing limit, used to recurse in the comment parser until a 2 MiB tokio
//! worker stack overflowed and the process aborted.

use rvoip_sip_core::framing::{inspect_sip_frame_with_policy, SipFramingPolicy};
use rvoip_sip_core::parse_message;

fn folded_message(header: &str, depth: usize, balanced: bool) -> Vec<u8> {
    let value = if balanced {
        format!("{}x{}", "(".repeat(depth), ")".repeat(depth))
    } else {
        "(".repeat(depth)
    };
    // Fold into 4000-byte physical lines, as the attack did.
    let chars: Vec<char> = value.chars().collect();
    let folded = chars
        .chunks(4000)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\r\n ");
    format!(
        "OPTIONS sip:a@b.c SIP/2.0\r\n\
         Via: SIP/2.0/UDP 1.2.3.4;branch=z9hG4bK1\r\n\
         From: <sip:x@y>;tag=1\r\n\
         To: <sip:a@b.c>\r\n\
         Call-ID: 1\r\n\
         CSeq: 1 OPTIONS\r\n\
         Max-Forwards: 70\r\n\
         {header}: {folded}\r\n\
         Content-Length: 0\r\n\r\n"
    )
    .into_bytes()
}

/// Parse on a thread with the tokio worker default stack (2 MiB).
fn parse_on_worker_sized_stack(msg: Vec<u8>) {
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            // Accepting or rejecting the message are both fine; aborting is not.
            let _ = parse_message(&msg);
        })
        .unwrap()
        .join()
        .expect("parser thread must not die");
}

#[test]
fn deep_unbalanced_comment_passes_framing_but_does_not_crash() {
    for header in ["Server", "User-Agent"] {
        let msg = folded_message(header, 6000, false);
        // The folded header gets through stream framing, so the parser is
        // the only line of defence.
        assert!(inspect_sip_frame_with_policy(&msg, SipFramingPolicy::Stream).is_ok());
        parse_on_worker_sized_stack(msg);
    }
}

#[test]
fn deep_balanced_comment_does_not_crash() {
    for header in ["Server", "User-Agent"] {
        parse_on_worker_sized_stack(folded_message(header, 20_000, true));
    }
}

#[test]
fn deep_retry_after_comment_does_not_crash() {
    let value_prefix = "120 ";
    let mut msg = String::from_utf8(folded_message("Retry-After", 6000, false)).unwrap();
    msg = msg.replacen("Retry-After: ", &format!("Retry-After: {value_prefix}"), 1);
    parse_on_worker_sized_stack(msg.into_bytes());
}
