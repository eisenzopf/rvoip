//! Advanced Routing Example
//!
//! This example demonstrates SIP routing mechanisms including Via headers,
//! Record-Route, Route processing, and proxy functionality.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use std::collections::HashMap;
use tracing::{debug, info};
use uuid::Uuid;

fn main() {
    // Initialize logging so we can see what's happening
    tracing_subscriber::fmt::init();
    
    info!("SIP Core Advanced Routing Example");
    
    // Example 1: Via header processing
    via_header_processing();
    
    // Example 2: Record-Route and Route header handling
    record_route_handling();
    
    // Example 3: SIP proxy simulation
    sip_proxy_simulation();
    
    info!("All examples completed successfully!");
}

/// Example 1: Via header processing
fn via_header_processing() {
    info!("Example 1: Via header processing");
    
    // Create a SIP request (INVITE) from Alice to Bob
    let alice_ip = "192.168.1.100";
    let alice_port = 5060;
    let alice_branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    
    let alice_request = sip! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        headers: {
            Via: format!("SIP/2.0/UDP {}:{};branch={}", alice_ip, alice_port, alice_branch),
            MaxForwards: 70,
            To: "Bob <sip:bob@example.com>",
            From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
            CallId: "a84b4c76e66710",
            CSeq: "314159 INVITE",
            Contact: "<sip:alice@192.168.1.100:5060>",
            ContentLength: 0
        }
    };
    
    info!("Alice sends INVITE to proxy");
    debug!("INVITE from Alice:\n{}", std::str::from_utf8(&alice_request.to_bytes()).unwrap());
    
    // Proxy receives the request and adds its own Via header
    let proxy_ip = "10.0.1.1";
    let proxy_port = 5060;
    let proxy_branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    
    let forwarded_request = add_via_header(
        &alice_request,
        proxy_ip,
        proxy_port,
        &proxy_branch
    );
    
    info!("Proxy forwards INVITE to Bob");
    debug!("INVITE from Proxy:\n{}", std::str::from_utf8(&forwarded_request.to_bytes()).unwrap());
    
    // Bob receives the request and creates a response
    let bob_response = create_response(&forwarded_request, StatusCode::OK);
    
    info!("Bob sends 200 OK back to Proxy");
    debug!("200 OK from Bob:\n{}", std::str::from_utf8(&bob_response.to_bytes()).unwrap());
    
    // Proxy processes the response (removes its Via header)
    let processed_response = process_response(&bob_response);
    
    info!("Proxy forwards 200 OK to Alice");
    debug!("200 OK from Proxy:\n{}", std::str::from_utf8(&processed_response.to_bytes()).unwrap());
    
    // Verify Via header processing by checking the number of Via headers
    let original_vias = alice_request.typed_headers::<Via>();
    let forwarded_vias = forwarded_request.typed_headers::<Via>();
    let bob_response_vias = bob_response.typed_headers::<Via>();
    let processed_response_vias = processed_response.typed_headers::<Via>();
    
    info!("Via headers in original request: {}", original_vias.len());
    info!("Via headers in forwarded request: {}", forwarded_vias.len());
    info!("Via headers in Bob's response: {}", bob_response_vias.len());
    info!("Via headers in processed response: {}", processed_response_vias.len());
}

