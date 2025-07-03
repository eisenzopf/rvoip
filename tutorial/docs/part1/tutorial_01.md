# Introduction to SIP

Session Initiation Protocol (SIP) is a signaling protocol used for initiating, maintaining, and terminating real-time sessions that include voice, video, and messaging applications. In this tutorial, we'll explore the fundamentals of SIP and how it works.

## What is SIP?

SIP is an application-layer control protocol that can establish, modify, and terminate multimedia sessions such as Internet telephony calls. SIP is text-based, similar to HTTP, making it relatively easy to debug and work with.

Key characteristics of SIP:

- **Text-based protocol**: SIP messages are human-readable text
- **Client-server architecture**: Involves User Agent Clients (UACs) and User Agent Servers (UASs)
- **Request-response model**: Similar to HTTP with requests and responses
- **Transport-independent**: Can run over TCP, UDP, TLS, or other transport protocols
- **Extensible**: Can be extended with new headers and methods

## SIP Architecture

SIP operates in a distributed architecture that includes several types of network elements:

- **User Agents (UAs)**: End devices that initiate and receive SIP sessions
  - **User Agent Client (UAC)**: Initiates SIP requests
  - **User Agent Server (UAS)**: Responds to SIP requests

- **SIP Servers**:
  - **Proxy Server**: Forwards requests to other servers
  - **Registrar**: Accepts registration requests from users
  - **Redirect Server**: Returns the location of requested users
  - **Back-to-Back User Agent (B2BUA)**: Acts as both UAC and UAS

## SIP Message Structure

SIP messages consist of a start-line, headers, and an optional body. There are two types of SIP messages:

1. **Requests**: Messages sent from a client to a server
2. **Responses**: Messages sent from a server to a client

### SIP Request Structure

```
METHOD Request-URI SIP/2.0
Header1: Value1
Header2: Value2
...
Content-Length: <length>

<message body>
```

### SIP Response Structure

```
SIP/2.0 Status-Code Reason-Phrase
Header1: Value1
Header2: Value2
...
Content-Length: <length>

<message body>
```

## SIP URIs

SIP uses Uniform Resource Identifiers (URIs) to identify users. A SIP URI looks like:

```
sip:username@domain.com:5060;uri-parameters?headers
```

Components:
- **Scheme**: "sip:" or "sips:" (secure SIP)
- **User**: The username part
- **Host**: Domain name or IP address
- **Port**: Optional port number (default is 5060 for SIP, 5061 for SIPS)
- **URI Parameters**: Optional parameters separated by semicolons
- **Headers**: Optional headers separated by ampersands

## SIP Methods

SIP defines several methods (or request types):

| Method | Description |
|--------|-------------|
| INVITE | Initiates a session |
| ACK | Acknowledges receipt of a final response to INVITE |
| BYE | Terminates a session |
| CANCEL | Cancels a pending request |
| REGISTER | Registers a user's location |
| OPTIONS | Queries capabilities of servers |
| INFO | Sends mid-session information |
| UPDATE | Updates session parameters |
| REFER | Asks recipient to issue a request |
| SUBSCRIBE | Requests notification of an event |
| NOTIFY | Provides information about an event |
| MESSAGE | Transports instant messages |

## SIP Response Codes

SIP responses are categorized by their status codes:

| Range | Category | Description |
|-------|----------|-------------|
| 1xx | Provisional | Request received, continuing to process |
| 2xx | Success | Action successfully received, understood, and accepted |
| 3xx | Redirection | Further action needs to be taken |
| 4xx | Client Error | Request contains bad syntax or cannot be fulfilled |
| 5xx | Server Error | Server failed to fulfill a valid request |
| 6xx | Global Failure | Request cannot be fulfilled at any server |

## SIP Headers

SIP messages include headers that provide additional information about the message. Some common headers include:

- **Via**: Shows the path taken by the request
- **From**: Indicates the initiator of the request
- **To**: Indicates the recipient of the request
- **Call-ID**: Unique identifier for the call
- **CSeq**: Command sequence number
- **Contact**: Provides a URI for direct communication
- **Content-Type**: Indicates the type of the message body
- **Content-Length**: Indicates the size of the message body

## Basic SIP Call Flow

A basic SIP call flow involves the following steps:

1. **INVITE**: Caller sends an INVITE request to initiate a session
2. **100 Trying**: Server acknowledges receipt of INVITE
3. **180 Ringing**: Callee's phone is ringing
4. **200 OK**: Callee accepts the call
5. **ACK**: Caller acknowledges receipt of 200 OK
6. Media session established (RTP/RTCP)
7. **BYE**: Either party terminates the session
8. **200 OK**: Acknowledgment of session termination

Here's a simplified diagram:

```
    Caller                    Callee
      |                         |
      |-------INVITE----------->|
      |<------100 Trying--------|
      |<------180 Ringing-------|
      |<------200 OK------------|
      |--------ACK------------->|
      |<====Media Session======>|
      |--------BYE------------->|
      |<------200 OK------------|
      |                         |
```

## Let's Examine a SIP Message

Here's an example of a SIP INVITE request:

```
INVITE sip:bob@example.com SIP/2.0
Via: SIP/2.0/UDP alice-pc.example.com:5060;branch=z9hG4bK776asdhds
Max-Forwards: 70
To: Bob <sip:bob@example.com>
From: Alice <sip:alice@example.com>;tag=1928301774
Call-ID: a84b4c76e66710@alice-pc.example.com
CSeq: 314159 INVITE
Contact: <sip:alice@alice-pc.example.com>
Content-Type: application/sdp
Content-Length: 142

v=0
o=alice 2890844526 2890844526 IN IP4 alice-pc.example.com
s=Session SDP
c=IN IP4 alice-pc.example.com
t=0 0
m=audio 49170 RTP/AVP 0
a=rtpmap:0 PCMU/8000
```

In this example:
- The request method is INVITE
- The Request-URI is sip:bob@example.com
- Various headers provide additional information
- The message body contains SDP for media negotiation

## SIP vs Other Protocols

SIP is often compared to other signaling protocols:

- **H.323**: An older, binary protocol for multimedia communications
- **MGCP/Megaco**: Protocols for controlling media gateways
- **WebRTC**: Browser-based real-time communications (often uses SIP for signaling)
- **XMPP**: Extensible messaging protocol that can also handle VoIP

SIP's advantages include its text-based nature, extensibility, and wide adoption in the telecommunications industry.

## Conclusion

In this tutorial, we've covered the basics of SIP, including its architecture, message structure, and call flow. In the next tutorial, we'll dive into parsing SIP messages using the `rvoip-sip-core` library.

## Exercise

1. Identify the different components in this SIP message:

```
REGISTER sip:registrar.example.com SIP/2.0
Via: SIP/2.0/UDP user-pc.example.com:5060;branch=z9hG4bKnashds7
Max-Forwards: 70
To: User <sip:user@example.com>
From: User <sip:user@example.com>;tag=a73kszlfl
Call-ID: 1j9FpLxk3uxtm8tn@user-pc.example.com
CSeq: 1 REGISTER
Contact: <sip:user@user-pc.example.com>
Expires: 3600
Content-Length: 0
```

2. What would a 200 OK response to this REGISTER request look like?

## References

- [RFC 3261: SIP: Session Initiation Protocol](https://datatracker.ietf.org/doc/html/rfc3261)
- [RFC 3665: SIP Basic Call Flow Examples](https://datatracker.ietf.org/doc/html/rfc3665)
- [RFC 5359: Session Initiation Protocol Service Examples](https://datatracker.ietf.org/doc/html/rfc5359) 