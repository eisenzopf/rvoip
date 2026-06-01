//! Canonical SIP message fixtures for benchmarks.
//!
//! Each constant is a CRLF-terminated SIP message suitable for feeding
//! directly into `parse_message`. Sizes are approximate, listed so bench
//! sweeps can pick representative samples without re-measuring.

#![allow(dead_code)]

pub struct Fixture {
    pub name: &'static str,
    pub bytes: &'static [u8],
}

/// ~200B — bare REGISTER, no auth, single Via.
pub const REGISTER_MINIMAL: &str = "\
REGISTER sip:registrar.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc.example.com:5060;branch=z9hG4bKreg01\r\n\
Max-Forwards: 70\r\n\
From: Alice <sip:alice@example.com>;tag=reg01\r\n\
To: Alice <sip:alice@example.com>\r\n\
Call-ID: reg-abc@pc.example.com\r\n\
CSeq: 1 REGISTER\r\n\
Contact: <sip:alice@pc.example.com:5060>\r\n\
Expires: 3600\r\n\
Content-Length: 0\r\n\r\n";

/// ~600B — REGISTER with Digest auth (post-401 challenge response).
pub const REGISTER_AUTHD: &str = "\
REGISTER sip:registrar.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc.example.com:5060;branch=z9hG4bKreg02\r\n\
Max-Forwards: 70\r\n\
From: Alice <sip:alice@example.com>;tag=reg02\r\n\
To: Alice <sip:alice@example.com>\r\n\
Call-ID: reg-def@pc.example.com\r\n\
CSeq: 2 REGISTER\r\n\
Contact: <sip:alice@pc.example.com:5060>\r\n\
Authorization: Digest username=\"alice\", realm=\"example.com\", \
nonce=\"f84f1cec41e6cbe5aea9c8e88d359\", uri=\"sip:registrar.example.com\", \
response=\"7587245234b3434cc3412213e5f113a5\", algorithm=MD5, \
cnonce=\"0a4f113b\", qop=auth, nc=00000001\r\n\
Expires: 3600\r\n\
User-Agent: rvoip-bench/1.0\r\n\
Content-Length: 0\r\n\r\n";

/// ~900B — INVITE with no body (signalling-only).
pub const INVITE_MINIMAL: &str = "\
INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bK776asdhds\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Contact: <sip:alice@pc33.atlanta.example.com>\r\n\
Allow: INVITE, ACK, CANCEL, BYE, OPTIONS, UPDATE, REFER, NOTIFY\r\n\
Supported: replaces, timer, 100rel\r\n\
User-Agent: rvoip-bench/1.0\r\n\
Content-Length: 0\r\n\r\n";

/// ~2KB — INVITE with SDP audio offer (PCMU/PCMA, no ICE).
pub const INVITE_SDP_AUDIO: &str = "\
INVITE sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bK776asdsd2\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301775\r\n\
Call-ID: invite-sdp-audio@pc33.atlanta.example.com\r\n\
CSeq: 1 INVITE\r\n\
Contact: <sip:alice@pc33.atlanta.example.com>\r\n\
Allow: INVITE, ACK, CANCEL, BYE, OPTIONS, UPDATE, REFER, NOTIFY, PRACK\r\n\
Supported: replaces, timer, 100rel\r\n\
Session-Expires: 1800;refresher=uac\r\n\
Min-SE: 90\r\n\
User-Agent: rvoip-bench/1.0\r\n\
Content-Type: application/sdp\r\n\
Content-Length: 258\r\n\r\n\
v=0\r\n\
o=alice 2890844526 2890844526 IN IP4 192.0.2.101\r\n\
s=rvoip-bench session\r\n\
c=IN IP4 192.0.2.101\r\n\
t=0 0\r\n\
m=audio 49170 RTP/AVP 0 8 101\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=rtpmap:8 PCMA/8000\r\n\
a=rtpmap:101 telephone-event/8000\r\n\
a=fmtp:101 0-15\r\n\
a=sendrecv\r\n\
a=ptime:20\r\n";

