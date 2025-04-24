use rvoip_sip_core::parser::headers::contact::parse_contact;
use rvoip_sip_core::types::contact::ContactValue;
use rvoip_sip_core::types::uri::Scheme;
use rvoip_sip_core::types::param::Param;
use ordered_float::NotNan;

fn main() {
    println!("Testing Contact header parser");

    // Test star
    let input1 = b" * ";
    match parse_contact(input1) {
        Ok((rem, val)) => {
            println!("Star test: Success");
            assert!(rem.is_empty());
            assert!(matches!(val, ContactValue::Star));
        },
        Err(e) => {
            println!("Star test: Failed - {:?}", e);
        }
    }

    // Test with single addr-spec
    let input2 = b"<sip:user@host.com>";
    match parse_contact(input2) {
        Ok((rem, val)) => {
            println!("Single addr-spec test: Success");
            assert!(rem.is_empty());
            if let ContactValue::Params(params) = val {
                assert_eq!(params.len(), 1);
                assert!(params[0].address.display_name.is_none());
                assert_eq!(params[0].address.uri.scheme, Scheme::Sip);
                assert!(params[0].address.params.is_empty());
            } else {
                println!("Single addr-spec test: Unexpected variant");
            }
        },
        Err(e) => {
            println!("Single addr-spec test: Failed - {:?}", e);
        }
    }

    // Test with name-addr and parameters
    let input3 = b"\"Mr. Watson\" <sip:watson@bell.com>;q=0.7;expires=3600";
    match parse_contact(input3) {
        Ok((rem, val)) => {
            println!("Name-addr with params test: Success");
            assert!(rem.is_empty());
            if let ContactValue::Params(params) = val {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].address.display_name, Some("Mr. Watson".to_string()));
                assert_eq!(params[0].address.uri.scheme, Scheme::Sip);
                assert_eq!(params[0].address.params.len(), 2);
                assert!(params[0].address.params.contains(&Param::Q(NotNan::new(0.7).unwrap())));
                assert!(params[0].address.params.contains(&Param::Expires(3600)));
            } else {
                println!("Name-addr with params test: Unexpected variant");
            }
        },
        Err(e) => {
            println!("Name-addr with params test: Failed - {:?}", e);
        }
    }

    // Test with multiple contacts
    let input4 = b"<sip:A@atlanta.com>, \"Bob\" <sip:bob@biloxi.com>;tag=123";
    match parse_contact(input4) {
        Ok((rem, val)) => {
            println!("Multiple contacts test: Success");
            assert!(rem.is_empty());
            if let ContactValue::Params(params) = val {
                assert_eq!(params.len(), 2);
                // First contact
                assert!(params[0].address.display_name.is_none());
                assert!(params[0].address.params.is_empty());
                // Second contact
                assert_eq!(params[1].address.display_name, Some("Bob".to_string()));
                assert_eq!(params[1].address.params.len(), 1);
                
                // Debug information
                println!("Second contact params: {:?}", params[1].address.params);
                
                // Check if there's a tag parameter
                let has_tag = params[1].address.params.iter().any(|p| {
                    if let Param::Tag(ref s) = p {
                        println!("Found tag param: {}", s);
                        s == "123"
                    } else {
                        false
                    }
                });
                
                if !has_tag {
                    println!("Tag parameter '123' not found");
                    for (i, param) in params[1].address.params.iter().enumerate() {
                        println!("Param[{}]: {:?}", i, param);
                    }
                }
                
                assert!(has_tag, "Tag parameter '123' should exist");
            } else {
                println!("Multiple contacts test: Unexpected variant");
            }
        },
        Err(e) => {
            println!("Multiple contacts test: Failed - {:?}", e);
        }
    }

    // Test addr-spec without angle brackets
    let input5 = b"sip:user@example.com";
    match parse_contact(input5) {
        Ok((rem, val)) => {
            println!("Addr-spec without brackets test: Success");
            assert!(rem.is_empty());
            if let ContactValue::Params(params) = val {
                assert_eq!(params.len(), 1);
                assert!(params[0].address.display_name.is_none());
                assert_eq!(params[0].address.uri.scheme, Scheme::Sip);
                assert_eq!(params[0].address.uri.host.to_string(), "example.com");
                assert_eq!(params[0].address.uri.user, Some("user".to_string()));
            } else {
                println!("Addr-spec without brackets test: Unexpected variant");
            }
        },
        Err(e) => {
            println!("Addr-spec without brackets test: Failed - {:?}", e);
        }
    }

    println!("All tests completed");
} 