/// Example 2: Record-Route and Route header handling
fn record_route_handling() {
    info!("Example 2: Record-Route and Route header handling");
    
    // Step 1: Create initial INVITE from Alice to Bob
    let alice_ip = "192.168.1.100";
    let alice_port = 5060;
    let alice_branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    let call_id = format!("{}@{}", Uuid::new_v4().to_string().split('-').next().unwrap(), alice_ip);
    
    let alice_request = sip! {
        method: Method::Invite,
        uri: "sip:bob@example.com",
        headers: {
            Via: format!("SIP/2.0/UDP {}:{};branch={}", alice_ip, alice_port, alice_branch),
            MaxForwards: 70,
            To: "Bob <sip:bob@example.com>",
            From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
            CallId: call_id,
            CSeq: "1 INVITE",
            Contact: "<sip:alice@192.168.1.100:5060>",
            ContentLength: 0
        }
    };
    
    info!("Alice sends INVITE to Proxy 1");
    
    // Step 2: Proxy 1 receives the request, adds Via and Record-Route headers
    let proxy1_ip = "10.0.1.1";
    let proxy1_port = 5060;
    let proxy1_branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    
    // Add Via header
    let mut request_with_via = add_via_header(
        &alice_request,
        proxy1_ip,
        proxy1_port,
        &proxy1_branch
    );
    
    // Add Record-Route header
    let record_route_uri = format!("sip:{}:{};lr", proxy1_ip, proxy1_port).parse::<Uri>().unwrap();
    let record_route = RecordRoute::new(Address::new(record_route_uri));
    
    request_with_via = request_with_via.with_header(TypedHeader::RecordRoute(record_route));
    
    info!("Proxy 1 forwards INVITE to Proxy 2");
    
    // Step 3: Proxy 2 receives the request, adds its own Via and Record-Route headers
    let proxy2_ip = "10.0.2.1";
    let proxy2_port = 5060;
    let proxy2_branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    
    // Add Via header
    let mut request_with_via2 = add_via_header(
        &request_with_via,
        proxy2_ip,
        proxy2_port,
        &proxy2_branch
    );
    
    // Add Record-Route header (always inserted at the top)
    let record_route_uri2 = format!("sip:{}:{};lr", proxy2_ip, proxy2_port).parse::<Uri>().unwrap();
    let record_route2 = RecordRoute::new(Address::new(record_route_uri2));
    
    // Insert at the beginning
    let mut headers = request_with_via2.headers().clone();
    headers.insert(0, HeaderField::new(
        HeaderName::RecordRoute,
        HeaderValue::new(record_route2.to_string())
    ));
    
    // Rebuild the request
    let mut builder = RequestBuilder::new(request_with_via2.method(), &request_with_via2.uri().to_string())
        .expect("Failed to create request builder");
    
    for header in headers {
        builder = builder.raw_header(header);
    }
    
    let request_with_via2 = builder.build();
    
    info!("Proxy 2 forwards INVITE to Bob");
    
    // Step 4: Bob receives the request and creates a 200 OK response
    let bob_to_tag = Uuid::new_v4().to_string().split('-').next().unwrap().to_string();
    
    // Create response but add To tag and copy Record-Route headers
    let mut response = create_response(&request_with_via2, StatusCode::OK);
    
    // Add To tag
    let to = response.typed_header::<To>().expect("Missing To header");
    response = response.with_header(TypedHeader::To(to.with_tag(&bob_to_tag)));
    
    info!("Bob sends 200 OK with Record-Route copied");
    
    // Step 5: Response traverses back through proxies
    let response_proxy2 = process_response(&response);
    info!("Proxy 2 forwards 200 OK to Proxy 1");
    
    let response_proxy1 = process_response(&response_proxy2);
    info!("Proxy 1 forwards 200 OK to Alice");
    
    // Step 6: Alice extracts the Route set from the Record-Route headers
    let record_routes = response_proxy1.typed_headers::<RecordRoute>();
    
    info!("Alice received {} Record-Route headers", record_routes.len());
    for (i, rr) in record_routes.iter().enumerate() {
        info!("Record-Route {}: {}", i+1, rr.address().uri());
    }
    
    // Step 7: Alice sends an in-dialog BYE request using the Route set
    // In-dialog request should have reversed Record-Route headers as Route headers
    let mut route_headers = Vec::new();
    for rr in record_routes.iter().rev() {
        let route_uri = rr.address().uri().clone();
        route_headers.push(Route::new(Address::new(route_uri)));
    }
    
    // Create the BYE request
    let bob_contact_uri = "sip:bob@192.168.1.200:5060".parse::<Uri>().unwrap();
    let alice_branch_bye = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    
    let mut bye_builder = RequestBuilder::new(Method::Bye, &bob_contact_uri.to_string())
        .unwrap()
        .header(TypedHeader::Via(
            Via::parse(&format!("SIP/2.0/UDP {}:{};branch={}", alice_ip, alice_port, alice_branch_bye)).unwrap()
        ))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::From(From::new(
            "Alice <sip:alice@atlanta.com>".parse::<Address>().unwrap()
                .with_parameter("tag", "1928301774")
        )))
        .header(TypedHeader::To(To::new(
            "Bob <sip:bob@example.com>".parse::<Address>().unwrap()
                .with_parameter("tag", &bob_to_tag)
        )))
        .header(TypedHeader::CallId(CallId::new(&call_id)))
        .header(TypedHeader::CSeq(CSeq::new(2, Method::Bye)))
        .header(TypedHeader::Contact(Contact::new(
            Address::new(format!("sip:alice@{}:{}", alice_ip, alice_port).parse::<Uri>().unwrap())
        )))
        .header(TypedHeader::ContentLength(ContentLength::new(0)));
    
    // Add Route headers
    for route in route_headers {
        bye_builder = bye_builder.header(TypedHeader::Route(route));
    }
    
    let bye_request = bye_builder.build();
    
    info!("Alice sends BYE with Route headers");
    debug!("BYE from Alice:\n{}", std::str::from_utf8(&bye_request.to_bytes()).unwrap());
    
    // Check Route headers
    let route_headers = bye_request.typed_headers::<Route>();
    info!("BYE request contains {} Route headers", route_headers.len());
    for (i, route) in route_headers.iter().enumerate() {
        info!("Route {}: {}", i+1, route.address().uri());
    }
}

