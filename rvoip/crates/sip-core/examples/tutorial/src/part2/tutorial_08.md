# Integrating SDP with SIP

In the previous tutorials, we explored SIP messaging and SDP creation separately. In this tutorial, we'll bring these two protocols together to establish multimedia sessions. SIP and SDP work hand-in-hand, with SIP handling the signaling and session management, while SDP describes the media parameters.

## SDP as a SIP Message Body

SIP messages can carry various content types in their message bodies. For multimedia session establishment, SDP is the standard format. When including SDP in a SIP message:

1. The Content-Type header must be set to `application/sdp`
2. The Content-Length header must reflect the size of the SDP body
3. The SDP content follows the headers, separated by a blank line

Here's how to include SDP in a SIP INVITE request using the `rvoip-sip-core` library:

```rust
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::RequestBuilder;
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use std::error::Error;

fn create_invite_with_sdp() -> Result<Message, Box<dyn Error>> {
    // 1. Create the SDP offer
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
    
    // 2. Convert SDP to string for inclusion in SIP message
    let sdp_string = sdp.to_string();
    
    // 3. Create SIP INVITE request with SDP offer
    let invite = RequestBuilder::new(Method::Invite, "sip:bob@biloxi.example.com")
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .contact("sip:alice@pc33.atlanta.example.com", None)
        // Set Content-Type header for SDP
        .content_type("application/sdp")
        // Include SDP as body
        .body(sdp_string)
        .build()?;
    
    Ok(invite)
}
```

The key components are:
- Using `.content_type("application/sdp")` to specify the body format
- Using `.body(sdp_string)` to include the SDP content

## The SIP/SDP Offer/Answer Model

