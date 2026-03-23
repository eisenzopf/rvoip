    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::builder::headers::ContentTypeBuilderExt;
    use crate::types::Method;
    use crate::sdp::SdpBuilder;
    
    #[test]
    fn test_basic_builder() {
        let multipart = MultipartBodyBuilder::new()
            .add_text_part("Text content")
            .add_html_part("<html><body><p>HTML content</p></body></html>")
            .build();
            
        assert_eq!(multipart.parts.len(), 2);
        assert!(multipart.boundary.starts_with("boundary-"));
        
        // Check first part (text)
        assert_eq!(multipart.parts[0].content_type().unwrap(), "text/plain");
        assert_eq!(
            std::str::from_utf8(&multipart.parts[0].raw_content).unwrap(),
            "Text content"
        );
        
        // Check second part (HTML)
        assert_eq!(multipart.parts[1].content_type().unwrap(), "text/html");
        assert_eq!(
            std::str::from_utf8(&multipart.parts[1].raw_content).unwrap(),
            "<html><body><p>HTML content</p></body></html>"
        );
    }
    
    #[test]
    fn test_custom_boundary() {
        let multipart = MultipartBodyBuilder::new()
            .boundary("custom-test-boundary")
            .add_text_part("Content")
            .build();
            
        assert_eq!(multipart.boundary, "custom-test-boundary");
    }
    
    #[test]
    fn test_preamble_epilogue() {
        let multipart = MultipartBodyBuilder::new()
            .preamble("This is the preamble")
            .epilogue("This is the epilogue")
            .add_text_part("Content")
            .build();
            
        assert_eq!(
            std::str::from_utf8(&multipart.preamble.unwrap()).unwrap(),
            "This is the preamble"
        );
        assert_eq!(
            std::str::from_utf8(&multipart.epilogue.unwrap()).unwrap(),
            "This is the epilogue"
        );
    }
    
    #[test]
    fn test_json_part() {
        let json = r#"{"name":"Alice","age":30}"#;
        let multipart = MultipartBodyBuilder::new()
            .add_json_part(json)
            .build();
            
        assert_eq!(multipart.parts.len(), 1);
        assert_eq!(multipart.parts[0].content_type().unwrap(), "application/json");
        assert_eq!(
            std::str::from_utf8(&multipart.parts[0].raw_content).unwrap(),
            json
        );
    }
    
    #[test]
    fn test_xml_part() {
        let xml = r#"<?xml version="1.0"?><root><node>value</node></root>"#;
        let multipart = MultipartBodyBuilder::new()
            .add_xml_part(xml)
            .build();
            
        assert_eq!(multipart.parts.len(), 1);
        assert_eq!(multipart.parts[0].content_type().unwrap(), "application/xml");
        assert_eq!(
            std::str::from_utf8(&multipart.parts[0].raw_content).unwrap(),
            xml
        );
    }
    
    #[test]
    fn test_sdp_part() {
        let sdp = SdpBuilder::new("Test Session")
            .origin("test", "123456", "789012", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(49170, "RTP/AVP")
                .formats(&["0", "8"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000")
                .done()
            .build()
            .unwrap();
            
        let multipart = MultipartBodyBuilder::new()
            .add_sdp_part(sdp.to_string())
            .build();
            
        assert_eq!(multipart.parts.len(), 1);
        assert_eq!(multipart.parts[0].content_type().unwrap(), "application/sdp");
        assert_eq!(
            std::str::from_utf8(&multipart.parts[0].raw_content).unwrap(),
            sdp.to_string()
        );
    }
    
    #[test]
    fn test_image_part() {
        let image_data = Bytes::from_static(&[0xFF, 0xD8, 0xFF, 0xE0]); // Fake JPEG header
        let multipart = MultipartBodyBuilder::new()
            .add_image_part("image/jpeg", image_data.clone(), Some("img1@example.com"))
            .build();
            
        assert_eq!(multipart.parts.len(), 1);
        assert_eq!(multipart.parts[0].content_type().unwrap(), "image/jpeg");
        
        // Check Content-ID header
        let content_id_headers = multipart.parts[0].headers.iter()
            .filter(|h| h.name == HeaderName::Other("Content-ID".to_string()))
            .collect::<Vec<_>>();
            
        assert_eq!(content_id_headers.len(), 1);
        assert!(content_id_headers[0].to_string().contains("<img1@example.com>"));
            
        // Check image data
        assert_eq!(multipart.parts[0].raw_content, image_data);
    }
    
    #[test]
    fn test_to_string() {
        let multipart = MultipartBodyBuilder::new()
            .boundary("simple-boundary")
            .add_text_part("Text content")
            .build();
            
        let body_string = multipart.to_string();
        
        // Check basic structure
        assert!(body_string.contains("--simple-boundary\r\n"));
        assert!(body_string.contains("Content-Type: text/plain\r\n"));
        assert!(body_string.contains("\r\nText content\r\n"));
        assert!(body_string.contains("--simple-boundary--\r\n"));
    }
    
    #[test]
    fn test_sip_message_integration() {
        let multipart = MultipartBodyBuilder::new()
            .boundary("test-boundary")
            .add_text_part("Plain text")
            .add_html_part("<html><body>HTML</body></html>")
            .build();
            
        let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
            .content_type(format!("multipart/alternative; boundary={}", multipart.boundary).as_str())
            .body(multipart.to_string())
            .build();
            
        let headers = message.all_headers();
        let content_type_headers = headers.iter()
            .filter(|h| match h {
                crate::types::TypedHeader::ContentType(_) => true,
                _ => false,
            })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert!(content_type_headers[0].to_string().contains("multipart/alternative"));
        assert!(content_type_headers[0].to_string().contains("boundary=\"test-boundary\""));
        
        // Check body
        let body_str = message.body();
        assert!(std::str::from_utf8(body_str).unwrap().contains("--test-boundary"));
        assert!(std::str::from_utf8(body_str).unwrap().contains("Content-Type: text/plain"));
        assert!(std::str::from_utf8(body_str).unwrap().contains("Plain text"));
        assert!(std::str::from_utf8(body_str).unwrap().contains("Content-Type: text/html"));
        assert!(std::str::from_utf8(body_str).unwrap().contains("<html><body>HTML</body></html>"));
    }
    
    #[test]
    fn test_multipart_part_builder() {
        // Basic part with content-type and body
        let part = MultipartPartBuilder::new()
            .content_type("text/plain")
            .body("This is text content")
            .build();
            
        assert_eq!(part.content_type().unwrap(), "text/plain");
        assert_eq!(
            std::str::from_utf8(&part.raw_content).unwrap(),
            "This is text content"
        );
        
        // Part with content-id
        let part = MultipartPartBuilder::new()
            .content_type("text/plain")
            .content_id("<text123@example.com>")
            .body("This is text content with ID")
            .build();
            
        assert_eq!(part.content_type().unwrap(), "text/plain");
        assert!(part.headers.iter().any(|h| 
            h.name == HeaderName::Other("Content-ID".to_string()) && 
            h.value.as_text() == Some("<text123@example.com>")
        ));
        
        // Part with content-disposition
        let part = MultipartPartBuilder::new()
            .content_type("application/sdp")
            .content_disposition("session")
            .body("v=0\r\no=- 1234 1234 IN IP4 127.0.0.1\r\ns=Test\r\n")
            .build();
            
        assert_eq!(part.content_type().unwrap(), "application/sdp");
        assert!(part.headers.iter().any(|h| 
            h.name == HeaderName::ContentDisposition && 
            h.value.as_text() == Some("session")
        ));
        
        // Part with content-transfer-encoding
        let part = MultipartPartBuilder::new()
            .content_type("image/png")
            .content_transfer_encoding("base64")
            .body("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
            .build();
            
        assert_eq!(part.content_type().unwrap(), "image/png");
        assert!(part.headers.iter().any(|h| 
            h.name == HeaderName::Other("Content-Transfer-Encoding".to_string()) && 
            h.value.as_text() == Some("base64")
        ));
    }
    
    #[test]
    fn test_multipart_builder_mixed() {
        let multipart = MultipartBuilder::mixed()
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/plain")
                    .body("Plain text part")
                    .build()
            )
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("application/json")
                    .body(r#"{"key":"value"}"#)
                    .build()
            )
            .build();
            
        // Check content type
        let content_type = multipart.content_type();
        assert!(content_type.starts_with("multipart/mixed; boundary="));
        
        // Get boundary, handling both quoted and unquoted formats
        let boundary_part = content_type.split("boundary=").nth(1).unwrap_or("");
        let boundary = boundary_part.trim_matches('"');
        
        // Check body
        let body = multipart.body();
        assert!(body.contains("Content-Type: text/plain"));
        assert!(body.contains("Plain text part"));
        assert!(body.contains("Content-Type: application/json"));
        assert!(body.contains(r#"{"key":"value"}"#));
        
        // Body should contain the boundary
        assert!(body.contains(&format!("--{}", boundary)));
        
        // Body should end with boundary--
        assert!(body.contains(&format!("--{}--", boundary)));
    }
    
    #[test]
    fn test_multipart_builder_alternative() {
        let multipart = MultipartBuilder::alternative()
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/plain")
                    .body("This is plain text")
                    .build()
            )
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/html")
                    .body("<html><body><p>This is HTML</p></body></html>")
                    .build()
            )
            .build();
            
        let content_type = multipart.content_type();
        assert!(content_type.starts_with("multipart/alternative; boundary="));
        
        let body = multipart.body();
        assert!(body.contains("Content-Type: text/plain"));
        assert!(body.contains("This is plain text"));
        assert!(body.contains("Content-Type: text/html"));
        assert!(body.contains("<html><body><p>This is HTML</p></body></html>"));
    }
    
    #[test]
    fn test_multipart_builder_related() {
        let multipart = MultipartBuilder::related()
            .type_parameter("text/html")
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/html")
                    .content_id("<main@example.com>")
                    .body("<html><body><img src=\"cid:image@example.com\"></body></html>")
                    .build()
            )
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("image/png")
                    .content_id("<image@example.com>")
                    .content_transfer_encoding("base64")
                    .content_disposition("inline")
                    .body("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
                    .build()
            )
            .build();
            
        // Check content type - should contain both multipart/related and text/html
        let content_type = multipart.content_type();
        assert!(content_type.contains("multipart/related"));
        assert!(content_type.contains("text/html"));
        
        let body = multipart.body();
        assert!(body.contains("Content-Type: text/html"));
        assert!(body.contains("Content-ID: <main@example.com>"));
        assert!(body.contains("<img src=\"cid:image@example.com\">"));
        assert!(body.contains("Content-Type: image/png"));
        assert!(body.contains("Content-ID: <image@example.com>"));
        assert!(body.contains("Content-Transfer-Encoding: base64"));
    }
    
    #[test]
    fn test_multipart_builder_preamble_epilogue() {
        let preamble = "This is a multipart message in MIME format.";
        let epilogue = "This is the epilogue. It is also ignored.";
        
        let multipart = MultipartBuilder::mixed()
            .preamble(preamble)
            .epilogue(epilogue)
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/plain")
                    .body("Test content")
                    .build()
            )
            .build();
            
        let body = multipart.body();
        
        // Preamble should be before the first boundary
        let content_type_str = multipart.content_type();
        let boundary_part = content_type_str.split("boundary=").nth(1).unwrap_or("");
        let boundary = boundary_part.trim_matches('"');
        let first_boundary_pos = body.find(&format!("--{}", boundary)).unwrap_or(0);
        let preamble_in_body = &body[0..first_boundary_pos];
        assert_eq!(preamble_in_body.trim(), preamble);
        
        // Epilogue should be after the last boundary
        let last_boundary_pos = body.rfind(&format!("--{}--", boundary)).map(|pos| pos + boundary.len() + 4).unwrap_or(body.len()); // +4 for "--" and "--"
        let epilogue_in_body = &body[last_boundary_pos..].trim().to_string();
        assert_eq!(epilogue_in_body, epilogue);
    }
    
    #[test]
    fn test_multipart_builder_custom_boundary() {
        let custom_boundary = "a-custom-boundary-string";
        
        let multipart = MultipartBuilder::mixed()
            .boundary(custom_boundary.to_string())
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/plain")
                    .body("Test with custom boundary")
                    .build()
            )
            .build();
            
        // Check content type has our custom boundary
        let content_type = multipart.content_type();
        assert!(content_type.contains(&format!("boundary=\"{}\"", custom_boundary)));
        
        // Check body uses our custom boundary
        let body = multipart.body();
        assert!(body.contains(&format!("--{}", custom_boundary)));
        assert!(body.contains(&format!("--{}--", custom_boundary)));
    }
    
    #[test]
    fn test_integration_with_sip_message() {
        let multipart = MultipartBuilder::mixed()
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/plain")
                    .body("Hello SIP world!")
                    .build()
            )
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("application/json")
                    .body(r#"{"greeting":"Hello JSON world!"}"#)
                    .build()
            )
            .build();
            
        let request = SimpleRequestBuilder::new(Method::Message, "sip:test@example.com").unwrap()
            .content_type(&multipart.content_type())
            .body(multipart.body())
            .build();
            
        // Check the request has proper Content-Type header
        let content_type_header = request.all_headers().iter()
            .find(|h| match h {
                TypedHeader::ContentType(_) => true,
                _ => false,
            })
            .unwrap();
        
        // Check the body is set correctly
        let header_str = content_type_header.to_string();
        assert!(header_str.contains("multipart/mixed") && header_str.contains("boundary="));
        
        // Check the body is set correctly
        assert_eq!(request.body(), multipart.body().as_bytes());
    }
    
    #[test]
    fn test_multipart_with_sdp() {
        let sdp = SdpBuilder::new("Test SDP")
            .origin("test", "123456", "789012", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(49170, "RTP/AVP")
                .formats(&["0", "8"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000")
                .done()
            .build()
            .unwrap();
            
        let multipart = MultipartBuilder::mixed()
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("application/sdp")
                    .content_disposition("session")
                    .body(sdp.to_string())
                    .build()
            )
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("application/xml")
                    .body("<metadata><session-id>12345</session-id></metadata>")
                    .build()
            )
            .build();
            
        // Check content type
        assert!(multipart.content_type().starts_with("multipart/mixed; boundary="));
        
        // Check body contains SDP
        let body = multipart.body();
        assert!(body.contains("v=0"));
        assert!(body.contains("m=audio 49170 RTP/AVP 0 8"));
        assert!(body.contains("a=rtpmap:0 PCMU/8000"));
        
        // Check body contains XML
        assert!(body.contains("<metadata><session-id>12345</session-id></metadata>"));
    }
