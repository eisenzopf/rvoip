use rvoip_sip_core::prelude::*;
use std::str::FromStr;

fn main() {
    println!("Testing parameter behavior");
    
    // Create URI and address, then add the tag parameter
    let uri = Uri::from_str("sip:bob@example.com").unwrap();
    let mut address = Address::new(None::<&str>, uri);
    
    // Add tag parameter to Address (not URI)
    address.set_tag("1234");
    
    // Add lr parameter to Address
    address.params.push(Param::Lr);
    
    println!("Address: {}", address);
    println!("has_param(\"tag\"): {}", address.has_param("tag"));
    println!("get_param(\"tag\"): {:?}", address.get_param("tag"));
    
    println!("has_param(\"lr\"): {}", address.has_param("lr"));
    println!("get_param(\"lr\"): {:?}", address.get_param("lr"));
    
    println!("has_param(\"unknown\"): {}", address.has_param("unknown"));
    println!("get_param(\"unknown\"): {:?}", address.get_param("unknown"));
    
    // Create an address with URI parameters
    let uri_with_param = Uri::from_str("sip:alice@example.com;transport=tcp").unwrap();
    let addr2 = Address::new(None::<&str>, uri_with_param);
    
    println!("\nAddress with URI param: {}", addr2);
    println!("has_param(\"transport\"): {}", addr2.has_param("transport"));
    println!("get_param(\"transport\"): {:?}", addr2.get_param("transport"));
    println!("URI: {}", addr2.uri);
} 