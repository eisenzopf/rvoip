# Introduction to SDP

Session Description Protocol (SDP) is a format for describing multimedia communication sessions for the purposes of session announcement, session invitation, and other forms of multimedia session establishment. SDP is defined in [RFC 8866](https://datatracker.ietf.org/doc/html/rfc8866).

## Purpose of SDP

In SIP, SDP is used to describe the parameters of the media session being established. It enables the communicating parties to:

1. Specify which media types will be used (audio, video, etc.)
2. Define the codecs and formats for each media type
3. Indicate network transport addresses for each media stream
4. Negotiate session parameters like bandwidth, encryption, and other attributes

SDP doesn't transport any media itself; it merely describes the media session for the endpoints to establish direct media communication.

## SDP Message Structure

SDP messages are plain text and consist of a series of lines, with each line having a type (single character), equals sign, and value:

```
<type>=<value>
```

An SDP message has three main sections:

1. **Session-level information** (applies to the entire session)
2. **Time description** (when the session is active)
3. **Media descriptions** (one or more, each describing a media stream)

### Session-level Fields

| Field | Description | Required |
|-------|-------------|----------|
| v= | Protocol version (always 0) | Yes |
| o= | Origin (username, session ID, version, network type, address type, address) | Yes |
| s= | Session name | Yes |
| i= | Session information | No |
| u= | URI of description | No |
| e= | Email address | No |
| p= | Phone number | No |
| c= | Connection information (network type, address type, address) | No* |
| b= | Bandwidth information | No |
| z= | Time zone adjustments | No |
| k= | Encryption key | No |
| a= | Session attributes | No |

*Connection information is required either at session-level or in each media description

### Time Description

| Field | Description | Required |
|-------|-------------|----------|
| t= | Time the session is active (start, stop) | Yes |
| r= | Repeat times | No |

### Media Description

| Field | Description | Required |
|-------|-------------|----------|
| m= | Media name, port, protocol, and format descriptions | Yes |
| i= | Media title | No |
| c= | Connection information | No* |
| b= | Bandwidth information | No |
| k= | Encryption key | No |
| a= | Media attributes | No |

*Connection information is required if not specified at session-level

## Common Examples in SIP Signaling

### Audio-only Call Offer

```
v=0
o=alice 2890844526 2890844526 IN IP4 alice.example.com
s=Audio Call
c=IN IP4 alice.example.com
t=0 0
m=audio 49170 RTP/AVP 0 8 96
a=rtpmap:0 PCMU/8000
a=rtpmap:8 PCMA/8000
a=rtpmap:96 telephone-event/8000
a=sendrecv
```

This SDP describes:
- A session originated by "alice" from alice.example.com
- A single audio stream on port 49170
- Three supported codecs: PCMU (G.711 μ-law), PCMA (G.711 A-law), and telephone-event (for DTMF)
- Bidirectional communication (sendrecv)

### Audio and Video Call Offer

```
v=0
o=bob 2890844527 2890844527 IN IP4 bob.example.com
s=Audio/Video Call
c=IN IP4 bob.example.com
t=0 0
m=audio 49170 RTP/AVP 0 8
a=rtpmap:0 PCMU/8000
a=rtpmap:8 PCMA/8000
a=sendrecv
m=video 51372 RTP/AVP 97
a=rtpmap:97 H264/90000
a=sendrecv
```

This SDP describes:
- A session with both audio and video streams
- Audio on port 49170 with PCMU and PCMA codecs
- Video on port 51372 using H.264 codec
- Both streams are bidirectional

## SDP in SIP Messages

SDP is typically carried in SIP messages within the body of:

- INVITE requests (SDP offer)
- 200 OK responses to INVITEs (SDP answer)
- ACK requests (in some scenarios)
- UPDATE or re-INVITE requests (for session modifications)

The Content-Type header in the SIP message indicates the presence of SDP: `Content-Type: application/sdp`

## SDP Offer/Answer Model

SIP uses the SDP Offer/Answer model defined in [RFC 3264](https://datatracker.ietf.org/doc/html/rfc3264). The basic process is:

1. **Offer**: The initiator sends an SDP describing the media streams they wish to establish
2. **Answer**: The recipient responds with an SDP describing the media streams they are willing to accept

This negotiation process allows both parties to agree on compatible media parameters.

## SDP Attributes

Attributes (a= lines) are extremely flexible and can appear at either session-level or media-level. Some common attributes in SIP/SDP include:

- **a=sendrecv, a=sendonly, a=recvonly, a=inactive**: Media direction
- **a=rtpmap**: RTP payload type mapping to encoding name, clock rate, and parameters
- **a=fmtp**: Format parameters for a codec
- **a=ptime**: Preferred packet duration
- **a=maxptime**: Maximum packet duration
- **a=rtcp**: RTCP port information
- **a=setup**: Role in DTLS setup (used with WebRTC)
- **a=fingerprint**: DTLS fingerprint (used with WebRTC)
- **a=ice-ufrag, a=ice-pwd**: ICE credentials (used with WebRTC)

## Building SDP with rvoip-sip-core

Our library provides a convenient builder pattern for creating SDP messages. Here's a simple example:

```rust
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;

let sdp = SdpBuilder::new("Audio Call")
    .origin("alice", "2890844526", "2890844526", "IN", "IP4", "alice.example.com")
    .connection("IN", "IP4", "alice.example.com")
    .time("0", "0")
    .media_audio(49170, "RTP/AVP")
        .formats(&["0", "8", "96"])
        .rtpmap("0", "PCMU/8000")
        .rtpmap("8", "PCMA/8000")
        .rtpmap("96", "telephone-event/8000")
        .direction(MediaDirection::SendRecv)
        .done()
    .build()?;
```

In the next tutorial, we'll dive deeper into creating more complex SDP messages using our builder API and handling SDP parsing.

## Conclusion

SDP is a critical component in SIP-based communication, responsible for describing the media sessions being established. Understanding SDP is essential for implementing any SIP-based multimedia application.

The text-based format makes SDP easy to debug, but it can also be complex due to the many optional fields and attributes available. Our library provides a builder pattern to simplify SDP creation and ensure validity of the resulting messages. 