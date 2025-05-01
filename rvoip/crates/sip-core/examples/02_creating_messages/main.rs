//! Creating SIP Messages Example
//! 
//! This example demonstrates how to create SIP requests and responses
//! using both the builder pattern and the more concise macro syntax.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use tracing::info;

fn main() {
    // Initialize logging so we can see what's happening
    tracing_subscriber::fmt::init();
    
    info!("SIP Core Creating Messages Example");
    
    // Example 1: Creating a SIP request using the builder pattern
    create_request_with_builder();
    
    // Example 2: Creating a SIP response using the builder pattern
    create_response_with_builder();
    
    // Example 3: Using macros for concise message creation
    create_message_with_macros();
    
    // Example 4: Creating messages with complex bodies
    create_message_with_body();
    
    info!("All examples completed successfully!");
}

/// Example 1: Creating a SIP request using the builder pattern
fn create_request_with_builder() {
    info!("Example 1: Creating a SIP request using the builder pattern");
    
    // Create URIs for the request
    let bob_uri = "sip:bob@example.com".parse::<Uri>().unwrap();
    let alice_uri = "sip:alice@atlanta.com".parse::<Uri>().unwrap();
    let contact_uri = "sip:alice@pc33.atlanta.com".parse::<Uri>().unwrap();
    
    // Build the INVITE request
    let request = RequestBuilder::new(Method::Invite, &bob_uri.to_string())
        .unwrap()
        .header(TypedHeader::From(From::new(
            Address::new_with_display_name("Alice", alice_uri)
                .with_parameter("tag", "1928301774")
        )))
        .header(TypedHeader::To(To::new(
            Address::new_with_display_name("Bob", bob_uri)
        )))
        .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
        .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
        .header(TypedHeader::Via(
            Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap()
        ))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::Contact(Contact::new(
            Address::new(contact_uri)
        )))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();
    
    // Convert to bytes and display
    let message_bytes = request.to_bytes();
    info!("Created SIP request:\n{}", std::str::from_utf8(&message_bytes).unwrap());
    
    // Demonstrate how to update parts of a request
    let updated_request = request
        .with_header(TypedHeader::Subject(Subject::new("Urgent call")))
        .with_header(TypedHeader::Priority(Priority::new(PriorityValue::Urgent)));
    
    info!("Added Subject and Priority headers");
    
    // Check if the new request has the headers we added
    if updated_request.typed_header::<Subject>().is_some() {
        info!("Subject header was successfully added");
    }
    
    if let Some(priority) = updated_request.typed_header::<Priority>() {
        info!("Priority was set to: {}", priority.value());
    }
}

/// Example 2: Creating a SIP response using the builder pattern
fn create_response_with_builder() {
    info!("Example 2: Creating a SIP response using the builder pattern");
    
    // Create URIs for the response
    let bob_uri = "sip:bob@example.com".parse::<Uri>().unwrap();
    let alice_uri = "sip:alice@atlanta.com".parse::<Uri>().unwrap();
    let contact_uri = "sip:bob@192.0.2.4".parse::<Uri>().unwrap();
    
    // Build a 200 OK response to an INVITE
    let response = ResponseBuilder::new(StatusCode::OK)
        .unwrap()
        .header(TypedHeader::Via(
            Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap()
        ))
        .header(TypedHeader::From(From::new(
            Address::new_with_display_name("Alice", alice_uri)
                .with_parameter("tag", "1928301774")
        )))
        .header(TypedHeader::To(To::new(
            Address::new_with_display_name("Bob", bob_uri)
                .with_parameter("tag", "a6c85cf")
        )))
        .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
        .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
        .header(TypedHeader::Contact(Contact::new(
            Address::new(contact_uri)
        )))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();
    
    // Convert to bytes and display
    let message_bytes = response.to_bytes();
    info!("Created SIP response:\n{}", std::str::from_utf8(&message_bytes).unwrap());
    
    // Create other common response types
    
    // 180 Ringing - typical intermediate response
    let ringing_response = ResponseBuilder::new(StatusCode::Ringing)
        .unwrap()
        .header(TypedHeader::Via(
            Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap()
        ))
        .header(TypedHeader::From(From::new(
            Address::new_with_display_name("Alice", alice_uri.clone())
                .with_parameter("tag", "1928301774")
        )))
        .header(TypedHeader::To(To::new(
            Address::new_with_display_name("Bob", bob_uri.clone())
                .with_parameter("tag", "a6c85cf")
        )))
        .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
        .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();
    
    info!("Created 180 Ringing response");
    
    // 404 Not Found - error response
    let not_found_response = ResponseBuilder::new(StatusCode::NotFound)
        .unwrap()
        .header(TypedHeader::Via(
            Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap()
        ))
        .header(TypedHeader::From(From::new(
            Address::new_with_display_name("Alice", alice_uri)
                .with_parameter("tag", "1928301774")
        )))
        .header(TypedHeader::To(To::new(
            Address::new_with_display_name("Bob", bob_uri)
                .with_parameter("tag", "a6c85cf")
        )))
        .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
        .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build();
    
    info!("Created 404 Not Found response");
    
    // Get the status code and reason phrase
    info!("Status: {} {}", not_found_response.status_code(), not_found_response.reason_phrase());
}

