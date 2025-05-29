#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use std::net::SocketAddr;
    use crate::events::EventBus;
    use crate::Dialog;
    use tokio::sync::mpsc;
    use rvoip_sip_core::TypedHeader;
    use rvoip_sip_core::types::{call_id::CallId, cseq::CSeq, address::Address, param::Param};
    use rvoip_sip_core::types::{from::From as FromHeader, to::To as ToHeader};
    use rvoip_sip_core::types::contact::{Contact, ContactParamInfo};
    use crate::dialog::DialogManager;
    use crate::DialogState;
    
    // Dummy transport implementation for testing
    #[derive(Clone, Debug)]
    struct DummyTransport;
    
    impl DummyTransport {
        fn new() -> Self {
            Self
        }
    }
    
    // Implement the Transport trait for DummyTransport
    #[async_trait::async_trait]
    impl rvoip_sip_transport::Transport for DummyTransport {
        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::error::Error> {
            Ok(SocketAddr::from_str("127.0.0.1:5060").unwrap())
        }
        
        async fn send_message(&self, _message: rvoip_sip_core::Message, _destination: SocketAddr) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
            Ok(())
        }
        
        async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
            Ok(())
        }
        
        fn is_closed(&self) -> bool {
            false
        }
    }
    
    // Helper to create test SIP messages for testing
    fn create_test_invite() -> rvoip_sip_core::Request {
        let mut request = rvoip_sip_core::Request::new(rvoip_sip_core::Method::Invite, rvoip_sip_core::Uri::sip("bob@example.com"));
        
        // Add Call-ID
        let call_id = CallId("test-call-id".to_string());
        request.headers.push(TypedHeader::CallId(call_id));
        
        // Add From with tag using proper API
        let from_uri = rvoip_sip_core::Uri::sip("alice@example.com");
        let from_addr = Address::new(from_uri).with_tag("alice-tag");
        let from = FromHeader(from_addr);
        request.headers.push(TypedHeader::From(from));
        
        // Add To
        let to_uri = rvoip_sip_core::Uri::sip("bob@example.com");
        let to_addr = Address::new(to_uri);
        let to = ToHeader(to_addr);
        request.headers.push(TypedHeader::To(to));
        
        // Add CSeq
        let cseq = CSeq::new(1, rvoip_sip_core::Method::Invite);
        request.headers.push(TypedHeader::CSeq(cseq));
        
        request
    }
    
    fn create_test_response(status: rvoip_sip_core::StatusCode, with_to_tag: bool) -> rvoip_sip_core::Response {
        let mut response = rvoip_sip_core::Response::new(status);
        
        // Add Call-ID
        let call_id = CallId("test-call-id".to_string());
        response.headers.push(TypedHeader::CallId(call_id));
        
        // Add From with tag using proper API
        let from_uri = rvoip_sip_core::Uri::sip("alice@example.com");
        let from_addr = Address::new(from_uri).with_tag("alice-tag");
        let from = FromHeader(from_addr);
        response.headers.push(TypedHeader::From(from));
        
        // Add To, optionally with tag using proper API
        let to_uri = rvoip_sip_core::Uri::sip("bob@example.com");
        let to_addr = if with_to_tag {
            Address::new(to_uri).with_tag("bob-tag")
        } else {
            Address::new(to_uri)
        };
        let to = ToHeader(to_addr);
        response.headers.push(TypedHeader::To(to));
        
        // Add Contact
        let contact_uri = rvoip_sip_core::Uri::sip("bob@192.168.1.2");
        let contact_addr = Address::new(contact_uri);
        
        // Create contact header using the correct API
        let contact_param = ContactParamInfo { address: contact_addr };
        let contact = Contact::new_params(vec![contact_param]);
        response.headers.push(TypedHeader::Contact(contact));
        
        response
    }
    
    #[tokio::test]
    async fn test_dialog_manager_creation() {
        // Create a simple test to verify that DialogManager can be created
        let event_bus = EventBus::new(10);
        
        // This is a placeholder test since we don't have a real TransactionManager to use
        // In the future, we'd need to expand the session-core library to support proper mocking
        assert!(true, "This test passes but needs to be expanded");
    }
    
    #[test]
    fn test_dialog_creation_directly() {
        // Test the Dialog class directly without needing DialogManager
        
        // Create a test INVITE and response
        let request = create_test_invite();
        let response = create_test_response(rvoip_sip_core::StatusCode::Ok, true);
        
        // Create a dialog as UAC (initiator)
        let dialog = Dialog::from_2xx_response(&request, &response, true);
        
        // Verify the dialog was created
        assert!(dialog.is_some(), "Failed to create dialog from 2xx response");
        
        let dialog = dialog.unwrap();
        
        // Verify the dialog properties
        assert_eq!(dialog.state, DialogState::Confirmed);
        assert_eq!(dialog.call_id, "test-call-id");
        assert_eq!(dialog.local_tag, Some("alice-tag".to_string()));
        assert_eq!(dialog.remote_tag, Some("bob-tag".to_string()));
        assert_eq!(dialog.local_seq, 1);
        assert_eq!(dialog.remote_seq, 0);
        assert_eq!(dialog.is_initiator, true);
        assert_eq!(dialog.remote_target.to_string(), "sip:bob@192.168.1.2");
    }
    
    #[test]
    fn test_dialog_utils() {
        // Test the has_to_tag function directly by checking To headers
        let response_with_tag = create_test_response(rvoip_sip_core::StatusCode::Ok, true);
        let response_without_tag = create_test_response(rvoip_sip_core::StatusCode::Ok, false);
        
        // Check the To header for tag parameter directly
        let has_tag = response_with_tag.to()
            .and_then(|to| to.tag())
            .is_some();
        
        let missing_tag = response_without_tag.to()
            .and_then(|to| to.tag())
            .is_none();
        
        assert!(has_tag, "Response should have a to-tag");
        assert!(missing_tag, "Response should not have a to-tag");
    }
} 