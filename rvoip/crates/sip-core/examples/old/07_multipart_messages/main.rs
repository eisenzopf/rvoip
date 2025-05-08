//! Multipart Message Handling Example
//!
//! This example demonstrates how to work with multipart MIME bodies in SIP messages,
//! including creating and parsing different content types.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use tracing::{debug, info};
use uuid::Uuid;

fn main() {
    // Initialize logging so we can see what's happening
    tracing_subscriber::fmt::init();
    
    info!("SIP Core Multipart Message Handling Example");
    
    // Example 1: Basic multipart message creation
    basic_multipart_message();
    
    // Example 2: Parsing multipart messages
    parsing_multipart_messages();
    
    // Example 3: Real-world example - REFER with Replaces
    refer_with_replaces();
    
    // Example 4: Real-world example - Session descriptions with alternative formats
    alternative_session_descriptions();
    
    info!("All examples completed successfully!");
}

/// Example 1: Basic multipart message creation
fn basic_multipart_message() {
    info!("Example 1: Basic multipart message creation");
    
    // Create a simple SDP body
    let sdp_body = concat!(
        "v=0\r\n",
        "o=alice 2890844526 2890844526 IN IP4 192.168.1.100\r\n",
        "s=SIP Call\r\n",
        "c=IN IP4 192.168.1.100\r\n",
        "t=0 0\r\n",
        "m=audio 49172 RTP/AVP 0 8\r\n",
        "a=rtpmap:0 PCMU/8000\r\n",
        "a=rtpmap:8 PCMA/8000\r\n"
    );
    
    // Create a simple XML body (e.g., PIDF presence document)
    let xml_body = concat!(
        "<?xml version='1.0' encoding='UTF-8'?>\r\n",
        "<presence xmlns='urn:ietf:params:xml:ns:pidf'\r\n",
        "          entity='sip:alice@example.com'>\r\n",
        "  <tuple id='a1'>\r\n",
        "    <status><basic>open</basic></status>\r\n",
        "    <contact>sip:alice@192.168.1.100</contact>\r\n",
        "  </tuple>\r\n",
        "</presence>\r\n"
    );
    
    // Create a multipart MIME body
    let boundary = "boundary1234";
    let mime_body = MultipartBody::new(boundary)
        .add_part(
            ContentType::new("application/sdp"),
            None,  // No Content-Disposition
            Bytes::from(sdp_body)
        )
        .add_part(
            ContentType::new("application/pidf+xml"),
            Some(ContentDisposition::new("render")),
            Bytes::from(xml_body)
        );
    
    // Create a SIP message with the multipart body
    let call_id = format!("{}@192.168.1.100", Uuid::new_v4().to_string().split('-').next().unwrap());
    let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    
    let multipart_message = sip! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        headers: {
            Via: format!("SIP/2.0/UDP 192.168.1.100:5060;branch={}", branch),
            From: "Alice <sip:alice@example.com>;tag=1928301774",
            To: "Bob <sip:bob@example.com>",
            CallId: call_id,
            CSeq: "1 INVITE",
            Contact: "<sip:alice@192.168.1.100:5060>",
            ContentType: format!("multipart/mixed; boundary={}", boundary),
            ContentLength: mime_body.to_bytes().len()
        },
        body: mime_body.to_bytes()
    };
    
    info!("Created INVITE with multipart body:");
    debug!("{}", std::str::from_utf8(&multipart_message.to_bytes()).unwrap());
    
    // Verify the content type is set correctly
    if let Some(content_type) = multipart_message.typed_header::<ContentType>() {
        info!("Content-Type: {}", content_type);
        info!("Is multipart: {}", content_type.is_multipart());
        
        if let Some(boundary_param) = content_type.parameter("boundary") {
            info!("Boundary parameter: {}", boundary_param);
        }
    }
    
    // Verify content length header
    if let Some(content_length) = multipart_message.typed_header::<ContentLength>() {
        info!("Content-Length: {}", content_length.value());
    }
}

