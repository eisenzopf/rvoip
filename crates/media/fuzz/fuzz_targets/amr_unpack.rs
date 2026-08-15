#![no_main]
//! Fuzz RFC 4867 AMR/AMR-WB depacketization — the inbound path that parses
//! attacker-controlled RTP payloads.
//!
//! `unpack` is fallible, so a rejection is a correct outcome; the property
//! under test is that it never panics, never hangs, and never allocates
//! unboundedly. The parser is bit-granular, walks a length-prefixed
//! table-of-contents chain whose entries determine how much data follows, and
//! reorders octets under robust sorting — plenty of room for an index to run
//! off the end.
//!
//! The first input byte selects the negotiated configuration, since which
//! framing and extensions are in force changes the parse completely and a
//! fuzzer cannot discover that from the payload alone.

use libfuzzer_sys::fuzz_target;
use codec_core::codecs::amr::{AmrPayloadCodec, AmrPayloadConfig, AmrVariant};

fuzz_target!(|data: &[u8]| {
    // Bound the input: a real RTP payload cannot exceed the MTU by much, and
    // huge inputs only slow the fuzzer down without reaching new code.
    if data.len() > 4096 {
        return;
    }
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };

    let variant = if selector & 1 == 0 {
        AmrVariant::NarrowBand
    } else {
        AmrVariant::WideBand
    };

    let mut config = if selector & 2 == 0 {
        AmrPayloadConfig::bandwidth_efficient(variant)
    } else {
        AmrPayloadConfig::octet_aligned(variant)
    };
    if selector & 4 != 0 {
        config = config.with_crc();
    }
    if selector & 8 != 0 {
        config = config.with_robust_sorting();
    }
    if selector & 16 != 0 {
        config = config.with_interleaving();
    }

    let Ok(codec) = AmrPayloadCodec::new(config) else {
        return;
    };

    if let Ok(packet) = codec.unpack(payload) {
        // Anything that parsed must re-pack: the depacketizer must not be able
        // to produce a packet the packetizer rejects, or the relay path would
        // fail on traffic it had just accepted.
        let repacked = codec
            .pack(&packet)
            .expect("a packet that unpacked must re-pack");

        // And that re-packed payload must parse back to the same packet.
        // Byte-for-byte equality with the input is not required — the input may
        // carry non-zero padding, which RFC 4867 says to ignore — but the
        // decoded meaning must be stable.
        let reparsed = codec
            .unpack(&repacked)
            .expect("a payload we produced must parse");
        assert_eq!(packet, reparsed, "AMR pack/unpack is not idempotent");
    }
});
