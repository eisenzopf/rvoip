# Example 5: Authentication and Security

This example demonstrates how to implement SIP authentication using the digest authentication mechanism defined in RFC 3261. Authentication is a crucial security feature in SIP that allows servers to verify client identities and prevent unauthorized access.

## What You'll Learn

- How to handle 401 Unauthorized responses and create authenticated requests
- How to create and validate Authorization headers
- How to implement digest authentication for SIP clients and servers
- How to properly handle nonce values, challenge-response mechanisms, and other security parameters
- How the SIP registration process works with authentication
- Best practices for secure SIP implementations

## Running the Example

```bash
# Run the example
cargo run --example 05_authentication

# Run with debug logs for more detail
RUST_LOG=debug cargo run --example 05_authentication
```

## Code Walkthrough

The example is divided into three parts:

1. **Handling 401 Unauthorized Responses**
   - Shows how to create an initial REGISTER request without authentication
   - Demonstrates how a server responds with a 401 Unauthorized challenge
   - Illustrates extracting authentication parameters from WWW-Authenticate headers
   - Shows creating a new request with proper Authorization headers

2. **Registrar Authentication Flow**
   - Implements a complete client-server authentication flow
   - Shows a reusable client implementation for handling authentication
   - Demonstrates server-side challenge creation and validation
   - Illustrates how CSeq values increment for retransmissions

3. **Creating and Validating Authorization Headers**
   - Shows how to create Authorization headers with various parameters
   - Demonstrates how to compute and validate digest responses
   - Illustrates what happens with correct vs. incorrect credentials
   - Shows how to handle different authentication quality-of-protection (qop) values

## Key Concepts

### SIP Digest Authentication

SIP uses HTTP Digest Authentication (RFC 2617) with a challenge-response mechanism:

1. Client sends request without authentication
2. Server responds with 401 Unauthorized containing a WWW-Authenticate header
3. Client creates a new request with an Authorization header containing a digest response
4. Server validates the digest and grants or denies access

### Authentication Headers

- **WWW-Authenticate**: Sent by server to challenge client
  - Contains realm, nonce, algorithm, qop (quality of protection)
  
- **Authorization**: Sent by client to authenticate
  - Contains username, realm, nonce, URI, response, algorithm, qop, cnonce, nc

### Digest Calculation

The digest response is calculated as:
```
HA1 = MD5(username:realm:password)
HA2 = MD5(method:uri)

If qop is present:
  response = MD5(HA1:nonce:nc:cnonce:qop:HA2)
Otherwise:
  response = MD5(HA1:nonce:HA2)
```

### Security Best Practices

- Always validate nonce values and prevent replay attacks
- Use a strong random number generator for nonce creation
- Implement nonce timeouts and rate limiting
- Store passwords securely (never in plaintext)
- Consider TLS transport for additional security

## Next Steps

After mastering authentication, you can proceed to Example 6 which covers advanced SIP routing mechanisms for proxies and servers. 