/// Example 3: SIP proxy simulation
fn sip_proxy_simulation() {
    info!("Example 3: SIP proxy simulation");
    
    // Create a SIP network with proxies and endpoints
    let mut network = SipNetwork::new();
    
    // Add two domains with proxies
    network.add_proxy("example.com", "10.0.1.1", 5060);
    network.add_proxy("atlanta.com", "10.0.2.1", 5060);
    
    // Add users
    network.add_user("alice", "atlanta.com", "192.168.1.100", 5060);
    network.add_user("bob", "example.com", "192.168.1.200", 5060);
    
    // Alice sends an INVITE to Bob
    info!("Alice initiates a call to Bob");
    let result = network.send_request("alice@atlanta.com", "bob@example.com", Method::Invite);
    
    if result {
        info!("Call successfully established!");
        
        // Alice sends BYE to end the call
        info!("Alice terminates the call with BYE");
        let bye_result = network.send_request("alice@atlanta.com", "bob@example.com", Method::Bye);
        
        if bye_result {
            info!("Call successfully terminated!");
        } else {
            info!("Failed to terminate call!");
        }
    } else {
        info!("Call establishment failed!");
    }
}

/// Add a Via header to a request (as a proxy would do)
fn add_via_header(request: &Request, ip: &str, port: u16, branch: &str) -> Request {
    // Create a new Via header
    let via_header = Via::parse(&format!("SIP/2.0/UDP {}:{};branch={}", ip, port, branch))
        .expect("Failed to parse Via header");
    
    // Add the Via header to a cloned request (at the beginning of the list)
    let mut headers = request.headers().clone();
    headers.insert(0, HeaderField::new(
        HeaderName::Via,
        HeaderValue::new(via_header.to_string())
    ));
    
    // Create a new request with the updated headers
    let mut request_builder = RequestBuilder::new(request.method(), &request.uri().to_string())
        .expect("Failed to create request builder");
    
    // Add all headers
    for header in headers {
        request_builder = request_builder.raw_header(header);
    }
    
    // Copy the body if present
    if let Some(body) = request.body() {
        request_builder = request_builder.body(body.clone());
    }
    
    request_builder.build()
}

