//! SIP Authentication Example
//!
//! This example demonstrates how to implement SIP authentication, including
//! creating and validating digest authentication headers.

use bytes::Bytes;
use rvoip_sip_core::prelude::*;
use tracing::{debug, info};
use uuid::Uuid;

fn main() {
    // Initialize logging so we can see what's happening
    tracing_subscriber::fmt::init();
    
    info!("SIP Core Authentication Example");
    
    // Example 1: Handling a 401 Unauthorized response and creating an authenticated request
    handle_401_unauthorized();
    
    // Example 2: Registrar authentication flow
    registrar_authentication_flow();
    
    // Example 3: Creating and validating Authorization headers
    create_and_validate_auth_headers();
    
    info!("All examples completed successfully!");
}

/// Example 1: Handling a 401 Unauthorized response and creating an authenticated request
fn handle_401_unauthorized() {
    info!("Example 1: Handling a 401 Unauthorized response");
    
    // Step 1: Create an initial REGISTER request without authentication
    let initial_register = create_register_request(false, None);
    
    info!("Initial REGISTER request:\n{}", std::str::from_utf8(&initial_register.to_bytes()).unwrap());
    
    // Step 2: Server responds with 401 Unauthorized
    let unauthorized_response = create_401_unauthorized(&initial_register);
    
    info!("401 Unauthorized response:\n{}", std::str::from_utf8(&unauthorized_response.to_bytes()).unwrap());
    
    // Step 3: Extract WWW-Authenticate header from 401 response
    let www_auth = unauthorized_response.typed_header::<WwwAuthenticate>()
        .expect("Missing WWW-Authenticate header");
    
    info!("Received challenge: realm={}, nonce={}", www_auth.realm(), www_auth.nonce());
    
    // Step 4: Create authenticated request using the challenge
    let authenticated_register = create_register_request(true, Some(www_auth));
    
    info!("Authenticated REGISTER request:\n{}", std::str::from_utf8(&authenticated_register.to_bytes()).unwrap());
    
    // Step 5: Verify the Authorization header is present and correctly formatted
    let auth_header = authenticated_register.typed_header::<Authorization>()
        .expect("Missing Authorization header");
    
    if auth_header.scheme() == "Digest" {
        info!("Authorization header created successfully with Digest scheme");
        info!("  Username: {}", auth_header.username());
        info!("  Realm: {}", auth_header.realm());
        info!("  URI: {}", auth_header.uri());
        info!("  Algorithm: {}", auth_header.algorithm().unwrap_or("None"));
    }
}

/// Example 2: Registrar authentication flow
fn registrar_authentication_flow() {
    info!("Example 2: Registrar authentication flow");
    
    // Step 1: Client creates an initial REGISTER request
    let alice_uri = "sip:alice@example.com";
    let user_info = UserInfo {
        username: "alice".to_string(),
        password: "secret".to_string(),
        domain: "example.com".to_string(),
    };
    
    let register_client = AuthClient::new(alice_uri, user_info);
    let initial_register = register_client.create_initial_register();
    
    info!("Client sends initial REGISTER request");
    
    // Step 2: Registrar receives request and creates 401 Unauthorized challenge
    let registrar = Registrar::new("example.com");
    let unauthorized_response = registrar.create_401_challenge(&initial_register);
    
    info!("Registrar responds with 401 Unauthorized");
    
    // Step 3: Client receives 401 and creates authenticated request
    let www_auth = unauthorized_response.typed_header::<WwwAuthenticate>()
        .expect("Missing WWW-Authenticate header");
    
    let authenticated_register = register_client.create_authenticated_register(&initial_register, www_auth);
    
    info!("Client sends authenticated REGISTER request");
    
    // Step 4: Registrar validates authentication and responds with 200 OK
    let response = registrar.process_authenticated_request(&authenticated_register);
    
    if response.status_code() == StatusCode::OK {
        info!("Registration successful!");
    } else {
        info!("Registration failed: {}", response.reason_phrase());
    }
}

/// Example 3: Creating and validating Authorization headers
fn create_and_validate_auth_headers() {
    info!("Example 3: Creating and validating Authorization headers");
    
    // Create a SIP request URI
    let request_uri = "sip:example.com".parse::<Uri>().unwrap();
    
    // Authentication credentials
    let username = "bob";
    let password = "password123";
    let realm = "example.com";
    let nonce = "dcd98b7102dd2f0e8b11d0f600bfb0c093";
    
    // Step 1: Create an Authorization header for an INVITE request
    let auth = Authorization::new_digest(
        realm,
        username,
        password,
        "INVITE",
        &request_uri.to_string()
    )
    .with_nonce(nonce)
    .with_algorithm("MD5")
    .with_qop("auth")
    .with_cnonce("0a4f113b")
    .with_nc("00000001");
    
    info!("Created Authorization header:\n{}", auth);
    
    // Step 2: Validate the credentials on the server side
    let expected_response = compute_digest_response(
        username,
        realm,
        password,
        "INVITE",
        &request_uri.to_string(),
        nonce,
        auth.qop(),
        auth.cnonce(),
        auth.nc()
    );
    
    if auth.response() == expected_response {
        info!("Authorization header validation successful!");
    } else {
        info!("Authorization header validation failed!");
        info!("Expected: {}", expected_response);
        info!("Actual: {}", auth.response());
    }
    
    // Step 3: Show what happens with wrong password
    let wrong_auth = Authorization::new_digest(
        realm,
        username,
        "wrong_password",
        "INVITE",
        &request_uri.to_string()
    )
    .with_nonce(nonce)
    .with_algorithm("MD5")
    .with_qop("auth")
    .with_cnonce("0a4f113b")
    .with_nc("00000001");
    
    if wrong_auth.response() == expected_response {
        info!("Wrong password validation passed (unexpected!)");
    } else {
        info!("Wrong password correctly rejected");
    }
}