/// Example 2: Parsing multipart messages
fn parsing_multipart_messages() {
    info!("Example 2: Parsing multipart messages");
    
    // Create a multipart message as a raw SIP message (similar to what would come over the network)
    let raw_message = concat!(
        "INVITE sip:bob@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK776asdhds\r\n",
        "Max-Forwards: 70\r\n",
        "To: Bob <sip:bob@example.com>\r\n",
        "From: Alice <sip:alice@example.com>;tag=1928301774\r\n",
        "Call-ID: a84b4c76e66710@192.168.1.100\r\n",
        "CSeq: 314159 INVITE\r\n",
        "Contact: <sip:alice@192.168.1.100:5060>\r\n",
        "Content-Type: multipart/mixed; boundary=boundary42\r\n",
        "Content-Length: 439\r\n",
        "\r\n",
        "--boundary42\r\n",
        "Content-Type: application/sdp\r\n",
        "\r\n",
        "v=0\r\n",
        "o=alice 2890844526 2890844526 IN IP4 192.168.1.100\r\n",
        "s=SIP Call\r\n",
        "c=IN IP4 192.168.1.100\r\n",
        "t=0 0\r\n",
        "m=audio 49172 RTP/AVP 0 8\r\n",
        "a=rtpmap:0 PCMU/8000\r\n",
        "a=rtpmap:8 PCMA/8000\r\n",
        "--boundary42\r\n",
        "Content-Type: application/pidf+xml\r\n",
        "Content-Disposition: render\r\n",
        "\r\n",
        "<?xml version='1.0' encoding='UTF-8'?>\r\n",
        "<presence xmlns='urn:ietf:params:xml:ns:pidf' entity='sip:alice@example.com'>\r\n",
        "  <tuple id='a1'>\r\n",
        "    <status><basic>open</basic></status>\r\n",
        "    <contact>sip:alice@192.168.1.100</contact>\r\n",
        "  </tuple>\r\n",
        "</presence>\r\n",
        "--boundary42--\r\n"
    );
    
    // Parse the raw message
    let parsed_message = Message::parse(raw_message.as_bytes()).expect("Failed to parse SIP message");
    
    // Get the Content-Type header
    if let Some(content_type) = parsed_message.typed_header::<ContentType>() {
        info!("Parsed Content-Type: {}", content_type);
        
        // Check if this is a multipart message
        if content_type.is_multipart() {
            // Get the boundary parameter
            if let Some(boundary) = content_type.parameter("boundary") {
                info!("Detected multipart message with boundary: {}", boundary);
                
                // Parse the multipart body
                if let Some(body) = parsed_message.body() {
                    let multipart = MultipartBody::parse(body, boundary).expect("Failed to parse multipart body");
                    
                    // Display information about each part
                    info!("Found {} parts in the multipart body:", multipart.parts().len());
                    
                    for (i, part) in multipart.parts().iter().enumerate() {
                        info!("Part {}: Content-Type: {}", i+1, part.content_type());
                        
                        if let Some(disp) = &part.content_disposition() {
                            info!("Part {}: Content-Disposition: {}", i+1, disp);
                        }
                        
                        // Show beginning of the body
                        let body_preview = if part.body().len() > 30 {
                            format!("{}...", String::from_utf8_lossy(&part.body()[..30]))
                        } else {
                            String::from_utf8_lossy(&part.body()).to_string()
                        };
                        
                        info!("Part {}: Body preview: {}", i+1, body_preview);
                    }
                    
                    // Example of processing specific content types
                    process_multipart_parts(&multipart);
                }
            }
        }
    }
}

/// Helper function to process different parts of a multipart body based on content type
fn process_multipart_parts(multipart: &MultipartBody) {
    for part in multipart.parts() {
        let content_type = part.content_type();
        
        if content_type.media_type() == "application/sdp" {
            info!("Processing SDP content:");
            
            // In a real application, you would parse the SDP properly
            // For this example, we'll just look for specific lines
            let sdp_str = String::from_utf8_lossy(&part.body());
            
            // Extract media lines
            for line in sdp_str.lines() {
                if line.starts_with("m=") {
                    info!("  Media line: {}", line);
                }
            }
        } else if content_type.media_type() == "application/pidf+xml" {
            info!("Processing PIDF presence document:");
            
            // In a real application, you would use an XML parser
            // For this example, we'll just check if certain tags exist
            let xml_str = String::from_utf8_lossy(&part.body());
            
            if xml_str.contains("<basic>open</basic>") {
                info!("  Presence status: open");
            } else if xml_str.contains("<basic>closed</basic>") {
                info!("  Presence status: closed");
            }
        }
    }
}