/// Example 3: Using macros for concise message creation
fn create_message_with_macros() {
    info!("Example 3: Using macros for concise message creation");
    
    // Create a SIP request with the sip! macro
    let request = sip! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        headers: {
            Via: "SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds",
            MaxForwards: 70,
            To: "Bob <sip:bob@example.com>",
            From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
            CallId: "a84b4c76e66710@pc33.atlanta.com",
            CSeq: "314159 INVITE",
            Contact: "<sip:alice@pc33.atlanta.com>",
            ContentLength: 0
        }
    };
    
    // Convert to bytes and display
    let message_bytes = request.to_bytes();
    info!("Created SIP request using macro:\n{}", std::str::from_utf8(&message_bytes).unwrap());
    
    // Create a SIP response with the sip! macro
    let response = sip! {
        status: StatusCode::OK,
        headers: {
            Via: "SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds",
            To: "Bob <sip:bob@example.com>;tag=a6c85cf",
            From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
            CallId: "a84b4c76e66710@pc33.atlanta.com",
            CSeq: "314159 INVITE",
            Contact: "<sip:bob@192.0.2.4>",
            ContentLength: 0
        }
    };
    
    // Convert to bytes and display
    let message_bytes = response.to_bytes();
    info!("Created SIP response using macro:\n{}", std::str::from_utf8(&message_bytes).unwrap());
}

/// Example 4: Creating messages with complex bodies
fn create_message_with_body() {
    info!("Example 4: Creating messages with bodies");
    
    // Create a simple SDP body
    let sdp_body = 
        "v=0\r\n\
         o=alice 2890844526 2890844526 IN IP4 pc33.atlanta.com\r\n\
         s=Session SDP\r\n\
         c=IN IP4 pc33.atlanta.com\r\n\
         t=0 0\r\n\
         m=audio 49172 RTP/AVP 0\r\n\
         a=rtpmap:0 PCMU/8000\r\n";
    
    // Create a SIP INVITE with SDP body
    let invite_with_sdp = sip! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        headers: {
            Via: "SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds",
            MaxForwards: 70,
            To: "Bob <sip:bob@example.com>",
            From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
            CallId: "a84b4c76e66710@pc33.atlanta.com",
            CSeq: "314159 INVITE",
            Contact: "<sip:alice@pc33.atlanta.com>",
            ContentType: "application/sdp",
            ContentLength: sdp_body.len()
        },
        body: sdp_body
    };
    
    // Convert to bytes and display
    let message_bytes = invite_with_sdp.to_bytes();
    info!("Created SIP INVITE with SDP body:\n{}", std::str::from_utf8(&message_bytes).unwrap());
    
    // Alternative way to set the body using builder
    let bob_uri = "sip:bob@example.com".parse::<Uri>().unwrap();
    let alice_uri = "sip:alice@atlanta.com".parse::<Uri>().unwrap();
    let contact_uri = "sip:alice@pc33.atlanta.com".parse::<Uri>().unwrap();
    
    let invite_with_builder = RequestBuilder::new(Method::Invite, &bob_uri.to_string())
        .unwrap()
        .header(TypedHeader::From(From::new(
            Address::new_with_display_name("Alice", alice_uri)
                .with_parameter("tag", "1928301774")
        )))
        .header(TypedHeader::To(To::new(
            Address::new_with_display_name("Bob", bob_uri)
        )))
        .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
        .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
        .header(TypedHeader::Via(
            Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap()
        ))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::Contact(Contact::new(
            Address::new(contact_uri)
        )))
        .header(TypedHeader::ContentType(ContentType::new("application/sdp")))
        .header(TypedHeader::ContentLength(ContentLength::new(sdp_body.len())))
        .body(Bytes::from(sdp_body))
        .build();
    
    info!("Created SIP INVITE with SDP body using builder");
    
    // Parse the SIP message and extract the body
    let parsed_message = parse_message(&invite_with_builder.to_bytes()).unwrap();
    if let Message::Request(request) = parsed_message {
        if let Some(body) = request.body() {
            info!("Extracted body from parsed message:\n{}", std::str::from_utf8(body).unwrap());
            
            // Check content type
            if let Some(content_type) = request.typed_header::<ContentType>() {
                info!("Content-Type: {}", content_type);
            }
        } else {
            info!("No body found in parsed message");
        }
    }
} 