/// Helper function to create a REGISTER request (with or without authentication)
fn create_register_request(with_auth: bool, www_auth: Option<&WwwAuthenticate>) -> Request {
    let contact_uri = "sip:alice@192.168.1.100:5060".parse::<Uri>().unwrap();
    let domain = "example.com";
    let register_uri = format!("sip:{}", domain).parse::<Uri>().unwrap();
    let from_uri = "sip:alice@example.com".parse::<Uri>().unwrap();
    let call_id = format!("{}@192.168.1.100", Uuid::new_v4().to_string().split('-').next().unwrap());
    let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
    
    let mut builder = RequestBuilder::new(Method::Register, &register_uri.to_string())
        .unwrap()
        .header(TypedHeader::Via(
            Via::parse(&format!("SIP/2.0/UDP 192.168.1.100:5060;branch={}", branch)).unwrap()
        ))
        .header(TypedHeader::From(From::new(
            Address::new(from_uri.clone()).with_parameter("tag", 
                Uuid::new_v4().to_string().split('-').next().unwrap())
        )))
        .header(TypedHeader::To(To::new(
            Address::new(from_uri)
        )))
        .header(TypedHeader::CallId(CallId::new(&call_id)))
        .header(TypedHeader::CSeq(CSeq::new(1, Method::Register)))
        .header(TypedHeader::Contact(Contact::new(
            Address::new(contact_uri)
        )))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::Expires(Expires::new(3600)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)));
    
    // Add Authorization header if requested
    if with_auth && www_auth.is_some() {
        let www_auth = www_auth.unwrap();
        
        // Create an Authorization header using the WWW-Authenticate challenge
        let auth = Authorization::new_digest(
            www_auth.realm(),
            "alice",
            "secret",
            "REGISTER",
            &register_uri.to_string()
        )
        .with_nonce(www_auth.nonce())
        .with_algorithm(www_auth.algorithm().unwrap_or("MD5"))
        .with_qop("auth")
        .with_cnonce("0a4f113b")
        .with_nc("00000001");
        
        builder = builder.header(TypedHeader::Authorization(auth));
    }
    
    builder.build()
}