/// Example 3: Real-world example - REFER with Replaces
fn refer_with_replaces() {
    info!("Example 3: Real-world example - REFER with Replaces");
    
    // Create a REFER request with a Refer-To header containing a Replaces parameter
    // This is used for attended transfer scenarios
    
    // Existing dialog information
    let existing_call_id = "existing-call-id-12345";
    let to_tag = "to-tag-67890";
    let from_tag = "from-tag-abcde";
    
    // Create the Refer-To header with Replaces parameter
    let replaces = Replaces::new(existing_call_id)
        .with_to_tag(to_tag)
        .with_from_tag(from_tag);
    
    // The Refer-To header points to the target with a Replaces parameter
    let refer_to_uri = Uri::parse("sip:charlie@example.com").unwrap()
        .with_parameter("Replaces", &replaces.to_string().replace("Replaces: ", ""));
    
    // Create the SIP REFER request
    let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    let call_id = format!("{}@192.168.1.100", Uuid::new_v4().to_string().split('-').next().unwrap());
    
    let refer_request = sip! {
        method: Method::Refer,
        uri: "sip:bob@example.com",
        headers: {
            Via: format!("SIP/2.0/UDP 192.168.1.100:5060;branch={}", branch),
            From: "Alice <sip:alice@example.com>;tag=1928301774",
            To: "Bob <sip:bob@example.com>;tag=456xyz",
            CallId: call_id,
            CSeq: "1 REFER",
            Contact: "<sip:alice@192.168.1.100:5060>",
            ReferTo: format!("<{}>", refer_to_uri),
            Referred_By: "<sip:alice@example.com>",
            ContentLength: 0
        }
    };
    
    info!("Created REFER request with Replaces parameter:");
    debug!("{}", std::str::from_utf8(&refer_request.to_bytes()).unwrap());
    
    // Alternative approach using multipart/mixed with application/sdp and resource-lists+xml
    // This is used in some implementations for multi-party conferencing
    
    // Create an XML resource list
    let resource_list = concat!(
        "<?xml version='1.0' encoding='UTF-8'?>\r\n",
        "<resource-lists xmlns='urn:ietf:params:xml:ns:resource-lists'>\r\n",
        "  <list>\r\n",
        "    <entry uri='sip:charlie@example.com?Replaces=", existing_call_id, "%3Bto-tag%3D", to_tag, "%3Bfrom-tag%3D", from_tag, "'/>\r\n",
        "    <entry uri='sip:david@example.com'/>\r\n",
        "  </list>\r\n",
        "</resource-lists>\r\n"
    );
    
    // Create a multipart MIME body with SDP and resource list
    let boundary = "boundary5678";
    let multipart_body = MultipartBody::new(boundary)
        .add_part(
            ContentType::new("application/resource-lists+xml"),
            Some(ContentDisposition::new("recipient-list")),
            Bytes::from(resource_list)
        );
    
    // Create a SIP REFER request with multipart body
    let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    let call_id = format!("{}@192.168.1.100", Uuid::new_v4().to_string().split('-').next().unwrap());
    
    let multipart_refer = sip! {
        method: Method::Refer,
        uri: "sip:conference@example.com",
        headers: {
            Via: format!("SIP/2.0/UDP 192.168.1.100:5060;branch={}", branch),
            From: "Alice <sip:alice@example.com>;tag=1928301774",
            To: "Conference <sip:conference@example.com>",
            CallId: call_id,
            CSeq: "1 REFER",
            Contact: "<sip:alice@192.168.1.100:5060>",
            Referred_By: "<sip:alice@example.com>",
            ContentType: format!("multipart/mixed; boundary={}", boundary),
            ContentLength: multipart_body.to_bytes().len()
        },
        body: multipart_body.to_bytes()
    };
    
    info!("Created REFER with multipart body containing resource list:");
    debug!("{}", std::str::from_utf8(&multipart_refer.to_bytes()).unwrap());
}

