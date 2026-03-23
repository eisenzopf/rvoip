//! WebRTC media path integration tests for rtp-core.
//!
//! Covers:
//! - RTP packet construction, serialization, and parsing roundtrips
//! - RTCP packet construction and parsing (SR, RR, BYE, compound)
//! - Sequence number and timestamp handling across packet streams
//! - STUN binding request construction and parsing
//! - ICE candidate types and SDP attribute formatting/parsing
//! - SRTP encrypt/decrypt roundtrip via the production adapter
//! - DTLS-SRTP handshake and key extraction over loopback
//! - Error handling for invalid/malformed packets

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;

// ─── RTP imports ────────────────────────────────────────────────────────────
#[allow(deprecated)]
use rvoip_rtp_core::{
    RtpPacket, RtpHeader, RtpSsrc,
    Error,
};
use rvoip_rtp_core::packet::extension::{
    RtpHeaderExtensions, ExtensionElement,
};

// ─── RTCP imports ───────────────────────────────────────────────────────────
use rvoip_rtp_core::packet::rtcp::{
    RtcpPacket, RtcpSenderReport, RtcpReceiverReport,
    RtcpReportBlock, RtcpGoodbye, NtpTimestamp,
    RtcpCompoundPacket,
};

// ─── STUN imports ───────────────────────────────────────────────────────────
#[allow(deprecated)]
use rvoip_rtp_core::stun::message::{
    StunMessage, TransactionId, BINDING_REQUEST, BINDING_RESPONSE,
    MAGIC_COOKIE, HEADER_SIZE,
};

// ─── ICE imports ────────────────────────────────────────────────────────────
use rvoip_rtp_core::ice::types::{
    CandidateType, IceRole, IceConnectionState, ComponentId,
    IceCandidate, IceCredentials,
};
use rvoip_rtp_core::ice::gather::{compute_priority, generate_foundation};

// ─── SRTP imports ───────────────────────────────────────────────────────────
use rvoip_rtp_core::srtp::adapter::SrtpContextAdapter;
use webrtc_srtp::protection_profile::ProtectionProfile;

// ─── DTLS imports ───────────────────────────────────────────────────────────
use rvoip_rtp_core::dtls::adapter::{
    DtlsAdapterConfig, DtlsConnectionAdapter, DtlsRole,
};

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 1: RTP Packet Basics
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn rtp_packet_construct_and_parse_roundtrip() {
    let payload = Bytes::from_static(b"hello webrtc audio");
    let packet = RtpPacket::new_with_payload(111, 5000, 160_000, 0xDEAD_BEEF, payload.clone());

    let serialized = packet.serialize().expect("serialize");
    let parsed = RtpPacket::parse(&serialized).expect("parse");

    assert_eq!(parsed.header.version, 2);
    assert_eq!(parsed.header.payload_type, 111);
    assert_eq!(parsed.header.sequence_number, 5000);
    assert_eq!(parsed.header.timestamp, 160_000);
    assert_eq!(parsed.header.ssrc, 0xDEAD_BEEF);
    assert_eq!(parsed.payload, payload);
}

#[test]
fn rtp_packet_with_marker_bit() {
    let mut header = RtpHeader::new(96, 1, 320, 0x1234_5678);
    header.marker = true;

    let packet = RtpPacket::new(header, Bytes::from_static(b"frame-start"));
    let serialized = packet.serialize().expect("serialize");
    let parsed = RtpPacket::parse(&serialized).expect("parse");

    assert!(parsed.header.marker, "Marker bit must survive roundtrip");
    assert_eq!(parsed.payload, Bytes::from_static(b"frame-start"));
}

#[test]
fn rtp_packet_with_csrc_list() {
    let mut header = RtpHeader::new(0, 100, 800, 0xAAAA_BBBB);
    header.csrc = vec![0x1111_1111, 0x2222_2222, 0x3333_3333];
    header.cc = 3;

    let packet = RtpPacket::new(header, Bytes::from_static(b"mix"));
    let serialized = packet.serialize().expect("serialize");
    let parsed = RtpPacket::parse(&serialized).expect("parse");

    assert_eq!(parsed.header.cc, 3);
    assert_eq!(parsed.header.csrc.len(), 3);
    assert_eq!(parsed.header.csrc[0], 0x1111_1111);
    assert_eq!(parsed.header.csrc[1], 0x2222_2222);
    assert_eq!(parsed.header.csrc[2], 0x3333_3333);
}