/// 100 Trying provisional response.
pub const RESPONSE_100_TRYING: &str = "\
SIP/2.0 100 Trying\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bK776asdhds\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Content-Length: 0\r\n\r\n";

/// 180 Ringing provisional response with to-tag.
pub const RESPONSE_180_RINGING: &str = "\
SIP/2.0 180 Ringing\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bK776asdhds\r\n\
To: Bob <sip:bob@biloxi.example.com>;tag=8321234356\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Contact: <sip:bob@biloxi.example.com>\r\n\
Content-Length: 0\r\n\r\n";

/// ~1KB — 200 OK to INVITE with SDP answer.
pub const RESPONSE_200_OK_INVITE: &str = "\
SIP/2.0 200 OK\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bK776asdhds\r\n\
To: Bob <sip:bob@biloxi.example.com>;tag=8321234356\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 INVITE\r\n\
Contact: <sip:bob@biloxi.example.com>\r\n\
Allow: INVITE, ACK, CANCEL, BYE, OPTIONS, UPDATE\r\n\
Content-Type: application/sdp\r\n\
Content-Length: 175\r\n\r\n\
v=0\r\n\
o=bob 2890844527 2890844527 IN IP4 192.0.2.202\r\n\
s=rvoip-bench session\r\n\
c=IN IP4 192.0.2.202\r\n\
t=0 0\r\n\
m=audio 3456 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=sendrecv\r\n\
a=ptime:20\r\n";

/// ACK terminating the 3-way INVITE handshake.
pub const ACK: &str = "\
ACK sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bK776asdhds\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>;tag=8321234356\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314159 ACK\r\n\
Content-Length: 0\r\n\r\n";

/// BYE to tear down the dialog.
pub const BYE: &str = "\
BYE sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bKbye01\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>;tag=8321234356\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=1928301774\r\n\
Call-ID: a84b4c76e66710@pc33.atlanta.example.com\r\n\
CSeq: 314160 BYE\r\n\
Content-Length: 0\r\n\r\n";

/// OPTIONS keep-alive request.
pub const OPTIONS: &str = "\
OPTIONS sip:bob@biloxi.example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP pc33.atlanta.example.com:5060;branch=z9hG4bKopt01\r\n\
Max-Forwards: 70\r\n\
To: Bob <sip:bob@biloxi.example.com>\r\n\
From: Alice <sip:alice@atlanta.example.com>;tag=opt01\r\n\
Call-ID: opts-abc@pc33.atlanta.example.com\r\n\
CSeq: 1 OPTIONS\r\n\
Contact: <sip:alice@pc33.atlanta.example.com>\r\n\
Accept: application/sdp\r\n\
Content-Length: 0\r\n\r\n";

/// Full corpus used across micro-benches. Order is stable so saved
/// Criterion baselines stay comparable across runs.
pub const CORPUS: &[Fixture] = &[
    Fixture {
        name: "register_minimal",
        bytes: REGISTER_MINIMAL.as_bytes(),
    },
    Fixture {
        name: "register_authd",
        bytes: REGISTER_AUTHD.as_bytes(),
    },
    Fixture {
        name: "invite_minimal",
        bytes: INVITE_MINIMAL.as_bytes(),
    },
    Fixture {
        name: "invite_sdp_audio",
        bytes: INVITE_SDP_AUDIO.as_bytes(),
    },
    Fixture {
        name: "response_100_trying",
        bytes: RESPONSE_100_TRYING.as_bytes(),
    },
    Fixture {
        name: "response_180_ringing",
        bytes: RESPONSE_180_RINGING.as_bytes(),
    },
    Fixture {
        name: "response_200_ok_invite",
        bytes: RESPONSE_200_OK_INVITE.as_bytes(),
    },
    Fixture {
        name: "ack",
        bytes: ACK.as_bytes(),
    },
    Fixture {
        name: "bye",
        bytes: BYE.as_bytes(),
    },
    Fixture {
        name: "options",
        bytes: OPTIONS.as_bytes(),
    },
];

pub fn corpus() -> &'static [Fixture] {
    CORPUS
}
