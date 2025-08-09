# CORRECTED Client-Server API Implementation Plan

## Assessment Results: Leveraging Existing SIP Helpers

After examining the existing codebase, I found that **we already have excellent SIP message creation helpers** and should leverage them instead of manual message construction.

## ✅ **Available Infrastructure:**

### **1. sip-core Provides Request Builders**
```rust
// Available convenience constructors:
SimpleRequestBuilder::register("sip:registrar.example.com")
SimpleRequestBuilder::options("sip:server.example.com") 
SimpleRequestBuilder::invite("sip:bob@example.com")
SimpleRequestBuilder::bye("sip:bob@example.com")
SimpleRequestBuilder::ack("sip:bob@example.com") 
SimpleRequestBuilder::cancel("sip:bob@example.com")

// Generic constructor for MESSAGE, SUBSCRIBE:
SimpleRequestBuilder::new(Method::Message, "sip:user@example.com")
SimpleRequestBuilder::new(Method::Subscribe, "sip:presence@example.com")
```

### **2. dialog-core Provides Integration**
- **Non-dialog requests**: `send_non_dialog_request(request, destination, timeout)`
- **Dialog requests**: `send_request_in_dialog(dialog_id, method, body)`
- **Response building**: `build_response(transaction_id, status_code, body)`

### **3. transaction-core Provides Advanced Builders**
- **InviteBuilder**: Complex INVITE with SDP
- **dialog_quick**: `reinvite_for_dialog()` for re-INVITEs

## 🔧 **CORRECTED Implementation Strategy:**

### **Phase 1: Complete SipClient Implementation (Leveraging Existing Helpers)**

#### **1.1: REGISTER Implementation**
```rust
impl SipClient for SessionCoordinator {
    async fn register(
        &self,
        registrar_uri: &str,
        from_uri: &str, 
        contact_uri: &str,
        expiry_seconds: u32,
        auth_info: Option<AuthInfo>,
    ) -> Result<RegistrationResult, SessionError> {
        use rvoip_sip_core::builder::SimpleRequestBuilder;
        use rvoip_sip_core::types::{TypedHeader, expires::Expires};
        
        // Use existing sip-core builder
        let mut request_builder = SimpleRequestBuilder::register(registrar_uri)?
            .from("User", from_uri, Some(&format!("reg-{}", uuid::Uuid::new_v4())))
            .to("User", from_uri, None) // To matches From for registration  
            .call_id(&format!("reg-{}", uuid::Uuid::new_v4()))
            .cseq(1)
            .via(&self.local_address.to_string(), "UDP", Some(&self.generate_branch()))
            .max_forwards(70)
            .contact(contact_uri, None)
            .header(TypedHeader::Expires(Expires::new(expiry_seconds)));
            
        // Add authentication if provided
        if let Some(auth) = auth_info {
            request_builder = request_builder.header(TypedHeader::Authorization(auth));
        }
        
        let request = request_builder.build()?;
        
        // Use dialog-core's non-dialog transaction handling
        let response = self.unified_dialog.send_non_dialog_request(
            request,
            self.resolve_destination(registrar_uri).await?,
            Duration::from_secs(32)
        ).await?;
        
        Ok(RegistrationResult::from_response(response)?)
    }
}
```

#### **1.2: OPTIONS Implementation**
```rust
async fn send_options(
    &self,
    target_uri: &str,
    supported_methods: Vec<Method>,
) -> Result<OptionsResult, SessionError> {
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::{TypedHeader, allow::Allow, supported::Supported};
    
    // Build Allow header
    let mut allow = Allow::new();
    for method in supported_methods {
        allow.add_method(method);
    }
    
    // Build Supported extensions header
    let supported = Supported::new(vec![
        "100rel".to_string(),
        "timer".to_string(), 
        "replaces".to_string()
    ]);
    
    // Use existing sip-core builder
    let request = SimpleRequestBuilder::options(target_uri)?
        .from("User", &self.local_uri, Some(&self.generate_tag()))
        .to("Server", target_uri, None)
        .call_id(&format!("opt-{}", uuid::Uuid::new_v4()))
        .cseq(1)
        .via(&self.local_address.to_string(), "UDP", Some(&self.generate_branch()))
        .max_forwards(70)
        .contact(&self.local_contact, None)
        .header(TypedHeader::Allow(allow))
        .header(TypedHeader::Supported(supported))
        .build()?;
        
    // Use dialog-core's non-dialog handling
    let response = self.unified_dialog.send_non_dialog_request(
        request,
        self.resolve_destination(target_uri).await?,
        Duration::from_secs(32)
    ).await?;
    
    Ok(OptionsResult::from_response(response)?)
}
```