SIP uses the offer/answer model defined in [RFC 3264](https://datatracker.ietf.org/doc/html/rfc3264) for negotiating media sessions. The basic flow is:

1. The caller sends an INVITE with an SDP offer
2. The callee responds with a 200 OK including an SDP answer
3. The caller acknowledges with an ACK

Let's implement this flow:

```rust
// 1. Create and send INVITE with SDP offer
let invite_with_offer = create_invite_with_sdp()?;
send_message(invite_with_offer);

// 2. Receive INVITE, parse it, and create a 200 OK response with SDP answer
fn handle_incoming_invite(invite: Message) -> Result<Message, Box<dyn Error>> {
    // Extract SDP from incoming INVITE
    let incoming_sdp = if let Some(body) = invite.body() {
        if invite.content_type() == Some("application/sdp") {
            Some(SdpSession::from_str(body)?)
        } else {
            None
        }
    } else {
        None
    };
    
    // Create SDP answer based on the offer
    let sdp_answer = match incoming_sdp {
        Some(offer) => {
            SdpBuilder::new("Answer Session")
                .origin("bob", "2890844527", "2890844527", "IN", "IP4", "bob.example.com")
                .connection("IN", "IP4", "bob.example.com")
                .time("0", "0")
                .media_audio(49180, "RTP/AVP")
                    // Select only one codec from the offer
                    .formats(&["0"])
                    .rtpmap("0", "PCMU/8000")
                    .direction(MediaDirection::SendRecv)
                    .done()
                .build()?
                .to_string()
        },
        None => {
            // No SDP in the INVITE, reject the call
            return create_error_response(invite, StatusCode::NotAcceptable, "No SDP offer found");
        }
    };
    
    // Create 200 OK response with SDP answer
    let response = ResponseBuilder::response_from_request(&invite, StatusCode::Ok, None)
        .contact("sip:bob@biloxi.example.com", None)
        .content_type("application/sdp")
        .body(sdp_answer)
        .build()?;
    
    Ok(response)
}

// 3. Send ACK to complete the transaction
fn acknowledge_response(response: &Message, original_invite: &Message) -> Result<Message, Box<dyn Error>> {
    // Create ACK request
    let ack = RequestBuilder::new(Method::Ack, "sip:bob@biloxi.example.com")
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id(original_invite.call_id().unwrap())
        .cseq(original_invite.cseq().unwrap().0, Method::Ack)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .build()?;
    
    Ok(ack)
}
```

## Parsing SDP from SIP Messages

When receiving a SIP message with an SDP body, you need to:

1. Check if the Content-Type is `application/sdp`
2. Extract the message body
3. Parse the body into an SdpSession

Here's a function to extract SDP from any SIP message:

```rust
fn extract_sdp_from_message(message: &Message) -> Option<Result<SdpSession, Error>> {
    // Check if the message has a body and the content type is application/sdp
    if let Some(body) = message.body() {
        if message.content_type() == Some("application/sdp") {
            // Try to parse the body as SDP
            return Some(SdpSession::from_str(body));
        }
    }
    
    None
}
```

## Session Modification

After a session is established, either party can modify the session parameters by sending a re-INVITE with a new SDP offer. The flow is the same as the initial INVITE, but occurs within an established dialog.

```rust
// Creating a re-INVITE to modify an existing session
fn create_reinvite_with_updated_sdp(original_invite: &Message) -> Result<Message, Box<dyn Error>> {
    // Create updated SDP with video added
    let updated_sdp = SdpBuilder::new("Audio/Video Call")
        .origin("alice", "2890844526", "2890844528", "IN", "IP4", "alice.example.com")
        .connection("IN", "IP4", "alice.example.com")
        .time("0", "0")
        // Existing audio stream
        .media_audio(49170, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        // New video stream
        .media_video(49174, "RTP/AVP")
            .formats(&["31"])
            .rtpmap("31", "H261/90000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    // Create re-INVITE with new SDP
    let reinvite = RequestBuilder::new(Method::Invite, "sip:bob@biloxi.example.com")
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id(original_invite.call_id().unwrap())
        // Increment CSeq for the new transaction
        .cseq(original_invite.cseq().unwrap().0 + 1, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds8"))
        .max_forwards(70)
        .contact("sip:alice@pc33.atlanta.example.com", None)
        .content_type("application/sdp")
        .body(updated_sdp.to_string())
        .build()?;
    
    Ok(reinvite)
}
```

## Handling SDP Negotiation Failures

Sometimes SDP negotiation fails because the endpoints cannot agree on compatible parameters. In such cases, the responding party should send an appropriate error response:

```rust
fn handle_incompatible_sdp(invite: &Message) -> Result<Message, Box<dyn Error>> {
    // Extract SDP from incoming INVITE
    let sdp_option = extract_sdp_from_message(invite);
    
    // Check if the SDP is compatible
    if let Some(Ok(sdp)) = sdp_option {
        // For example, check if the codecs are supported
        let supported = false; // Placeholder for compatibility check
        
        if !supported {
            // Create 488 Not Acceptable Here response
            let response = ResponseBuilder::response_from_request(invite, StatusCode::NotAcceptableHere, None)
                .warning("304", "atlanta.example.com", "Incompatible media format")
                .build()?;
            
            return Ok(response);
        }
    }
    
    // If we reach here, there's another issue with the SDP
    let response = ResponseBuilder::response_from_request(invite, StatusCode::BadRequest, Some("Invalid SDP"))
        .build()?;
    
    Ok(response)
}
```

## Best Practices for SIP/SDP Integration

1. **Content Negotiation**: Always check the Content-Type header before parsing the body
2. **Codec Selection**: In answers, only include codecs that were present in the offer
3. **Connection Information**: Ensure that the c= line in SDP contains a reachable address
4. **SDP Version**: Increment the origin session version (o= line) when sending a new SDP
5. **Media Line Order**: Maintain the same media line order in answers as in offers
6. **Early Media**: Use reliable provisional responses (100rel) for early media scenarios
7. **ICE Integration**: If using ICE for NAT traversal, include ICE candidates in SDP

## Example: Complete INVITE/200 OK/ACK Flow with SDP

Here's a complete example showing the full SIP/SDP negotiation flow:

```rust
// Setup stage
let call_id = "3848276298220188511@atlanta.example.com";
let from_tag = "9fxced76sl";
let cseq = 314159;

// Step 1: Create and send INVITE with SDP offer
let invite = RequestBuilder::new(Method::Invite, "sip:bob@biloxi.example.com")
    .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
    .to("Bob", "sip:bob@biloxi.example.com", None)
    .call_id(call_id)
    .cseq(cseq, Method::Invite)
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .max_forwards(70)
    .contact("sip:alice@pc33.atlanta.example.com", None)
    .content_type("application/sdp")
    .body(create_offer_sdp("alice", "atlanta.example.com")?)
    .build()?;

// Send INVITE and wait for response...

// Step 2: Receive 200 OK with SDP answer
let ok_response = ResponseBuilder::new(StatusCode::Ok, None)
    .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
    .to("Bob", "sip:bob@biloxi.example.com", Some("8a8sdg87s"))  // Bob's tag
    .call_id(call_id)
    .cseq(cseq, Method::Invite)
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
    .contact("sip:bob@192.0.2.4", None)
    .content_type("application/sdp")
    .body(create_answer_sdp("bob", "biloxi.example.com")?)
    .build()?;

// Step 3: Send ACK to acknowledge the 200 OK
let ack = RequestBuilder::new(Method::Ack, "sip:bob@192.0.2.4")  // From Contact in 200 OK
    .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
    .to("Bob", "sip:bob@biloxi.example.com", Some("8a8sdg87s"))  // Bob's tag
    .call_id(call_id)
    .cseq(cseq, Method::Ack)
    .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds9"))
    .max_forwards(70)
    .build()?;

// Session is now established!

// Helper functions for creating SDP
fn create_offer_sdp(username: &str, domain: &str) -> Result<String, Error> {
    let sdp = SdpBuilder::new("Call Offer")
        .origin(username, "2890844526", "2890844526", "IN", "IP4", domain)
        .connection("IN", "IP4", domain)
        .time("0", "0")
        .media_audio(49170, "RTP/AVP")
            .formats(&["0", "8"])
            .rtpmap("0", "PCMU/8000")
            .rtpmap("8", "PCMA/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    Ok(sdp.to_string())
}

fn create_answer_sdp(username: &str, domain: &str) -> Result<String, Error> {
    let sdp = SdpBuilder::new("Call Answer")
        .origin(username, "2890844527", "2890844527", "IN", "IP4", domain)
        .connection("IN", "IP4", domain)
        .time("0", "0")
        .media_audio(49180, "RTP/AVP")
            .formats(&["0"])  // Choose only PCMU
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    Ok(sdp.to_string())
}
```

## Conclusion

In this tutorial, we've explored how to integrate SDP with SIP to establish and modify multimedia sessions. We've covered:

- Including SDP bodies in SIP messages
- Implementing the SDP offer/answer model with SIP signaling
- Parsing SDP from incoming SIP messages
- Modifying sessions with re-INVITEs
- Handling SDP negotiation failures
- Best practices for SIP/SDP integration

In the next tutorial, we'll dive deeper into media negotiation with SDP, exploring advanced topics like handling multiple media streams, codec preferences, and handling SDP in complex call scenarios.
