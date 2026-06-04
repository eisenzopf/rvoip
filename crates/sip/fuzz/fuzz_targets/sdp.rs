#![no_main]

use bytes::Bytes;
use libfuzzer_sys::fuzz_target;

const WEBRTC_SDP: &str = "v=0\r\n\
o=- 20518 0 IN IP4 0.0.0.0\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE audio data\r\n\
a=msid-semantic:WMS *\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:audio\r\n\
a=ice-ufrag:F7gI\r\n\
a=ice-pwd:x9cml/YzichV2+XlhiMu8g\r\n\
a=ice-options:trickle\r\n\
a=fingerprint:sha-256 D1:2C:74:A7:E3:B5:8D:B1:69:6C:72:E9:6F:7F:79:5B\r\n\
a=setup:actpass\r\n\
a=rtcp-mux\r\n\
a=rtcp-rsize\r\n\
a=rtpmap:111 opus/48000/2\r\n\
a=rtcp-fb:111 transport-cc\r\n\
a=extmap:1/sendrecv https://example.com/rtp-ext\r\n\
a=rid:f send pt=111;max-width=1280\r\n\
a=simulcast:send f\r\n\
a=candidate:1 1 udp 2122260223 127.0.0.1 50000 typ host generation 0\r\n\
a=end-of-candidates\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:data\r\n\
a=sctp-port:5000\r\n\
a=max-message-size:262144\r\n\
a=dcmap:0 label=\"chat\"\r\n\
a=dcsa:0 fmtp:webrtc-datachannel ordered=true\r\n";

fuzz_target!(|data: &[u8]| {
    if data.len() > 65_536 {
        return;
    }

    let bytes = Bytes::copy_from_slice(data);
    parse_lenient_and_strict(bytes);

    // Valid RFC-shaped SDP with fuzzer-controlled text in a typed standard
    // attribute. This keeps the parser on valid line ordering while still
    // mutating attribute payloads.
    let fuzz_text = hex_prefix(data, 512);
    let valid_core = format!(
        "v=0\r\n\
o=- 1 1 IN IP4 127.0.0.1\r\n\
s=fuzz\r\n\
i={fuzz_text}\r\n\
e=fuzz@example.invalid\r\n\
p=+1 555 0100\r\n\
c=IN IP4 127.0.0.1\r\n\
b=TIAS:64000\r\n\
t=0 0\r\n\
z=0 0\r\n\
k=clear:{fuzz_text}\r\n\
a=tool:{fuzz_text}\r\n\
m=audio 9/2 RTP/AVP 0 101\r\n\
i=fuzz media\r\n\
c=IN IP4 127.0.0.1\r\n\
b=AS:64\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=rtpmap:101 telephone-event/8000\r\n"
    );
    parse_lenient_and_strict(Bytes::from(valid_core));

    // WebRTC corpus path with a fuzzer-controlled preserved extension line.
    let webrtc = format!("{WEBRTC_SDP}a=x-fuzz:{fuzz_text}\r\n");
    parse_lenient_and_strict(Bytes::from(webrtc));

    // Line permutation path: prefix arbitrary UTF-8 lines with the mandatory
    // session header so invalid/valid mixed line ordering hits both modes.
    if let Ok(text) = std::str::from_utf8(data) {
        let mut mixed = String::from("v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=fuzz\r\nt=0 0\r\n");
        for line in text.lines().take(64) {
            mixed.push_str(line);
            mixed.push_str("\r\n");
        }
        parse_lenient_and_strict(Bytes::from(mixed));
    }
});

fn parse_lenient_and_strict(bytes: Bytes) {
    let _ = rvoip_sip_core::parse_sdp(&bytes);
    let _ = rvoip_sip_core::sdp::parse_sdp_strict(&bytes);
}

fn hex_prefix(data: &[u8], max: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len().min(max) * 2);
    for byte in data.iter().take(max) {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    if out.is_empty() {
        out.push('0');
    }
    out
}