/// Example 4: Real-world example - Session descriptions with alternative formats
fn alternative_session_descriptions() {
    info!("Example 4: Real-world example - Session descriptions with alternative formats");
    
    // Create an SDP body for a standard audio/video call
    let sdp_body = concat!(
        "v=0\r\n",
        "o=alice 2890844526 2890844526 IN IP4 192.168.1.100\r\n",
        "s=SIP Call with Multiple Descriptions\r\n",
        "c=IN IP4 192.168.1.100\r\n",
        "t=0 0\r\n",
        "m=audio 49172 RTP/AVP 0 8\r\n",
        "a=rtpmap:0 PCMU/8000\r\n",
        "a=rtpmap:8 PCMA/8000\r\n",
        "m=video 49174 RTP/AVP 96\r\n",
        "a=rtpmap:96 H264/90000\r\n"
    );
    
    // Create a JSON-based session description (hypothetical format for WebRTC integration)
    let json_body = concat!(
        "{\r\n",
        "  \"version\": 0,\r\n",
        "  \"origin\": {\r\n",
        "    \"username\": \"alice\",\r\n",
        "    \"sessionId\": 2890844526,\r\n",
        "    \"sessionVersion\": 2890844526,\r\n",
        "    \"netType\": \"IN\",\r\n",
        "    \"addrType\": \"IP4\",\r\n",
        "    \"address\": \"192.168.1.100\"\r\n",
        "  },\r\n",
        "  \"media\": [\r\n",
        "    {\r\n",
        "      \"type\": \"audio\",\r\n",
        "      \"port\": 49172,\r\n",
        "      \"protocol\": \"RTP/AVP\",\r\n",
        "      \"codecs\": [\"PCMU/8000\", \"PCMA/8000\"]\r\n",
        "    },\r\n",
        "    {\r\n",
        "      \"type\": \"video\",\r\n",
        "      \"port\": 49174,\r\n",
        "      \"protocol\": \"RTP/AVP\",\r\n",
        "      \"codecs\": [\"H264/90000\"]\r\n",
        "    }\r\n",
        "  ]\r\n",
        "}\r\n"
    );
    
    // Create a multipart/alternative body with two session description formats
    let boundary = "boundary_alt_123";
    let multipart_body = MultipartBody::new(boundary)
        .alternative() // Mark as multipart/alternative
        .add_part(
            ContentType::new("application/sdp"),
            None,
            Bytes::from(sdp_body)
        )
        .add_part(
            ContentType::new("application/json"),
            None,
            Bytes::from(json_body)
        );
    
    // Create a SIP INVITE with the multipart/alternative body
    let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    let call_id = format!("{}@192.168.1.100", Uuid::new_v4().to_string().split('-').next().unwrap());
    
    let multipart_invite = sip! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        headers: {
            Via: format!("SIP/2.0/UDP 192.168.1.100:5060;branch={}", branch),
            From: "Alice <sip:alice@example.com>;tag=1928301774",
            To: "Bob <sip:bob@example.com>",
            CallId: call_id,
            CSeq: "1 INVITE",
            Contact: "<sip:alice@192.168.1.100:5060>",
            ContentType: format!("multipart/alternative; boundary={}", boundary),
            ContentLength: multipart_body.to_bytes().len()
        },
        body: multipart_body.to_bytes()
    };
    
    info!("Created INVITE with multipart/alternative body:");
    debug!("{}", std::str::from_utf8(&multipart_invite.to_bytes()).unwrap());
    
    // Parse the multipart/alternative body
    if let Some(body) = multipart_invite.body() {
        if let Some(content_type) = multipart_invite.typed_header::<ContentType>() {
            if content_type.is_multipart() {
                if let Some(boundary) = content_type.parameter("boundary") {
                    let multipart = MultipartBody::parse(body, boundary).expect("Failed to parse multipart body");
                    
                    info!("Parsed multipart/alternative with {} parts", multipart.parts().len());
                    
                    // Show each alternative
                    for (i, part) in multipart.parts().iter().enumerate() {
                        info!("Alternative {}: Content-Type: {}", i+1, part.content_type());
                        
                        // Process based on content type
                        if part.content_type().media_type() == "application/sdp" {
                            info!("Found SDP alternative (preferred for traditional SIP)");
                        } else if part.content_type().media_type() == "application/json" {
                            info!("Found JSON alternative (preferred for WebRTC clients)");
                        }
                    }
                    
                    // In a real application, you would select the most appropriate format
                    // based on the capabilities of the client
                    info!("A SIP client would typically select the application/sdp alternative");
                    info!("A WebRTC client might prefer the application/json alternative");
                }
            }
        }
    }
} 