#### **1.3: MESSAGE Implementation**
```rust
async fn send_message(
    &self,
    to_uri: &str,
    content: &str,
    content_type: Option<&str>,
) -> Result<MessageResult, SessionError> {
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::{Method, TypedHeader};
    
    // Use generic builder for MESSAGE (no convenience constructor yet)
    let mut request_builder = SimpleRequestBuilder::new(Method::Message, to_uri)?
        .from("User", &self.local_uri, Some(&self.generate_tag()))
        .to("User", to_uri, None)
        .call_id(&format!("msg-{}", uuid::Uuid::new_v4()))
        .cseq(1)
        .via(&self.local_address.to_string(), "UDP", Some(&self.generate_branch()))
        .max_forwards(70)
        .contact(&self.local_contact, None)
        .body(content);
        
    // Set content type
    let ct = content_type.unwrap_or("text/plain");
    request_builder = request_builder.content_type(ct);
    
    let request = request_builder.build()?;
    
    // Use dialog-core's non-dialog handling
    let response = self.unified_dialog.send_non_dialog_request(
        request,
        self.resolve_destination(to_uri).await?,
        Duration::from_secs(32)
    ).await?;
    
    Ok(MessageResult::from_response(response)?)
}
```

#### **1.4: SUBSCRIBE Implementation**
```rust
async fn subscribe(
    &self,
    resource_uri: &str,
    event_package: &str,
    expiry_seconds: u32,
) -> Result<SubscriptionResult, SessionError> {
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::{Method, TypedHeader, event::Event, expires::Expires};
    
    // Use generic builder for SUBSCRIBE
    let request = SimpleRequestBuilder::new(Method::Subscribe, resource_uri)?
        .from("User", &self.local_uri, Some(&self.generate_tag()))
        .to("Resource", resource_uri, None)
        .call_id(&format!("sub-{}", uuid::Uuid::new_v4()))
        .cseq(1)
        .via(&self.local_address.to_string(), "UDP", Some(&self.generate_branch()))
        .max_forwards(70)
        .contact(&self.local_contact, None)
        .header(TypedHeader::Event(Event::new(event_package)))
        .header(TypedHeader::Expires(Expires::new(expiry_seconds)))
        .build()?;
        
    // Use dialog-core's non-dialog handling  
    let response = self.unified_dialog.send_non_dialog_request(
        request,
        self.resolve_destination(resource_uri).await?,
        Duration::from_secs(32)
    ).await?;
    
    Ok(SubscriptionResult::from_response(response)?)
}
```

### **Phase 2: Complete ServerSessionManager Implementation**

#### **2.1: Multi-Session Bridge Operations**
```rust
impl ServerSessionManager for SessionCoordinator {
    async fn create_bridge(&self, bridge_config: BridgeConfig) -> Result<BridgeId, SessionError> {
        // Use existing bridge functionality from session-core/bridge
        let bridge = self.bridge_manager.create_bridge(bridge_config).await?;
        Ok(bridge.id())
    }
    
    async fn add_session_to_bridge(
        &self,
        bridge_id: &BridgeId,
        session_id: &SessionId
    ) -> Result<(), SessionError> {
        // Use existing bridge operations
        self.bridge_manager.add_session(bridge_id, session_id).await
    }
}
```

### **Phase 3: Integration Updates**

#### **3.1: client-core Integration**
```rust
// In client-core/src/client/manager.rs
impl ClientManager {
    async fn register_with_server(&self) -> Result<(), ClientError> {
        // Use SipClient trait methods instead of generic SessionControl
        self.session_coordinator.register(
            &self.config.registrar_uri,
            &self.config.local_uri,
            &self.config.contact_uri,
            3600,
            self.auth_info.clone()
        ).await?;
        
        Ok(())
    }
    
    async fn send_presence_subscription(&self, target: &str) -> Result<(), ClientError> {
        // Use SipClient trait for presence
        self.session_coordinator.subscribe(
            target,
            "presence",
            1800
        ).await?;
        
        Ok(())
    }
}
```

#### **3.2: call-engine Integration**
```rust
// In call-engine/src/orchestrator/session_manager.rs
impl CallOrchestrator {
    async fn create_conference_bridge(&self, participants: Vec<SessionId>) -> Result<BridgeId, CallError> {
        // Use ServerSessionManager trait methods
        let bridge_config = BridgeConfig::new()
            .with_type(BridgeType::Conference)
            .with_mixing_enabled(true);
            
        let bridge_id = self.session_coordinator.create_bridge(bridge_config).await?;
        
        // Add all participants
        for participant in participants {
            self.session_coordinator.add_session_to_bridge(&bridge_id, &participant).await?;
        }
        
        Ok(bridge_id)
    }
}
```

## 📋 **Implementation Tasks (Updated):**

### **Priority 1: Leverage Existing Helpers**
- [x] ~~Assess existing sip-core and dialog-core helpers~~
- [ ] Add convenience constructors for MESSAGE and SUBSCRIBE to sip-core (optional)
- [ ] Implement SipClient trait using existing builders
- [ ] Implement ServerSessionManager trait using existing bridge functionality

### **Priority 2: Integration**
- [ ] Update client-core to use SipClient methods
- [ ] Update call-engine to use ServerSessionManager methods
- [ ] Create integration tests

### **Priority 3: Documentation**
- [ ] Update API documentation to show proper builder usage
- [ ] Create examples demonstrating the separation

## 🎯 **Key Benefits of This Approach:**

1. **Reuse Existing Infrastructure**: Leverages well-tested sip-core builders
2. **Maintain RFC Compliance**: Existing builders handle SIP standards correctly
3. **Reduce Code Duplication**: No manual message construction 
4. **Type Safety**: Existing builders provide compile-time validation
5. **Future Compatibility**: Easy to extend when new SIP methods are added

This corrected approach builds on the solid foundation already present in the codebase rather than reinventing message construction.