/// Helper function to create a 401 Unauthorized response
fn create_401_unauthorized(request: &Request) -> Response {
    let nonce = format!("{}", Uuid::new_v4());
    
    ResponseBuilder::new(StatusCode::Unauthorized)
        .unwrap()
        .headers(request.headers().clone())
        .header(TypedHeader::WwwAuthenticate(
            WwwAuthenticate::new_digest("example.com")
                .with_nonce(&nonce)
                .with_algorithm("MD5")
                .with_qop("auth")
        ))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

/// Client that handles authentication
struct AuthClient {
    uri: String,
    user_info: UserInfo,
    cseq: u32,
}

impl AuthClient {
    fn new(uri: &str, user_info: UserInfo) -> Self {
        Self {
            uri: uri.to_string(),
            user_info,
            cseq: 1,
        }
    }
    
    fn create_initial_register(&self) -> Request {
        let register_uri = format!("sip:{}", self.user_info.domain).parse::<Uri>().unwrap();
        let from_uri = self.uri.parse::<Uri>().unwrap();
        let contact_uri = format!("sip:{}@192.168.1.100:5060", self.user_info.username).parse::<Uri>().unwrap();
        let call_id = format!("{}@192.168.1.100", Uuid::new_v4().to_string().split('-').next().unwrap());
        let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
        
        RequestBuilder::new(Method::Register, &register_uri.to_string())
            .unwrap()
            .header(TypedHeader::Via(
                Via::parse(&format!("SIP/2.0/UDP 192.168.1.100:5060;branch={}", branch)).unwrap()
            ))
            .header(TypedHeader::From(From::new(
                Address::new(from_uri.clone()).with_parameter("tag", 
                    Uuid::new_v4().to_string().split('-').next().unwrap())
            )))
            .header(TypedHeader::To(To::new(
                Address::new(from_uri)
            )))
            .header(TypedHeader::CallId(CallId::new(&call_id)))
            .header(TypedHeader::CSeq(CSeq::new(self.cseq, Method::Register)))
            .header(TypedHeader::Contact(Contact::new(
                Address::new(contact_uri)
            )))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::Expires(Expires::new(3600)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build()
    }
    
    fn create_authenticated_register(&self, original_request: &Request, www_auth: &WwwAuthenticate) -> Request {
        // Increment CSeq for new request
        let cseq = original_request.typed_header::<CSeq>().unwrap();
        let new_cseq = cseq.sequence() + 1;
        
        // Get the original From header with its tag
        let from = original_request.typed_header::<From>().unwrap();
        
        // Get the Call-ID
        let call_id = original_request.typed_header::<CallId>().unwrap();
        
        // Create a new branch parameter for Via
        let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().split('-').next().unwrap());
        
        // Get request URI
        let register_uri = original_request.uri().to_string();
        
        // Create an Authorization header
        let auth = Authorization::new_digest(
            www_auth.realm(),
            &self.user_info.username,
            &self.user_info.password,
            "REGISTER",
            &register_uri
        )
        .with_nonce(www_auth.nonce())
        .with_algorithm(www_auth.algorithm().unwrap_or("MD5"))
        .with_qop("auth")
        .with_cnonce("0a4f113b")
        .with_nc("00000001");
        
        // Build the new request, copying most headers from the original
        let mut builder = RequestBuilder::new(Method::Register, &register_uri)
            .unwrap()
            .header(TypedHeader::Via(
                Via::parse(&format!("SIP/2.0/UDP 192.168.1.100:5060;branch={}", branch)).unwrap()
            ))
            .header(TypedHeader::From(from.clone()))
            .header(TypedHeader::To(original_request.typed_header::<To>().unwrap().clone()))
            .header(TypedHeader::CallId(call_id.clone()))
            .header(TypedHeader::CSeq(CSeq::new(new_cseq, Method::Register)))
            .header(TypedHeader::Contact(original_request.typed_header::<Contact>().unwrap().clone()))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::Expires(Expires::new(3600)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .header(TypedHeader::Authorization(auth));
        
        // Add any other headers from the original request that we want to preserve
        if let Some(user_agent) = original_request.typed_header::<UserAgent>() {
            builder = builder.header(TypedHeader::UserAgent(user_agent.clone()));
        }
        
        builder.build()
    }
}

/// Server-side registrar that handles authentication
struct Registrar {
    realm: String,
    // In a real implementation, this would contain a database of users and registered contacts
}

impl Registrar {
    fn new(realm: &str) -> Self {
        Self {
            realm: realm.to_string(),
        }
    }
    
    fn create_401_challenge(&self, request: &Request) -> Response {
        let nonce = format!("{}", Uuid::new_v4());
        
        ResponseBuilder::new(StatusCode::Unauthorized)
            .unwrap()
            .headers(request.headers().clone())
            .header(TypedHeader::WwwAuthenticate(
                WwwAuthenticate::new_digest(&self.realm)
                    .with_nonce(&nonce)
                    .with_algorithm("MD5")
                    .with_qop("auth")
            ))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build()
    }
    
    fn process_authenticated_request(&self, request: &Request) -> Response {
        // In a real implementation, you would:
        // 1. Verify the Authorization header exists
        // 2. Validate the digest response by computing the expected response
        // 3. Check the nonce is valid and not expired or replayed
        // 4. If all checks pass, register the contact and send 200 OK
        
        if let Some(auth) = request.typed_header::<Authorization>() {
            // In this example, we'll just check if the username is "alice" and pretend the digest is valid
            if auth.username() == "alice" {
                // Authentication successful
                ResponseBuilder::new(StatusCode::OK)
                    .unwrap()
                    .headers(request.headers().clone())
                    .header(TypedHeader::ContentLength(ContentLength::new(0)))
                    .build()
            } else {
                // Authentication failed
                ResponseBuilder::new(StatusCode::Forbidden)
                    .unwrap()
                    .headers(request.headers().clone())
                    .header(TypedHeader::ContentLength(ContentLength::new(0)))
                    .build()
            }
        } else {
            // No Authorization header
            self.create_401_challenge(request)
        }
    }
}

/// Helper struct to store user credentials
struct UserInfo {
    username: String,
    password: String,
    domain: String,
}

/// Helper function to compute a digest response
/// This is a simplified version for demonstration purposes
fn compute_digest_response(
    username: &str,
    realm: &str,
    password: &str,
    method: &str,
    uri: &str,
    nonce: &str,
    qop: Option<&str>,
    cnonce: Option<&str>,
    nc: Option<&str>
) -> String {
    // In a real implementation, you would:
    // 1. Compute HA1 = MD5(username:realm:password)
    // 2. Compute HA2 = MD5(method:uri)
    // 3. If qop is present:
    //    response = MD5(HA1:nonce:nc:cnonce:qop:HA2)
    // 4. Otherwise:
    //    response = MD5(HA1:nonce:HA2)
    
    // For this example, we'll just return a dummy response
    // In a real implementation, use an actual MD5 library
    format!("dummy_response_for_{}:{}:{}:{}:{}:{}:{}:{}",
        username, realm, password, method, uri, nonce, 
        qop.unwrap_or(""), cnonce.unwrap_or(""))
} 