#[test]
fn rtp_packet_with_header_extension() {
    let mut header = RtpHeader::new(96, 42, 960, 0xFEED_FACE);
    header.extension = true;

    let mut ext = RtpHeaderExtensions::new_legacy(0x1234);
    ext.elements.push(ExtensionElement {
        id: 1,
        data: Bytes::from_static(b"extension data"),
    });
    header.extensions = Some(ext);

    let packet = RtpPacket::new(header, Bytes::from_static(b"opus"));
    let serialized = packet.serialize().expect("serialize");
    let parsed = RtpPacket::parse(&serialized).expect("parse");

    assert!(parsed.header.extension);
    let parsed_ext = parsed.header.extensions.expect("extensions present");
    assert_eq!(parsed_ext.profile_id, 0x1234);
    assert!(!parsed_ext.elements.is_empty());
    assert!(
        parsed_ext.elements[0].data.starts_with(b"extension data"),
        "Extension data must start with original bytes"
    );
    assert_eq!(parsed.payload, Bytes::from_static(b"opus"));
}

#[test]
fn rtp_empty_payload_roundtrip() {
    let packet = RtpPacket::new_with_payload(0, 1, 160, 0x0000_0001, Bytes::new());
    let serialized = packet.serialize().expect("serialize");
    let parsed = RtpPacket::parse(&serialized).expect("parse");

    assert!(parsed.payload.is_empty());
    assert_eq!(parsed.header.sequence_number, 1);
}

/// Verify that a stream of packets has monotonically increasing sequence
/// numbers and correct timestamp increments.
#[test]
fn rtp_sequence_and_timestamp_stream() {
    let ssrc: RtpSsrc = 0xCAFE_BABE;
    let clock_rate_samples_per_packet = 960u32; // Opus @ 48 kHz, 20 ms

    let packets: Vec<RtpPacket> = (0u16..50)
        .map(|seq| {
            let ts = seq as u32 * clock_rate_samples_per_packet;
            RtpPacket::new_with_payload(111, seq, ts, ssrc, Bytes::from_static(b"x"))
        })
        .collect();

    // Serialize and re-parse each packet
    for (i, pkt) in packets.iter().enumerate() {
        let bytes = pkt.serialize().expect("serialize");
        let parsed = RtpPacket::parse(&bytes).expect("parse");

        assert_eq!(parsed.header.sequence_number, i as u16);
        assert_eq!(
            parsed.header.timestamp,
            i as u32 * clock_rate_samples_per_packet
        );
        assert_eq!(parsed.header.ssrc, ssrc);
    }
}

