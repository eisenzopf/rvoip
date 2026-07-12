//! Public-API conformance vectors for the UCTP 0.2 media framing contract.

use bytes::Bytes;
use rvoip_uctp::substrate::{pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload};
use rvoip_uctp::{
    UCTP_COMPATIBILITY, UCTP_DATAGRAM_VERSION, UCTP_ENVELOPE_VERSION, UCTP_RAW_QUIC_ALPN_BYTES,
};

const FULL_RTP_GOLDEN: &[u8] = &[
    0x01, 0xa5, 0x12, 0x34, 0x01, 0x02, 0x03, 0x04, // UCTP header
    0x80, 0x6f, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, // RTP V/PT, seq, timestamp
    0x01, 0x23, 0x45, 0x67, // RTP SSRC
    0xde, 0xad, 0xbe, 0xef, // codec payload
];

#[test]
fn public_checked_api_matches_full_rtp_golden_vector() {
    let datagram = RtpDatagram {
        flags: 0xa5,
        stream_local_id: 0x1234,
        seq: 0x0102_0304,
        rtp: RtpMediaPayload {
            payload: Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]),
            payload_type: 111,
            sequence_number: 0x4567,
            timestamp: 0x89ab_cdef,
            ssrc: 0x0123_4567,
        },
    };

    let encoded = pack_rtp_datagram(&datagram).unwrap();
    assert_eq!(encoded.as_ref(), FULL_RTP_GOLDEN);
    assert_eq!(unpack_rtp_datagram(FULL_RTP_GOLDEN).unwrap(), datagram);
}

#[test]
fn compatibility_descriptor_matches_the_golden_wire() {
    assert_eq!(UCTP_DATAGRAM_VERSION, FULL_RTP_GOLDEN[0]);
    assert_eq!(UCTP_ENVELOPE_VERSION, 1);
    assert_eq!(UCTP_RAW_QUIC_ALPN_BYTES, b"uctp/1");
    assert!(UCTP_COMPATIBILITY.supports_datagram(FULL_RTP_GOLDEN[0]));
    assert!(UCTP_COMPATIBILITY.supports_envelope(UCTP_ENVELOPE_VERSION));
}

#[test]
fn packet_capture_fixture_contains_the_authoritative_full_rtp_datagram() {
    let capture = include_str!("fixtures/uctp_full_rtp.pcap.hex")
        .split_ascii_whitespace()
        .map(|octet| u8::from_str_radix(octet, 16).unwrap())
        .collect::<Vec<_>>();

    // Classic little-endian PCAP, one Ethernet/IPv4/UDP packet. The fixture
    // represents the decrypted/exported UCTP application datagram so capture
    // tooling can assert the same bytes as the typed codec API.
    assert_eq!(&capture[..4], &[0xd4, 0xc3, 0xb2, 0xa1]);
    assert_eq!(u32::from_le_bytes(capture[20..24].try_into().unwrap()), 1);
    let captured_len = u32::from_le_bytes(capture[32..36].try_into().unwrap()) as usize;
    assert_eq!(captured_len, 66);
    assert_eq!(capture.len(), 24 + 16 + captured_len);
    assert_eq!(&capture[52..54], &[0x08, 0x00]); // Ethernet: IPv4
    assert_eq!(capture[63], 17); // IPv4 protocol: UDP

    let udp_payload_offset = 24 + 16 + 14 + 20 + 8;
    let wire = &capture[udp_payload_offset..];
    assert_eq!(wire, FULL_RTP_GOLDEN);
    let parsed = unpack_rtp_datagram(wire).unwrap();
    assert_eq!(parsed.stream_local_id, 0x1234);
    assert_eq!(parsed.rtp.payload.as_ref(), &[0xde, 0xad, 0xbe, 0xef]);
}
