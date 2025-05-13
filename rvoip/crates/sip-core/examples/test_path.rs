use rvoip_sip_core::json::{SipValue, path};
use std::collections::HashMap;

fn main() {
    // Create a SIP-like structure with Via headers
    let mut via = HashMap::new();
    via.insert("sent_protocol".to_string(), SipValue::Object({
        let mut proto = HashMap::new();
        proto.insert("transport".to_string(), SipValue::String("UDP".to_string()));
        proto
    }));
    via.insert("sent_by_host".to_string(), SipValue::Object({
        let mut host = HashMap::new();
        host.insert("Domain".to_string(), SipValue::String("example.com".to_string()));
        host
    }));
    
    // Create Via params with branch
    let mut branch_param = HashMap::new();
    branch_param.insert("Branch".to_string(), SipValue::String("z9hG4bK776asdhds".to_string()));
    let via_params = vec![SipValue::Object(branch_param)];
    via.insert("params".to_string(), SipValue::Array(via_params));
    
    // Create headers with Via array
    let mut headers = HashMap::new();
    headers.insert("Via".to_string(), SipValue::Array(vec![SipValue::Object(via)]));
    
    // Create message object
    let mut msg = HashMap::new();
    msg.insert("headers".to_string(), SipValue::Object(headers));
    let value = SipValue::Object(msg);
    
    // Try to access elements
    println!("Testing SIP path access:");
    
    // Access headers
    let headers_value = path::get_path(&value, "headers");
    println!("headers path exists: {}", headers_value.is_some());
    
    // Access Via headers
    let via_headers = path::get_path(&value, "headers.Via");
    println!("headers.Via path exists: {}", via_headers.is_some());
    if let Some(via) = via_headers {
        println!("Via is array: {}", via.is_array());
    }
    
    // Access first Via header
    let via0 = path::get_path(&value, "headers.Via[0]");
    println!("headers.Via[0] path exists: {}", via0.is_some());
    if let Some(v0) = via0 {
        println!("Via[0] is object: {}", v0.is_object());
    }
    
    // Access branch param
    let branch = path::get_path(&value, "headers.Via[0].params[0].Branch");
    println!("headers.Via[0].params[0].Branch path exists: {}", branch.is_some());
    if let Some(branch_val) = branch {
        println!("Branch value: {:?}", branch_val.as_str());
    }
    
    // Access transport
    let transport = path::get_path(&value, "headers.Via[0].sent_protocol.transport");
    println!("headers.Via[0].sent_protocol.transport path exists: {}", transport.is_some());
    if let Some(transport_val) = transport {
        println!("Transport value: {:?}", transport_val.as_str());
    }
} 