/// Sequence number wraps around at u16::MAX.
#[test]
fn rtp_sequence_number_wraparound() {
    let pkt_before = RtpPacket::new_with_payload(0, u16::MAX, 0, 1, Bytes::from_static(b"."));
    let pkt_after = RtpPacket::new_with_payload(0, 0, 160, 1, Bytes::from_static(b"."));

    let parsed_before =
        RtpPacket::parse(&pkt_before.serialize().expect("ser")).expect("parse");
    let parsed_after =
        RtpPacket::parse(&pkt_after.serialize().expect("ser")).expect("parse");

    assert_eq!(parsed_before.header.sequence_number, u16::MAX);
    assert_eq!(parsed_after.header.sequence_number, 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 2: RTCP Packet Construction and Parsing
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn rtcp_sender_report_roundtrip() {
    let mut sr = RtcpSenderReport::new(0xABCD_0001);
    sr.rtp_timestamp = 48_000;
    sr.sender_packet_count = 100;
    sr.sender_octet_count = 16_000;

    let mut block = RtcpReportBlock::new(0xABCD_0002);
    block.fraction_lost = 10;
    block.cumulative_lost = 5;
    block.highest_seq = 500;
    block.jitter = 320;
    sr.add_report_block(block);

    let rtcp = RtcpPacket::SenderReport(sr.clone());
    let bytes = rtcp.serialize().expect("serialize SR");
    let parsed = RtcpPacket::parse(&bytes).expect("parse SR");

    if let RtcpPacket::SenderReport(parsed_sr) = parsed {
        assert_eq!(parsed_sr.ssrc, 0xABCD_0001);
        assert_eq!(parsed_sr.rtp_timestamp, 48_000);
        assert_eq!(parsed_sr.sender_packet_count, 100);
        assert_eq!(parsed_sr.sender_octet_count, 16_000);
        assert_eq!(parsed_sr.report_blocks.len(), 1);
        assert_eq!(parsed_sr.report_blocks[0].ssrc, 0xABCD_0002);
        assert_eq!(parsed_sr.report_blocks[0].fraction_lost, 10);
        assert_eq!(parsed_sr.report_blocks[0].jitter, 320);
    } else {
        panic!("Expected SenderReport variant");
    }
}

#[test]
fn rtcp_receiver_report_roundtrip() {
    let mut rr = RtcpReceiverReport::new(0xBEEF_0001);
    let mut block = RtcpReportBlock::new(0xBEEF_0002);
    block.highest_seq = 1000;
    block.jitter = 50;
    block.fraction_lost = 25;
    block.cumulative_lost = 12;
    rr.add_report_block(block);

    let rtcp = RtcpPacket::ReceiverReport(rr);
    let bytes = rtcp.serialize().expect("serialize RR");
    let parsed = RtcpPacket::parse(&bytes).expect("parse RR");

    if let RtcpPacket::ReceiverReport(parsed_rr) = parsed {
        assert_eq!(parsed_rr.ssrc, 0xBEEF_0001);
        assert_eq!(parsed_rr.report_blocks.len(), 1);
        assert_eq!(parsed_rr.report_blocks[0].highest_seq, 1000);
        assert_eq!(parsed_rr.report_blocks[0].fraction_lost, 25);
    } else {
        panic!("Expected ReceiverReport variant");
    }
}

#[test]
fn rtcp_goodbye_roundtrip() {
    let bye = RtcpGoodbye::new_with_reason(0xDEAD_0001, "leaving session".to_string());
    let rtcp = RtcpPacket::Goodbye(bye);
    let bytes = rtcp.serialize().expect("serialize BYE");
    let parsed = RtcpPacket::parse(&bytes).expect("parse BYE");

    if let RtcpPacket::Goodbye(parsed_bye) = parsed {
        assert_eq!(parsed_bye.sources.len(), 1);
        assert_eq!(parsed_bye.sources[0], 0xDEAD_0001);
        // Reason may or may not survive roundtrip depending on serialization;
        // at minimum the SSRC must be correct.
    } else {
        panic!("Expected Goodbye variant");
    }
}

#[test]
fn rtcp_compound_packet_sr_plus_bye() {
    let sr = RtcpSenderReport::new(0x1111_2222);
    let mut compound = RtcpCompoundPacket::new_with_sr(sr);

    let bye = RtcpGoodbye::new_for_source(0x1111_2222);
    compound.packets.push(RtcpPacket::Goodbye(bye));

    assert_eq!(compound.packets.len(), 2);
    assert!(compound.get_sr().is_some());
}

#[test]
fn ntp_timestamp_roundtrip_via_u64() {
    let ts = NtpTimestamp::now();
    let as_u64 = ts.to_u64();
    let restored = NtpTimestamp::from_u64(as_u64);

    assert_eq!(ts.seconds, restored.seconds);
    assert_eq!(ts.fraction, restored.fraction);
}

#[test]
fn ntp_timestamp_to_u32_compact() {
    let ts = NtpTimestamp {
        seconds: 0xAABB_CCDD,
        fraction: 0x1122_3344,
    };
    let compact = ts.to_u32();
    // Middle 32 bits: lower 16 of seconds (0xCCDD) << 16 | upper 16 of fraction (0x1122)
    let expected = (0xCCDDu32 << 16) | 0x1122u32;
    assert_eq!(compact, expected);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 3: STUN Binding Request/Response
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[allow(deprecated)]
fn stun_binding_request_encode_decode() {
    let req = StunMessage::binding_request();
    let encoded = req.encode();

    // Must start with correct message type
    assert_eq!(encoded[0] & 0xC0, 0x00, "First two bits must be zero");
    let msg_type = u16::from_be_bytes([encoded[0], encoded[1]]);
    assert_eq!(msg_type, BINDING_REQUEST);

    // Magic cookie
    let cookie = u32::from_be_bytes([encoded[4], encoded[5], encoded[6], encoded[7]]);
    assert_eq!(cookie, MAGIC_COOKIE);

    // Total size must be at least the 20-byte header
    assert!(encoded.len() >= HEADER_SIZE);

    // Decode roundtrip
    let decoded = StunMessage::decode(&encoded).expect("decode");
    assert_eq!(decoded.msg_type, BINDING_REQUEST);
    assert_eq!(decoded.transaction_id, req.transaction_id);
}

#[test]
#[allow(deprecated)]
fn stun_transaction_id_is_random() {
    let id1 = TransactionId::random();
    let id2 = TransactionId::random();
    // Extremely unlikely to collide
    assert_ne!(id1, id2, "Two random transaction IDs should differ");
}

#[test]
#[allow(deprecated)]
fn stun_binding_response_with_xor_mapped_address() {
    use rvoip_rtp_core::stun::message::{
        StunAttribute, encode_xor_address, ATTR_XOR_MAPPED_ADDRESS,
    };

    let txn_id = TransactionId::random();
    let mapped_addr: SocketAddr = "203.0.113.5:12345".parse().unwrap();

    // Manually build a Binding Response with XOR-MAPPED-ADDRESS
    // since StunMessage::encode() skips response-only attributes.
    let xor_value = encode_xor_address(&mapped_addr, &txn_id.0);

    let mut buf = Vec::with_capacity(HEADER_SIZE + 4 + xor_value.len());
    // Message type: Binding Response
    buf.extend_from_slice(&BINDING_RESPONSE.to_be_bytes());
    // Message length: attr header (4) + attr value
    let attr_total = 4 + xor_value.len();
    buf.extend_from_slice(&(attr_total as u16).to_be_bytes());
    // Magic cookie
    buf.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    // Transaction ID
    buf.extend_from_slice(&txn_id.0);
    // XOR-MAPPED-ADDRESS attribute
    buf.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
    buf.extend_from_slice(&(xor_value.len() as u16).to_be_bytes());
    buf.extend_from_slice(&xor_value);

    let decoded = StunMessage::decode(&buf).expect("decode response");
    assert_eq!(decoded.msg_type, BINDING_RESPONSE);
    assert_eq!(decoded.transaction_id, txn_id);

    let found_addr = decoded.mapped_address();
    assert!(found_addr.is_some(), "Should have a mapped address");
    assert_eq!(found_addr.unwrap(), mapped_addr);
}

#[test]
#[allow(deprecated)]
fn stun_decode_rejects_short_buffer() {
    let short = [0u8; 10]; // Less than 20-byte header
    let result = StunMessage::decode(&short);
    assert!(result.is_err());
}

#[test]
#[allow(deprecated)]
fn stun_decode_rejects_bad_magic_cookie() {
    let mut msg = StunMessage::binding_request().encode();
    // Corrupt the magic cookie
    msg[4] = 0xFF;
    msg[5] = 0xFF;
    msg[6] = 0xFF;
    msg[7] = 0xFF;
    let result = StunMessage::decode(&msg);
    assert!(result.is_err());
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 4: ICE Types and Candidate Handling
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ice_candidate_type_preference_ordering() {
    // RFC 8445: host > prflx > srflx > relay
    assert!(CandidateType::Host.type_preference() > CandidateType::PeerReflexive.type_preference());
    assert!(CandidateType::PeerReflexive.type_preference() > CandidateType::ServerReflexive.type_preference());
    assert!(CandidateType::ServerReflexive.type_preference() > CandidateType::Relay.type_preference());
}

#[test]
fn ice_candidate_priority_computation() {
    let host_prio = compute_priority(CandidateType::Host, 65535, ComponentId::Rtp);
    let srflx_prio = compute_priority(CandidateType::ServerReflexive, 65535, ComponentId::Rtp);
    let relay_prio = compute_priority(CandidateType::Relay, 65535, ComponentId::Rtp);

    assert!(host_prio > srflx_prio);
    assert!(srflx_prio > relay_prio);

    // RTP component (id=1) should have slightly higher priority than RTCP (id=2)
    let rtp_prio = compute_priority(CandidateType::Host, 65535, ComponentId::Rtp);
    let rtcp_prio = compute_priority(CandidateType::Host, 65535, ComponentId::Rtcp);
    assert!(rtp_prio > rtcp_prio);
}

#[test]
fn ice_candidate_foundation_deterministic() {
    let addr: SocketAddr = "192.168.1.10:5000".parse().unwrap();
    let f1 = generate_foundation(CandidateType::Host, &addr, None);
    let f2 = generate_foundation(CandidateType::Host, &addr, None);
    assert_eq!(f1, f2, "Same inputs must produce the same foundation");
}

#[test]
fn ice_candidate_foundation_differs_by_type() {
    let addr: SocketAddr = "192.168.1.10:5000".parse().unwrap();
    let f_host = generate_foundation(CandidateType::Host, &addr, None);
    let f_srflx = generate_foundation(CandidateType::ServerReflexive, &addr, None);
    assert_ne!(f_host, f_srflx, "Different types should produce different foundations");
}

#[test]
fn ice_candidate_sdp_attribute_roundtrip() {
    let candidate = IceCandidate {
        foundation: "1234".to_string(),
        component: ComponentId::Rtp,
        transport: "udp".to_string(),
        priority: 2130706431,
        address: "192.168.1.10:5000".parse().unwrap(),
        candidate_type: CandidateType::Host,
        related_address: None,
        ufrag: "test".to_string(),
    };

    let sdp_line = candidate.to_sdp_attribute();
    assert!(sdp_line.contains("host"), "SDP line should contain 'host'");
    assert!(sdp_line.contains("192.168.1.10"), "SDP line should contain IP");
    assert!(sdp_line.contains("5000"), "SDP line should contain port");

    // Parse it back
    let parsed = IceCandidate::from_sdp_attribute(&sdp_line).expect("parse SDP candidate");
    assert_eq!(parsed.foundation, "1234");
    assert_eq!(parsed.component, ComponentId::Rtp);
    assert_eq!(parsed.candidate_type, CandidateType::Host);
    assert_eq!(parsed.address.port(), 5000);
}

#[test]
fn ice_candidate_srflx_with_related_address() {
    let candidate = IceCandidate {
        foundation: "5678".to_string(),
        component: ComponentId::Rtp,
        transport: "udp".to_string(),
        priority: 1694498815,
        address: "203.0.113.5:12345".parse().unwrap(),
        candidate_type: CandidateType::ServerReflexive,
        related_address: Some("192.168.1.10:5000".parse().unwrap()),
        ufrag: "ufrag1".to_string(),
    };

    let sdp_line = candidate.to_sdp_attribute();
    assert!(sdp_line.contains("srflx"), "SDP line should contain 'srflx'");
    assert!(sdp_line.contains("raddr"), "SDP line should contain related address");
    assert!(sdp_line.contains("rport"), "SDP line should contain related port");
}

#[test]
fn ice_candidate_type_from_str() {
    assert_eq!("host".parse::<CandidateType>().unwrap(), CandidateType::Host);
    assert_eq!("srflx".parse::<CandidateType>().unwrap(), CandidateType::ServerReflexive);
    assert_eq!("prflx".parse::<CandidateType>().unwrap(), CandidateType::PeerReflexive);
    assert_eq!("relay".parse::<CandidateType>().unwrap(), CandidateType::Relay);
    assert!("invalid".parse::<CandidateType>().is_err());
}

#[test]
fn ice_role_display() {
    assert_eq!(format!("{}", IceRole::Controlling), "controlling");
    assert_eq!(format!("{}", IceRole::Controlled), "controlled");
}

#[test]
fn ice_connection_state_display() {
    assert_eq!(format!("{}", IceConnectionState::New), "new");
    assert_eq!(format!("{}", IceConnectionState::Connected), "connected");
    assert_eq!(format!("{}", IceConnectionState::Failed), "failed");
}

#[test]
fn ice_credentials_struct() {
    let creds = IceCredentials {
        ufrag: "abc123".to_string(),
        pwd: "secret_password_over_22_chars".to_string(),
    };
    assert_eq!(creds.ufrag, "abc123");
    assert_eq!(creds.pwd, "secret_password_over_22_chars");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 5: SRTP Encrypt/Decrypt via Production Adapter
// ═══════════════════════════════════════════════════════════════════════════

/// Build a minimal valid RTP packet for SRTP testing.
fn build_rtp_bytes(seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(12 + payload.len());
    pkt.push(0x80); // V=2
    pkt.push(0x00); // PT=0
    pkt.extend_from_slice(&seq.to_be_bytes());
    let ts = 160u32.wrapping_mul(seq as u32);
    pkt.extend_from_slice(&ts.to_be_bytes());
    pkt.extend_from_slice(&1u32.to_be_bytes()); // SSRC=1
    pkt.extend_from_slice(payload);
    pkt
}

/// Build a minimal RTCP Sender Report for SRTCP testing.
fn build_rtcp_sr_bytes() -> Vec<u8> {
    vec![
        0x80, 0xC8, // V=2, PT=200 (SR)
        0x00, 0x06, // Length=6 (28 bytes)
        0x00, 0x00, 0x00, 0x01, // SSRC=1
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // NTP ts
        0x00, 0x00, 0x00, 0xA0, // RTP ts
        0x00, 0x00, 0x00, 0x0A, // pkt count
        0x00, 0x00, 0x01, 0x00, // octet count
    ]
}

#[test]
fn srtp_protect_unprotect_roundtrip() {
    // AES-128-CM-HMAC-SHA1-80: 16-byte key, 14-byte salt
    let key_local: Vec<u8> = (0..16).collect();
    let salt_local: Vec<u8> = (16..30).collect();
    let key_remote: Vec<u8> = (32..48).collect();
    let salt_remote: Vec<u8> = (48..62).collect();

    let mut sender = SrtpContextAdapter::new(
        &key_local, &salt_local,
        &key_remote, &salt_remote,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )
    .expect("create sender");

    let mut receiver = SrtpContextAdapter::new(
        &key_remote, &salt_remote,
        &key_local, &salt_local,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )
    .expect("create receiver");

    let rtp = build_rtp_bytes(1, b"secure audio payload");
    let protected = sender.protect_rtp(&rtp).expect("protect");

    assert_ne!(protected.as_ref(), &rtp[..], "Protected must differ from plaintext");

    let unprotected = receiver.unprotect_rtp(&protected).expect("unprotect");
    assert_eq!(&unprotected[..], &rtp[..], "Roundtrip must restore original");
}

#[test]
fn srtp_multiple_packets_sequential() {
    let key_a: Vec<u8> = (0..16).collect();
    let salt_a: Vec<u8> = (16..30).collect();
    let key_b: Vec<u8> = (32..48).collect();
    let salt_b: Vec<u8> = (48..62).collect();

    let mut sender = SrtpContextAdapter::new(
        &key_a, &salt_a, &key_b, &salt_b,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )
    .expect("sender");

    let mut receiver = SrtpContextAdapter::new(
        &key_b, &salt_b, &key_a, &salt_a,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )
    .expect("receiver");

    for seq in 1u16..=20 {
        let rtp = build_rtp_bytes(seq, format!("frame-{seq}").as_bytes());
        let protected = sender.protect_rtp(&rtp).expect("protect");
        let unprotected = receiver.unprotect_rtp(&protected).expect("unprotect");
        assert_eq!(&unprotected[..], &rtp[..]);
    }
}

#[test]
fn srtcp_protect_unprotect_roundtrip() {
    let key_a: Vec<u8> = (0..16).collect();
    let salt_a: Vec<u8> = (16..30).collect();
    let key_b: Vec<u8> = (32..48).collect();
    let salt_b: Vec<u8> = (48..62).collect();

    let mut sender = SrtpContextAdapter::new(
        &key_a, &salt_a, &key_b, &salt_b,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )
    .expect("sender");

    let mut receiver = SrtpContextAdapter::new(
        &key_b, &salt_b, &key_a, &salt_a,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )
    .expect("receiver");

    let rtcp = build_rtcp_sr_bytes();
    let protected = sender.protect_rtcp(&rtcp).expect("protect RTCP");
    assert_ne!(protected.as_ref(), &rtcp[..]);

    let unprotected = receiver.unprotect_rtcp(&protected).expect("unprotect RTCP");
    assert_eq!(&unprotected[..], &rtcp[..]);
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 6: DTLS-SRTP Handshake and Key Extraction
// ═══════════════════════════════════════════════════════════════════════════

/// Perform a DTLS handshake between two local endpoints over loopback UDP.
async fn dtls_handshake_pair(
) -> Result<(rvoip_rtp_core::dtls::adapter::SrtpKeyMaterial, rvoip_rtp_core::dtls::adapter::SrtpKeyMaterial), Error>
{
    use std::sync::Arc;
    use tokio::net::UdpSocket;

    let sock_a = UdpSocket::bind("127.0.0.1:0").await.map_err(|e| Error::IoError(e.to_string()))?;
    let sock_b = UdpSocket::bind("127.0.0.1:0").await.map_err(|e| Error::IoError(e.to_string()))?;

    let addr_a = sock_a.local_addr().map_err(|e| Error::IoError(e.to_string()))?;
    let addr_b = sock_b.local_addr().map_err(|e| Error::IoError(e.to_string()))?;

    sock_a.connect(addr_b).await.map_err(|e| Error::IoError(e.to_string()))?;
    sock_b.connect(addr_a).await.map_err(|e| Error::IoError(e.to_string()))?;

    let conn_a: Arc<dyn webrtc_util_dtls::Conn + Send + Sync> = Arc::new(sock_a);
    let conn_b: Arc<dyn webrtc_util_dtls::Conn + Send + Sync> = Arc::new(sock_b);

    let config = DtlsAdapterConfig {
        insecure_skip_verify: true,
        ..DtlsAdapterConfig::default()
    };

    let mut client = DtlsConnectionAdapter::new(DtlsRole::Client).await?;
    let mut server = DtlsConnectionAdapter::new(DtlsRole::Server).await?;

    let config_c = config.clone();
    let client_handle = tokio::spawn(async move {
        client.handshake(conn_a, &config_c).await?;
        client.get_srtp_keys().await
    });

    let server_handle = tokio::spawn(async move {
        server.handshake(conn_b, &config).await?;
        server.get_srtp_keys().await
    });

    let client_keys = client_handle
        .await
        .map_err(|e| Error::IoError(format!("client task panicked: {e}")))??;
    let server_keys = server_handle
        .await
        .map_err(|e| Error::IoError(format!("server task panicked: {e}")))??;

    Ok((client_keys, server_keys))
}

#[tokio::test]
async fn dtls_handshake_extracts_matching_srtp_keys() {
    let result = tokio::time::timeout(Duration::from_secs(15), dtls_handshake_pair()).await;
    let (client_keys, server_keys) = result
        .expect("DTLS handshake timed out")
        .expect("DTLS handshake failed");

    // Client's local (tx) key == Server's remote (rx) key
    assert_eq!(client_keys.local_key, server_keys.remote_key);
    assert_eq!(client_keys.local_salt, server_keys.remote_salt);

    // Server's local (tx) key == Client's remote (rx) key
    assert_eq!(server_keys.local_key, client_keys.remote_key);
    assert_eq!(server_keys.local_salt, client_keys.remote_salt);

    // Both agree on SRTP profile
    assert_eq!(client_keys.profile, server_keys.profile);

    // Keys are non-trivial
    assert!(client_keys.local_key.iter().any(|&b| b != 0));
    assert!(server_keys.local_key.iter().any(|&b| b != 0));
}

#[tokio::test]
async fn dtls_to_srtp_full_pipeline_over_udp() {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let (client_keys, server_keys) = dtls_handshake_pair().await?;

        let mut client_srtp = SrtpContextAdapter::from_key_material(&client_keys)?;
        let mut server_srtp = SrtpContextAdapter::from_key_material(&server_keys)?;

        // Set up media UDP channel
        let tx_sock = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .map_err(|e| Error::IoError(e.to_string()))?;
        let rx_sock = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .map_err(|e| Error::IoError(e.to_string()))?;
        let rx_addr = rx_sock.local_addr().map_err(|e| Error::IoError(e.to_string()))?;

        // Client sends protected RTP
        let payload = b"dtls-srtp encrypted audio";
        let rtp = build_rtp_bytes(100, payload);
        let srtp = client_srtp.protect_rtp(&rtp)?;

        tx_sock.send_to(&srtp, rx_addr).await.map_err(|e| Error::IoError(e.to_string()))?;

        // Server receives and decrypts
        let mut buf = vec![0u8; 2048];
        let (n, _) = rx_sock.recv_from(&mut buf).await.map_err(|e| Error::IoError(e.to_string()))?;
        let decrypted = server_srtp.unprotect_rtp(&buf[..n])?;

        assert_eq!(&decrypted[..], &rtp[..], "Full pipeline: decrypted must match original");
        assert_eq!(&decrypted[12..], payload, "Payload must match");

        Ok::<(), Error>(())
    })
    .await;

    result.expect("timed out").expect("pipeline failed");
}

// ═══════════════════════════════════════════════════════════════════════════
// SECTION 7: Error Handling
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn rtp_parse_rejects_too_short_buffer() {
    let short = [0x80, 0x00, 0x00]; // Only 3 bytes, need at least 12
    let result = RtpPacket::parse(&short);
    assert!(result.is_err(), "Parsing a 3-byte buffer must fail");
}

#[test]
fn rtp_parse_rejects_wrong_version() {
    // Version 3 (first byte 0xC0) is invalid
    let mut bad = vec![0xC0, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xA0,
                       0x00, 0x00, 0x00, 0x01, 0x41, 0x42];
    let result = RtpPacket::parse(&bad);
    assert!(result.is_err(), "Version != 2 should be rejected");
}

#[test]
fn rtp_parse_rejects_empty_buffer() {
    let result = RtpPacket::parse(&[]);
    assert!(result.is_err());
}

#[test]
fn rtcp_parse_rejects_too_short_buffer() {
    let short = [0x80, 0xC8]; // Only 2 bytes
    let result = RtcpPacket::parse(&short);
    assert!(result.is_err());
}

#[test]
fn rtcp_parse_rejects_invalid_version() {
    // Version 0 (first byte 0x00)
    let bad = [0x00, 0xC8, 0x00, 0x06,
               0x00, 0x00, 0x00, 0x01,
               0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
               0x00, 0x00, 0x00, 0xA0,
               0x00, 0x00, 0x00, 0x0A,
               0x00, 0x00, 0x01, 0x00];
    let result = RtcpPacket::parse(&bad);
    assert!(result.is_err(), "Invalid RTCP version should fail");
}

#[test]
fn rtcp_parse_rejects_unknown_packet_type() {
    // Packet type 199 is not a valid RTCP type
    let bad = [0x80, 199, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01];
    let result = RtcpPacket::parse(&bad);
    assert!(result.is_err(), "Unknown RTCP packet type should fail");
}

#[test]
fn srtp_unprotect_with_wrong_key_fails() {
    let key_a: Vec<u8> = (0..16).collect();
    let salt_a: Vec<u8> = (16..30).collect();
    let key_b: Vec<u8> = (32..48).collect();
    let salt_b: Vec<u8> = (48..62).collect();
    let key_c: Vec<u8> = (64..80).collect(); // Wrong key
    let salt_c: Vec<u8> = (80..94).collect();

    let mut sender = SrtpContextAdapter::new(
        &key_a, &salt_a, &key_b, &salt_b,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )
    .expect("sender");

    // Receiver has the wrong keys
    let mut wrong_receiver = SrtpContextAdapter::new(
        &key_c, &salt_c, &key_c, &salt_c,
        ProtectionProfile::Aes128CmHmacSha1_80,
    )
    .expect("wrong receiver");

    let rtp = build_rtp_bytes(1, b"secret data");
    let protected = sender.protect_rtp(&rtp).expect("protect");
    let result = wrong_receiver.unprotect_rtp(&protected);

    assert!(result.is_err(), "Unprotect with wrong key must fail");
}

#[test]
fn error_type_display() {
    let err = Error::InvalidPacket("bad header".to_string());
    assert!(err.to_string().contains("bad header"));

    let err = Error::BufferTooSmall {
        required: 12,
        available: 3,
    };
    assert!(err.to_string().contains("12"));
    assert!(err.to_string().contains("3"));

    let err = Error::DtlsHandshakeError("timeout".to_string());
    assert!(err.to_string().contains("timeout"));

    let err = Error::StunError("bad cookie".to_string());
    assert!(err.to_string().contains("bad cookie"));
}
