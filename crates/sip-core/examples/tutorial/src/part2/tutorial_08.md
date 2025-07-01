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
use rvoip_sip_core::{RequestBuilder, ResponseBuilder};
use rvoip_sip_core::sdp::SdpBuilder;
use rvoip_sip_core::sdp::attributes::MediaDirection;
use rvoip_sip_core::types::sdp::SdpSession;
use rvoip_sip_core::json::SipJsonExt;  // Import for JSON access
use rvoip_sip_core::json::ext::SipMessageJson; // Import for SIP header convenience methods
use std::error::Error as StdError;
use std::str::FromStr;
use bytes::Bytes;

fn create_invite_with_sdp() -> Result<Message, Box<dyn StdError>> {
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
    let invite = RequestBuilder::new(Method::Invite, "sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some("9fxced76sl"))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id("3848276298220188511@atlanta.example.com")
        .cseq(314159)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .contact("sip:alice@pc33.atlanta.example.com", None)
        // Set Content-Type header for SDP
        .content_type("application/sdp")
        // Include SDP as body (using Bytes)
        .body(Bytes::from(sdp_string))
        .build();
    
    Ok(Message::Request(invite))
}
```

The key components are:
- Using `.content_type("application/sdp")` to specify the body format
- Using `.body(Bytes::from(sdp_string))` to include the SDP content as bytes

## Extracting SDP from SIP Messages

When receiving a SIP message with an SDP body, you need to extract and parse it. Here's a robust method:

```rust
fn extract_sdp_from_message(message: &Message) -> Option<std::result::Result<SdpSession, Error>> {
    // Check for body content
    let bytes = message.body();
    if !bytes.is_empty() {
        // Convert bytes to string
        if let Ok(body_str) = std::str::from_utf8(bytes) {
            // Verify this looks like SDP (starts with v=0)
            if body_str.trim_start().starts_with("v=0") {
                return Some(SdpSession::from_str(body_str));
            }
        }
    }
    
    None
}
```

This function:
1. Gets the raw bytes from the message body
2. Converts them to a UTF-8 string
3. Verifies it looks like SDP by checking for "v=0" at the start
4. Returns a Result containing the parsed SDP session or an error

## The SIP/SDP Offer/Answer Model

SIP uses the offer/answer model defined in [RFC 3264](https://datatracker.ietf.org/doc/html/rfc3264) for negotiating media sessions. The basic flow is:

1. The caller sends an INVITE with an SDP offer
2. The callee responds with a 200 OK including an SDP answer
3. The caller acknowledges with an ACK

Let's implement responding to an INVITE with a 200 OK containing an SDP answer:

```rust
fn create_response_with_sdp_answer(invite: &Message) -> Result<Message, Error> {
    // Extract the SDP offer from the INVITE
    let incoming_sdp = if let Some(sdp_result) = extract_sdp_from_message(invite) {
        sdp_result?
    } else {
        return Err(Error::Parser("No SDP in INVITE".into()));
    };
    
    // Create SDP answer based on the offer
    let sdp_answer = SdpBuilder::new("Answer Session")
        .origin("bob", "2890844527", "2890844527", "IN", "IP4", "bob.example.com")
        .connection("IN", "IP4", "bob.example.com")
        .time("0", "0")
        .media_audio(49180, "RTP/AVP")
            // Select only one codec from the offer
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    // Create 200 OK response with SDP answer
    if let Message::Request(req) = invite {
        // Use the dialog_response helper method
        let response = ResponseBuilder::dialog_response(
            req,
            StatusCode::Ok,
            None
        )
        .to("Bob", "sip:bob@biloxi.example.com", Some("8a8sdg87s")) // Add/override To with tag
        .contact("sip:bob@biloxi.example.com", None)
        .content_type("application/sdp")
        .body(Bytes::from(sdp_answer.to_string()))
        .build();
        
        Ok(Message::Response(response))
    } else {
        Err(Error::Parser("Not a request".into()))
    }
}
```

Note the use of the `dialog_response` helper method, which creates a response that includes all the necessary headers for dialog establishment.

## JSON Path Access to SIP Headers

Our library provides powerful JSON path access to SIP message headers, making it easy to extract values nested deep within the message structure. This is essential for proper dialog handling:

```rust
// Getting values with path accessors
let to_display_name = response.path_str_or("headers.To.display_name", "");
let to_uri = response.path_str_or("headers.To.uri", "");
let to_tag = response.path_str("headers.To.params[0].Tag");

// This also works for complex deeply nested structures like Via headers
let branch = response.path_str("headers.Via[0].params[0].Branch");

// You can use it to extract values with defaults
let from_display_name = invite.path_str_or("headers.From.display_name", "Alice");
```

These path accessors:
- Handle the internal Request/Response wrapping structure
- Provide case-insensitive header matching
- Support array indexing for multi-value headers
- Handle nested parameter structures like Via and From/To headers

## Creating an ACK to Complete Dialog Establishment

After receiving a 200 OK response, you need to send an ACK to complete the dialog establishment:

```rust
fn create_ack_for_response(response: &Message, original_invite: &Message) -> Result<Message, Error> {
    // Get information from the response and original INVITE using JSON path accessors
    let to_display_name = response.path_str_or("headers.To.display_name", "");
    let to_uri = response.path_str_or("headers.To.uri", "");
    let to_tag = response.path_str("headers.To.params[0].Tag");
    
    // Use a simple default for the contact URI or extract from Contact header
    let contact_uri = "sip:bob@192.0.2.4";
    
    // Get From, Call-ID, and CSeq from original INVITE
    let from_display_name = original_invite.path_str_or("headers.From.display_name", "Alice");
    let from_uri = original_invite.path_str_or("headers.From.uri", "sip:alice@atlanta.example.com");
    let from_tag = original_invite.path_str("headers.From.params[0].Tag");
    let call_id = original_invite.path_str_or("headers.CallId", "3848276298220188511@atlanta.example.com");
    let cseq = original_invite.path("headers.CSeq.seq")
        .and_then(|v| v.as_i64())
        .unwrap_or(314159) as u32;
    
    // Create ACK request
    let ack = RequestBuilder::new(Method::Ack, contact_uri)?
        .from(&from_display_name, &from_uri, from_tag.as_deref())
        .to(&to_display_name, &to_uri, to_tag.as_deref())
        .call_id(&call_id)
        .cseq(cseq)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .build();
    
    Ok(Message::Request(ack))
}
```

## Session Modification with re-INVITE

After a session is established, either party can modify the session parameters by sending a re-INVITE with a new SDP offer:

```rust
fn create_reinvite_with_updated_sdp(original_invite: &Message) -> Result<Message, Error> {
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
    
    // Get headers from the original INVITE using JSON path
    let from_display_name = original_invite.path_str_or("headers.From.display_name", "Alice");
    let from_uri = original_invite.path_str_or("headers.From.uri", "sip:alice@atlanta.example.com");
    let from_tag = original_invite.path_str("headers.From.params[0].Tag");
    let to_display_name = original_invite.path_str_or("headers.To.display_name", "Bob");
    let to_uri = original_invite.path_str_or("headers.To.uri", "sip:bob@biloxi.example.com");
    let call_id = original_invite.path_str_or("headers.CallId", "3848276298220188511@atlanta.example.com");
    
    // Get CSeq and increment
    let cseq = original_invite.path("headers.CSeq.seq")
        .and_then(|v| v.as_i64())
        .unwrap_or(314159) as u32 + 1;
    
    // Create re-INVITE with new SDP
    let reinvite = RequestBuilder::new(Method::Invite, "sip:bob@biloxi.example.com")?
        .from(&from_display_name, &from_uri, from_tag.as_deref())
        .to(&to_display_name, &to_uri, None)  // No To tag for reinvite
        .call_id(&call_id)
        .cseq(cseq)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds8"))
        .max_forwards(70)
        .contact("sip:alice@pc33.atlanta.example.com", None)
        .content_type("application/sdp")
        .body(Bytes::from(updated_sdp.to_string()))
        .build();
    
    Ok(Message::Request(reinvite))
}
```

Note that in the re-INVITE:
- We increment the CSeq number
- We include an updated SDP with a new origin version
- We use the JSON path accessor to extract headers from the original INVITE

## Handling SDP Negotiation Failures

Sometimes SDP negotiation fails because the endpoints cannot agree on compatible parameters:

```rust
fn handle_incompatible_sdp(invite: &Message) -> Result<Message, Error> {
    // Extract SDP from incoming INVITE
    let sdp_option = extract_sdp_from_message(invite);
    
    // Check if the SDP is compatible
    match sdp_option {
        Some(Ok(sdp)) => {
            // In a real implementation, we would check supported codecs
            // For this example, we'll assume incompatibility
            
            println!("Received SDP with unsupported codecs:");
            for media in &sdp.media_descriptions {
                if media.media == "audio" {
                    println!("Audio formats: {:?}", media.formats);
                    
                    if let Message::Request(req) = invite {
                        // Try parsing URI for warning
                        let warning_agent = Uri::from_str("sip:biloxi.example.com").unwrap_or_else(|_| {
                            // Fallback to domain with SIP scheme
                            Uri::sip("biloxi.example.com")
                        });
                        
                        // Create a 488 Not Acceptable Here response
                        let response = ResponseBuilder::error_response(
                            req,
                            StatusCode::NotAcceptableHere, 
                            None
                        )
                        .warning(304, warning_agent, "Incompatible media format")
                        .build();
                        
                        return Ok(Message::Response(response));
                    }
                }
            }
        },
        Some(Err(_)) => {
            // SDP parsing failed
            if let Message::Request(req) = invite {
                // Create a 400 Bad Request response with "Invalid SDP" reason phrase
                let response = ResponseBuilder::error_response(
                    req,
                    StatusCode::BadRequest,
                    Some("Invalid SDP")
                )
                .build();
                
                return Ok(Message::Response(response));
            }
        },
        None => {
            // No SDP in the INVITE
            if let Message::Request(req) = invite {
                // Create a 406 Not Acceptable response with "SDP Required" reason phrase
                let response = ResponseBuilder::error_response(
                    req,
                    StatusCode::NotAcceptable,
                    Some("SDP Required")
                )
                .build();
                
                return Ok(Message::Response(response));
            }
        }
    }
    
    // If we reach here, there's another issue with the request
    if let Message::Request(req) = invite {
        // Create a 400 Bad Request response
        let response = ResponseBuilder::error_response(
            req,
            StatusCode::BadRequest,
            None
        )
        .build();
        
        return Ok(Message::Response(response));
    }
    
    Err(Error::Parser("Not a request".into()))
}
```

Notice how the warning header is created with:
- An integer status code (304)
- A Uri object for the warning agent
- A text message explaining the issue

## Complete SIP/SDP Dialog Flow

Here's a complete dialog flow showing all stages from INVITE to BYE:

```rust
fn demonstrate_complete_dialog_flow() -> Result<(), Error> {
    // Setup parameters
    let call_id = "3848276298220188511@atlanta.example.com";
    let from_tag = "9fxced76sl";
    let to_tag = "8a8sdg87s";  // Will be added by Bob
    let cseq = 314159;
    
    // Step 1: Create and send INVITE with SDP offer
    println!("Step 1: Initial INVITE with SDP offer");
    let invite = RequestBuilder::new(Method::Invite, "sip:bob@biloxi.example.com")?
        .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
        .to("Bob", "sip:bob@biloxi.example.com", None)
        .call_id(call_id)
        .cseq(cseq)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .max_forwards(70)
        .contact("sip:alice@pc33.atlanta.example.com", None)
        .content_type("application/sdp")
        .body(Bytes::from(create_offer_sdp("alice", "atlanta.example.com")?))
        .build();
    
    println!("{}\n", Message::Request(invite));
    
    // Step 2: Receive 180 Ringing (no SDP)
    println!("Step 2: 180 Ringing (no SDP)");
    let ringing = ResponseBuilder::new(StatusCode::Ringing, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
        .to("Bob", "sip:bob@biloxi.example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .contact("sip:bob@192.0.2.4", None)
        .build();
    
    println!("{}\n", Message::Response(ringing));
    
    // Step 3: Receive 200 OK with SDP answer
    println!("Step 3: 200 OK with SDP answer");
    let ok_response = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
        .to("Bob", "sip:bob@biloxi.example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds7"))
        .contact("sip:bob@192.0.2.4", None)
        .content_type("application/sdp")
        .body(Bytes::from(create_answer_sdp("bob", "biloxi.example.com")?))
        .build();
    
    println!("{}\n", Message::Response(ok_response));
    
    // Step 4: Send ACK to acknowledge the 200 OK
    println!("Step 4: ACK to acknowledge 200 OK");
    
    // Using JSON path accessors to get contact URI
    let contact_uri = "sip:bob@192.0.2.4".to_string();
    
    let ack = RequestBuilder::new(Method::Ack, &contact_uri)?
        .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
        .to("Bob", "sip:bob@biloxi.example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds9"))
        .max_forwards(70)
        .build();
    
    println!("{}\n", Message::Request(ack));
    
    // Step 5: After some time, send re-INVITE to add video
    println!("Step 5: re-INVITE to add video");
    
    // Using convenience methods from SipMessageJson
    let contact_uri = "sip:bob@192.0.2.4".to_string();
    
    let reinvite = RequestBuilder::new(Method::Invite, &contact_uri)?
        .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
        .to("Bob", "sip:bob@biloxi.example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq + 1)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds10"))
        .max_forwards(70)
        .contact("sip:alice@pc33.atlanta.example.com", None)
        .content_type("application/sdp")
        .body(Bytes::from(create_updated_sdp("alice", "atlanta.example.com")?))
        .build();
    
    println!("{}\n", Message::Request(reinvite));
    
    // Step 6: Receive 200 OK for re-INVITE with updated SDP
    println!("Step 6: 200 OK for re-INVITE with updated SDP");
    
    let ok_reinvite = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
        .to("Bob", "sip:bob@biloxi.example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq + 1, Method::Invite)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds10"))
        .contact("sip:bob@192.0.2.4", None)
        .content_type("application/sdp")
        .body(Bytes::from(create_video_answer_sdp("bob", "biloxi.example.com")?))
        .build();
    
    println!("{}\n", Message::Response(ok_reinvite));
    
    // Step 7: ACK the 200 OK for re-INVITE
    println!("Step 7: ACK for 200 OK of re-INVITE");
    
    // Using convenience methods to access URI
    let contact_uri = "sip:bob@192.0.2.4".to_string();
    
    let ack_reinvite = RequestBuilder::new(Method::Ack, &contact_uri)?
        .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
        .to("Bob", "sip:bob@biloxi.example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq + 1)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds11"))
        .max_forwards(70)
        .build();
    
    println!("{}\n", Message::Request(ack_reinvite));
    
    // Step 8: Later, send BYE to terminate the session
    println!("Step 8: BYE to terminate session");
    
    let bye = RequestBuilder::new(Method::Bye, &contact_uri)?
        .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
        .to("Bob", "sip:bob@biloxi.example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq + 2)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds12"))
        .max_forwards(70)
        .build();
    
    println!("{}\n", Message::Request(bye));
    
    // Step 9: Receive 200 OK for BYE
    println!("Step 9: 200 OK for BYE");
    
    let ok_bye = ResponseBuilder::new(StatusCode::Ok, None)
        .from("Alice", "sip:alice@atlanta.example.com", Some(from_tag))
        .to("Bob", "sip:bob@biloxi.example.com", Some(to_tag))
        .call_id(call_id)
        .cseq(cseq + 2, Method::Bye)
        .via("atlanta.example.com:5060", "UDP", Some("z9hG4bKnashds12"))
        .build();
    
    println!("{}\n", Message::Response(ok_bye));
    
    println!("Dialog completed successfully!");
    
    Ok(())
}
```

## Helper Functions for SDP Creation

These helper functions create different SDP messages for various stages of the call:

```rust
// Initial offer for audio call
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

