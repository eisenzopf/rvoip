//! Substrate-level tests per `UCTP_IMPLEMENTATION_PLAN.md` §3.8.
//!
//! Tests live in the inline `#[cfg(test)] mod tests` blocks of
//! `substrate::datagram`, `substrate::framing`, and `substrate::correlation`
//! (small enough to keep co-located with the code).
//!
//! This integration-test file exists only as an aggregation point — it
//! re-exercises a few of the key paths end-to-end so a single
//! `cargo test -p rvoip-uctp --test substrate` is a meaningful smoke.

use bytes::Bytes;
use rvoip_uctp::substrate::{pack, unpack, MediaDatagram};

#[test]
fn datagram_pack_unpack_smoke() {
    let d = MediaDatagram {
        flags: 0xa5,
        stream_local_id: 0xcafe,
        seq: 0xdeadbeef,
        payload: Bytes::from_static(&[0x12, 0x34, 0x56]),
    };
    let wire = pack(&d);
    assert_eq!(wire.len(), 8 + 3);
    // ver=1, flags=a5, stream_local_id=cafe (BE), seq=deadbeef (BE)
    assert_eq!(wire[0], 1);
    assert_eq!(wire[1], 0xa5);
    assert_eq!(&wire[2..4], &[0xca, 0xfe]);
    assert_eq!(&wire[4..8], &[0xde, 0xad, 0xbe, 0xef]);
    let d2 = unpack(&wire).unwrap();
    assert_eq!(d, d2);
}