/// Create a response to a request
fn create_response(request: &Request, status: StatusCode) -> Response {
    ResponseBuilder::new(status)
        .unwrap()
        .headers(request.headers().clone())
        .header(TypedHeader::Contact(Contact::new(
            Address::new("sip:bob@192.168.1.200:5060".parse::<Uri>().unwrap())
        )))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

/// Process a response (as a proxy would do)
/// This removes the top Via header (the proxy's own Via)
fn process_response(response: &Response) -> Response {
    // Get all headers except the first Via
    let mut headers = response.headers().clone();
    
    // Find the first Via header index
    let first_via_index = headers.iter()
        .position(|h| h.name() == HeaderName::Via)
        .expect("No Via header found");
    
    // Remove the first Via header
    headers.remove(first_via_index);
    
    // Create a new response with the updated headers
    let mut response_builder = ResponseBuilder::new(response.status_code())
        .expect("Failed to create response builder");
    
    // Add all headers
    for header in headers {
        response_builder = response_builder.raw_header(header);
    }
    
    // Copy the body if present
    if let Some(body) = response.body() {
        response_builder = response_builder.body(body.clone());
    }
    
    response_builder.build()
}

/// Represents a user in the SIP network simulation
struct User {
    username: String,
    domain: String,
    ip: String,
    port: u16,
    contact_uri: Uri,
    dialogs: HashMap<String, Dialog>,
}

impl User {
    fn new(username: &str, domain: &str, ip: &str, port: u16) -> Self {
        let contact_uri = format!("sip:{}@{}:{}", username, ip, port).parse::<Uri>().unwrap();
        
        Self {
            username: username.to_string(),
            domain: domain.to_string(),
            ip: ip.to_string(),
            port,
            contact_uri,
            dialogs: HashMap::new(),
        }
    }
    
    fn address(&self) -> String {
        format!("{}@{}", self.username, self.domain)
    }
    
    fn create_request(&self, method: Method, to_address: &str) -> Request {
        let to_uri = format!("sip:{}", to_address).parse::<Uri>().unwrap();
        let from_uri = format!("sip:{}", self.address()).parse::<Uri>().unwrap();
        let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
        let call_id = format!("{}@{}", Uuid::new_v4().to_string().split('-').next().unwrap(), self.ip);
        let from_tag = Uuid::new_v4().to_string().split('-').next().unwrap().to_string();
        
        sip! {
            method: method,
            uri: to_uri.to_string(),
            headers: {
                Via: format!("SIP/2.0/UDP {}:{};branch={}", self.ip, self.port, branch),
                MaxForwards: 70,
                To: format!("<{}>", to_uri),
                From: format!("<{}>;tag={}", from_uri, from_tag),
                CallId: call_id,
                CSeq: format!("1 {}", method),
                Contact: format!("<{}>", self.contact_uri),
                ContentLength: 0
            }
        }
    }
    
    fn process_request(&mut self, request: &Request) -> Response {
        // This is a simplified implementation
        // In a real system, you would:
        // 1. Check if this is a new dialog or existing one
        // 2. Update dialog state as needed
        // 3. Process the request based on its method
        
        info!("{} received {} request", self.address(), request.method());
        
        let to = request.typed_header::<To>().expect("Missing To header");
        let from = request.typed_header::<From>().expect("Missing From header");
        let call_id = request.typed_header::<CallId>().expect("Missing Call-ID header");
        
        // For simplicity, we'll just accept all requests
        let to_tag = match request.method() {
            Method::Invite => Some(Uuid::new_v4().to_string().split('-').next().unwrap().to_string()),
            _ => to.tag().map(|t| t.to_string()),
        };
        
        // Create a basic response
        let mut builder = ResponseBuilder::new(StatusCode::OK)
            .unwrap()
            .headers(request.headers().clone())
            .header(TypedHeader::Contact(Contact::new(
                Address::new(self.contact_uri.clone())
            )))
            .header(TypedHeader::ContentLength(ContentLength::new(0)));
        
        // Add To tag for INVITE (for dialog creation)
        if let Some(tag) = to_tag {
            let to_with_tag = to.clone().with_tag(&tag);
            builder = builder.header(TypedHeader::To(to_with_tag));
        }
        
        builder.build()
    }
}

/// Represents a SIP dialog for the simulation
struct Dialog {
    call_id: String,
    local_tag: String,
    remote_tag: String,
    route_set: Vec<Route>,
}

impl Dialog {
    fn new(call_id: &str, local_tag: &str, remote_tag: &str, route_set: Vec<Route>) -> Self {
        Self {
            call_id: call_id.to_string(),
            local_tag: local_tag.to_string(),
            remote_tag: remote_tag.to_string(),
            route_set,
        }
    }
}

/// Represents a SIP proxy for the simulation
struct Proxy {
    domain: String,
    ip: String,
    port: u16,
    record_route: bool,
}

impl Proxy {
    fn new(domain: &str, ip: &str, port: u16) -> Self {
        Self {
            domain: domain.to_string(),
            ip: ip.to_string(),
            port,
            record_route: true,
        }
    }
    
    fn process_request(&self, request: &Request) -> Request {
        info!("Proxy for {} processing request", self.domain);
        
        // Decrement Max-Forwards
        let max_forwards = request.typed_header::<MaxForwards>()
            .expect("Missing Max-Forwards header");
        
        let new_max_forwards = max_forwards.value().saturating_sub(1);
        
        // Add Via header
        let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
        let mut processed_request = add_via_header(request, &self.ip, self.port, &branch);
        
        // Update Max-Forwards
        processed_request = processed_request.with_header(
            TypedHeader::MaxForwards(MaxForwards::new(new_max_forwards))
        );
        
        // Add Record-Route if needed
        if self.record_route && request.method() == Method::Invite {
            let record_route_uri = format!("sip:{}:{};lr", self.ip, self.port).parse::<Uri>().unwrap();
            let record_route = RecordRoute::new(Address::new(record_route_uri));
            
            processed_request = processed_request.with_header(TypedHeader::RecordRoute(record_route));
        }
        
        processed_request
    }
    
    fn process_response(&self, response: &Response) -> Response {
        info!("Proxy for {} processing response", self.domain);
        
        // Simply remove the top Via header (the proxy's Via)
        process_response(response)
    }
}

/// Simulates a SIP network with users and proxies
struct SipNetwork {
    users: HashMap<String, User>,
    proxies: HashMap<String, Proxy>,
}

impl SipNetwork {
    fn new() -> Self {
        Self {
            users: HashMap::new(),
            proxies: HashMap::new(),
        }
    }
    
    fn add_user(&mut self, username: &str, domain: &str, ip: &str, port: u16) {
        let user = User::new(username, domain, ip, port);
        self.users.insert(user.address(), user);
        info!("Added user {} to the network", user.address());
    }
    
    fn add_proxy(&mut self, domain: &str, ip: &str, port: u16) {
        let proxy = Proxy::new(domain, ip, port);
        self.proxies.insert(domain.to_string(), proxy);
        info!("Added proxy for domain {} to the network", domain);
    }
    
    fn send_request(&mut self, from_address: &str, to_address: &str, method: Method) -> bool {
        // Get the sender
        let from_user = match self.users.get(from_address) {
            Some(user) => user,
            None => {
                info!("User {} not found", from_address);
                return false;
            }
        };
        
        // Create the initial request
        let initial_request = from_user.create_request(method, to_address);
        info!("{} sends {} to {}", from_address, method, to_address);
        
        // Extract the domain from to_address
        let to_parts: Vec<&str> = to_address.split('@').collect();
        if to_parts.len() != 2 {
            info!("Invalid to address: {}", to_address);
            return false;
        }
        
        let to_domain = to_parts[1];
        
        // Route through the sender's domain proxy
        let from_parts: Vec<&str> = from_address.split('@').collect();
        let from_domain = from_parts[1];
        
        // Route the request through proxies
        let mut current_request = initial_request;
        
        // Step 1: Through sender's outbound proxy
        if let Some(from_proxy) = self.proxies.get(from_domain) {
            current_request = from_proxy.process_request(&current_request);
            info!("Request routed through {} proxy", from_domain);
        }
        
        // Step 2: Through recipient's domain proxy
        if let Some(to_proxy) = self.proxies.get(to_domain) {
            current_request = to_proxy.process_request(&current_request);
            info!("Request routed through {} proxy", to_domain);
        }
        
        // Step 3: Deliver to the recipient
        if let Some(to_user) = self.users.get_mut(to_address) {
            let response = to_user.process_request(&current_request);
            info!("{} responds with {} {}", to_address, response.status_code(), response.reason_phrase());
            
            // Route response back through proxies in reverse order
            let mut current_response = response;
            
            // Through recipient's domain proxy
            if let Some(to_proxy) = self.proxies.get(to_domain) {
                current_response = to_proxy.process_response(&current_response);
                info!("Response routed back through {} proxy", to_domain);
            }
            
            // Through sender's domain proxy
            if let Some(from_proxy) = self.proxies.get(from_domain) {
                current_response = from_proxy.process_response(&current_response);
                info!("Response routed back through {} proxy", from_domain);
            }
            
            // Deliver to sender
            info!("Response delivered to {}", from_address);
            
            // Request succeeded
            true
        } else {
            info!("User {} not found", to_address);
            false
        }
    }
} 