// Answer accepting only PCMU codec
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

// Updated offer adding video
fn create_updated_sdp(username: &str, domain: &str) -> Result<String, Error> {
    let sdp = SdpBuilder::new("Updated Call Offer")
        .origin(username, "2890844526", "2890844528", "IN", "IP4", domain)  // Increment version
        .connection("IN", "IP4", domain)
        .time("0", "0")
        // Existing audio stream
        .media_audio(49170, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        // New video stream
        .media_video(49174, "RTP/AVP")
            .formats(&["31", "34"])
            .rtpmap("31", "H261/90000")
            .rtpmap("34", "H263/90000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    Ok(sdp.to_string())
}

// Answer accepting audio and one video codec
fn create_video_answer_sdp(username: &str, domain: &str) -> Result<String, Error> {
    let sdp = SdpBuilder::new("Video Call Answer")
        .origin(username, "2890844527", "2890844528", "IN", "IP4", domain)  // Increment version
        .connection("IN", "IP4", domain)
        .time("0", "0")
        // Audio stream
        .media_audio(49180, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(MediaDirection::SendRecv)
            .done()
        // Video stream - accept H.261 only
        .media_video(49182, "RTP/AVP")
            .formats(&["31"])
            .rtpmap("31", "H261/90000")
            .direction(MediaDirection::SendRecv)
            .done()
        .build()?;
    
    Ok(sdp.to_string())
}
```

## Best Practices for SIP/SDP Integration

1. **Convert SDP to Bytes for Body**: Always convert SDP strings to `Bytes` when using them as message bodies
2. **JSON Path Access**: Use the path accessor methods to extract values from complex SIP message structures
3. **Dialog-Specific Helpers**: Use the `dialog_response` method for 200 OK responses to establish dialogs
4. **SDP Version Management**: Increment the SDP version (in the origin line) when sending updated offers
5. **Proper CSeq Handling**: Increment CSeq numbers across transactions and maintain the same CSeq in final ACK
6. **Branch ID Management**: Use unique branch IDs in Via headers for each transaction
7. **Tag Handling**: Ensure the From tag remains consistent throughout the dialog, while the To tag (once added) is included in all subsequent messages

## Conclusion

In this tutorial, we've explored how to integrate SDP with SIP to establish, modify, and terminate multimedia sessions. We've covered:

- Including SDP bodies in SIP messages using the `Bytes` type
- Using JSON path accessors to extract headers and parameters from SIP messages
- Implementing the complete SDP offer/answer model with SIP signaling
- Creating and responding to re-INVITEs to modify a session
- Handling SDP negotiation failures
- Implementing a complete dialog flow from INVITE through BYE

The rvoip-sip-core library provides powerful tools for working with SIP/SDP integration, making it easy to implement compliant and robust VoIP applications.
