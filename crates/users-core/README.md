# Users-Core

User management and authentication service for RVoIP. Issues JWT tokens that are validated by auth-core.

## Overview

Users-Core provides internal user management and JWT token issuance for the RVoIP ecosystem. It works in conjunction with auth-core, which validates tokens from all sources (users-core, OAuth2 providers, etc.).

### What Users-Core Does:
- 🔐 **User accounts** - Create and manage users with secure password storage
- 🎫 **JWT tokens** - Issue and manage access tokens for your APIs  
- 🔑 **API keys** - Long-lived keys for service accounts and integrations
- 🌐 **REST API** - Ready-to-use HTTP endpoints for all operations

### Architecture

```
┌─────────────────┐
│   Clients       │
└────────┬────────┘
         │ Login/Register
         ▼
┌─────────────────┐      Issues JWT      ┌─────────────────┐
│   Users-Core    │───────────────────▶  │   Auth-Core     │
│                 │                       │                 │
│ • Store Users   │      Validates       │ • Validate JWT  │
│ • Issue JWT     │◀─────────────────────│ • Cache Tokens  │
│ • API Keys      │      Public Key      │ • OAuth2 Too    │
└────────┬────────┘                       └─────────────────┘
         │                                         ▲
         ▼                                         │
┌─────────────────┐                       ┌────────┴────────┐
│     SQLite      │                       │ Session-Core-V2 │
└─────────────────┘                       │                 │
                                          │ • SIP REGISTER  │
                                          │ • Uses tokens   │
                                          └─────────────────┘
```

**Key Points:**
- Users-Core **issues** JWT tokens but doesn't validate them
- Auth-Core **validates** all tokens (from users-core, OAuth2, etc.)
- Session-Core-V2 calls Auth-Core for validation, not Users-Core directly
- This separation allows flexible authentication strategies

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
users-core = "0.1"

# If you want to use the REST API client examples
users-core = { version = "0.1", features = ["client"] }
```

## Quick Start

### 1. Basic Usage (Library)

```rust
use users_core::{init, CreateUserRequest, UsersConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize with default config (uses sqlite://users.db)
    let config = UsersConfig::default();
    let auth = init(config).await?;
    
    // Create a user
    let user = auth.create_user(CreateUserRequest {
        username: "alice".to_string(),
        password: "SecurePass123!".to_string(),  // 12+ chars required
        email: Some("alice@example.com".to_string()),
        display_name: Some("Alice Smith".to_string()),
        roles: vec!["user".to_string()],  // Allowed: "user", "admin", "moderator", "guest"
    }).await?;
    
    println!("Created user: {}", user.username);
    
    // Authenticate
    let result = auth.authenticate_password("alice", "SecurePass123!").await?;
    println!("Access token: {}", result.access_token);
    
    Ok(())
}
```

### 2. REST API Server

The easiest way to get started is using the built-in REST API:

```bash
# Run the example REST API server
cargo run --example rest_api_server

# Or run the interactive demo (on port 8082)
cd examples/rest_api_demo
./run_demo.sh

# Run all examples at once
./examples/run_all_examples.sh
```

This starts a full REST API on `http://localhost:8081` (or 8082 for the demo) with all endpoints ready to use.

### 3. Try the Demo

The `rest_api_demo` shows all features in action:

```bash
cd crates/users-core/examples/rest_api_demo
./run_demo.sh
```

This will:
- Start a server with an admin user
- Run a client that tests all endpoints
- Show you exactly how everything works

## Key Features

### 🔐 User Management

```rust
// Create a user
let user = auth.create_user(CreateUserRequest {
    username: "bob".to_string(),
    password: "BobSecurePass123".to_string(),  // 12+ chars required
    email: Some("bob@example.com".to_string()),
    display_name: None,
    roles: vec!["user".to_string()],
}).await?;

// Update user
auth.user_store().update_user(&user.id, UpdateUserRequest {
    email: Some(Some("newemail@example.com".to_string())),
    display_name: Some(Some("Bob Smith".to_string())),
    roles: Some(vec!["user".to_string(), "admin".to_string()]),
    active: None,
}).await?;

// List all users
let users = auth.user_store().list_users(UserFilter::default()).await?;
```

### 🎫 JWT Authentication

```rust
// Login with password
let tokens = auth.authenticate_password("alice", "SecurePass123!").await?;

// Access token for API calls
println!("Access token: {}", tokens.access_token);  

// Refresh token for getting new access tokens
println!("Refresh token: {}", tokens.refresh_token);

// Refresh the access token
let new_tokens = auth.refresh_token(&tokens.refresh_token).await?;
```

### 🔑 API Keys

```rust
// Create an API key for service accounts
let (api_key_info, raw_key) = auth.api_key_store()
    .create_api_key(CreateApiKeyRequest {
        user_id: user.id.clone(),
        name: "My Service".to_string(),
        permissions: vec!["read".to_string(), "write".to_string()],
        expires_at: None,
    })
    .await?;

println!("API Key: {}", raw_key);  // Save this! Can't retrieve it later

// Allowed permissions: "read", "write", "delete", "admin", "*" (wildcard)

// Validate API key
let key_info = auth.api_key_store()
    .validate_api_key(&raw_key)
    .await?;
```

## REST API Endpoints

For complete API documentation including request/response schemas, see the [OpenAPI specification](./openapi.yaml).

### Authentication
- `POST /auth/login` - Login with username/password
- `POST /auth/logout` - Logout and revoke tokens  
- `POST /auth/refresh` - Get new access token
- `GET /auth/jwks.json` - Public keys for token validation

### Users
- `POST /users` - Create user (admin only)
- `GET /users` - List all users
- `GET /users/:id` - Get user details
- `PUT /users/:id` - Update user
- `DELETE /users/:id` - Delete user (admin only)
- `POST /users/:id/password` - Change password
- `POST /users/:id/roles` - Update roles (admin only)

### API Keys
- `POST /users/:id/api-keys` - Create API key
- `GET /users/:id/api-keys` - List user's keys
- `DELETE /api-keys/:id` - Revoke API key

### Monitoring
- `GET /health` - Health check
- `GET /metrics` - Service metrics

## Configuration

Create a `users_core.toml` file or use environment variables:

```toml
[users_core]
# Database location
database_url = "sqlite://users.db"

# REST API binding
api_bind_address = "127.0.0.1:8081"

[users_core.jwt]
# Token settings
issuer = "https://users.rvoip.local"
audience = ["rvoip-api", "rvoip-sip"]
access_ttl_seconds = 900        # 15 minutes
refresh_ttl_seconds = 2592000   # 30 days

[users_core.password]
# Password requirements
min_length = 12              # Minimum 12 characters (default)
require_uppercase = true
require_lowercase = true
require_numbers = true
require_special = false      # Optional (modern best practice)

# Argon2 settings (advanced)
argon2_memory_cost = 65536      # 64MB
argon2_time_cost = 3
argon2_parallelism = 4

[users_core.tls]
# HTTPS/TLS configuration (strongly recommended for production)
enabled = false              # Set to true to enable HTTPS
cert_path = "certs/server.crt"
key_path = "certs/server.key"
require_tls = true           # If true, refuse to start without TLS
```

### Enabling HTTPS

For production, always use HTTPS:

```bash
# Generate development certificates
./scripts/generate_dev_certs.sh

# Enable TLS in your configuration
# Set [users_core.tls] enabled = true
```

## Examples

Check out the `examples/` directory for complete working examples:

- `basic_usage.rs` - Simple user creation and login
- `token_validation.rs` - JWT validation and introspection
- `api_key_service.rs` - Using API keys for services
- `sip_register_flow.rs` - SIP REGISTER authentication flow
- `multi_device_presence.rs` - Multi-device registration example
- `session_core_v2_integration.rs` - Full integration example
- `rest_api_demo/` - Interactive client/server demo

### Running Examples

```bash
# Run a specific example
cargo run --example basic_usage

# Run all examples with detailed output
./examples/run_all_examples.sh

# Run all examples quietly (CI mode)
./examples/run_all_examples_ci.sh
```

## Rate Limiting

Rate limiting is automatically applied to all REST API endpoints to prevent abuse and protect your service.

### How It Works

The rate limiter tracks requests in two ways:
- **Authenticated users**: Tracked by user ID
- **Unauthenticated requests**: Tracked by IP address

### Default Limits

```
General API Requests:
- 100 requests per minute per user/IP
- 1000 requests per hour per user/IP

Login Attempts:
- 5 failed attempts per hour per username
- 15-minute lockout after exceeding limit
```

### Response Headers

When rate limited, the API returns:
- **Status**: `429 Too Many Requests`
- **Header**: `Retry-After: <seconds>` indicating when to retry

### Example Responses

```bash
# Too many requests
HTTP/1.1 429 Too Many Requests
Retry-After: 60

# Account locked due to failed logins
HTTP/1.1 429 Too Many Requests
Retry-After: 900
```

### Important Notes

1. **Currently not configurable**: Rate limits are hardcoded in the library. Future versions will add configuration options.

2. **In-memory storage**: Rate limit tracking is stored in memory, so it resets when the server restarts.

3. **Failed login tracking**: Failed login attempts are tracked by username (not IP) to prevent account-specific brute force attacks.

4. **Automatic cleanup**: Old rate limit entries are automatically cleaned up every 5 minutes.

### Programmatic Usage

If you're using the library directly (not REST API), rate limiting is only applied to the REST endpoints. Direct library calls are not rate limited:

```rust
// Direct library calls are NOT rate limited
let result = auth.authenticate_password("alice", "password").await?;

// REST API calls ARE rate limited
let response = client.post("http://localhost:8081/auth/login")
    .json(&login_request)
    .send()
    .await?;
```

### Future Enhancements

- Configuration through `UsersConfig`
- Redis-based storage for distributed deployments
- Per-endpoint custom limits
- API key-specific rate limits
- Whitelist/bypass for trusted IPs

## Role-Based Access Control (RBAC)

Users-Core implements a simple but effective role-based access control system.

### Available Roles

```
- user      # Standard user role
- admin     # Administrative privileges
- moderator # Moderation capabilities (reserved for future use)
- guest     # Limited access (reserved for future use)
```

### Assigning Roles

Roles are assigned when creating or updating users:

```rust
// Create a user with roles
let user = auth.create_user(CreateUserRequest {
    username: "alice".to_string(),
    password: "SecurePass123!".to_string(),
    email: Some("alice@example.com".to_string()),
    display_name: Some("Alice Smith".to_string()),
    roles: vec!["user".to_string(), "admin".to_string()],
}).await?;

// Update user roles (admin only)
auth.user_store().update_user(&user_id, UpdateUserRequest {
    roles: Some(vec!["user".to_string()]),  // Remove admin role
    ..Default::default()
}).await?;

// REST API - Update roles endpoint
// POST /users/{id}/roles
// Requires admin authentication
```

### REST API Role Requirements

| Endpoint | Method | Required Role | Notes |
|----------|--------|---------------|-------|
| `/users` | POST | admin | Create new users |
| `/users` | GET | authenticated | List all users |
| `/users/:id` | GET | authenticated | View user details |
| `/users/:id` | PUT | self or admin | Update user info |
| `/users/:id` | DELETE | admin | Delete users |
| `/users/:id/password` | POST | self only | Change own password |
| `/users/:id/roles` | POST | admin | Update user roles |
| `/users/:id/api-keys` | POST | self or admin | Create API keys |
| `/users/:id/api-keys` | GET | self or admin | List API keys |
| `/api-keys/:id` | DELETE | key owner | Revoke API key |
| `/metrics` | GET | authenticated | View metrics |

### Checking Roles in Code

When using the library directly:

```rust
// Check if a user has admin role
let user = auth.user_store().get_user(&user_id).await?;
if user.roles.contains(&"admin".to_string()) {
    // Admin-only operation
}
```

In REST API handlers (automatically populated from JWT):

```rust
// AuthContext is automatically extracted from JWT
async fn admin_only_handler(
    auth: AuthContext,  // Automatically populated
) -> Result<Response, Error> {
    if !auth.is_admin() {
        return Err(Error::Forbidden);
    }
    // Admin operation here
}

// Check specific permissions for API keys
if auth.has_permission("write") {
    // Allowed for JWT tokens or API keys with "write" permission
}
```

### JWT Token Claims

When a user authenticates, their roles are included in the JWT token:

```json
{
  "sub": "user-id-here",
  "username": "alice",
  "email": "alice@example.com",
  "roles": ["user", "admin"],
  "scope": "openid profile email admin",
  "iat": 1735689600,
  "exp": 1735690500
}
```

### API Key Permissions vs Roles

- **JWT tokens**: Include user roles and have full API access
- **API keys**: Have specific permissions but no roles

API key permissions:
- `read` - Read access to resources
- `write` - Write/modify access
- `delete` - Delete access
- `admin` - Administrative operations
- `*` - Wildcard (all permissions)

### Example: Admin-Only Operations

```bash
# REST API - Create a new user (admin only)
curl -X POST http://localhost:8081/users \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "username": "newuser",
    "password": "SecurePass123!",
    "roles": ["user"]
  }'

# Will fail with 403 Forbidden if not admin
curl -X POST http://localhost:8081/users \
  -H "Authorization: Bearer $USER_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"username": "newuser", "password": "pass"}'

# Update user roles (admin only)
curl -X POST http://localhost:8081/users/$USER_ID/roles \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"roles": ["user", "moderator"]}'
```

### Best Practices

1. **Principle of Least Privilege**: Only assign the minimum roles needed
2. **Regular Audits**: Periodically review who has admin access
3. **Role Separation**: Use API keys for services instead of admin accounts
4. **Secure Defaults**: New users get only the "user" role by default

### Future RBAC Enhancements

- Custom roles and permissions
- Role hierarchies
- Resource-based permissions
- Fine-grained access control
- Permission inheritance

## Security Features

✅ **Password Security**
- Argon2id hashing (resistant to GPU attacks)
- Strong default policy:
  - Minimum 12 characters
  - Requires uppercase, lowercase, and numbers
  - Blocks common passwords
  - Prevents username in password
  - Limits consecutive characters (max 3)
  - Requires 6+ unique characters
- Password strength meter
- No passwords stored in plain text

✅ **Token Security**  
- RS256 signed JWTs (asymmetric crypto)
- Short-lived access tokens (15 min default)
- Refresh tokens for seamless re-authentication
- Token revocation support

✅ **API Key Security**
- SHA256 hashed storage
- Granular permissions
- Expiration support
- One-time display (can't retrieve after creation)

✅ **General Security**
- Rate limiting (see Rate Limiting section above)
- SQL injection protection (parameterized queries)
- Input validation:
  - Username: 3-32 chars, alphanumeric + ._-
  - Email validation with dangerous character blocking
  - Display name sanitization (XSS prevention)
  - Role whitelisting: "user", "admin", "moderator", "guest"
- Timing attack prevention (constant-time auth)
- HTTPS/TLS support with security headers:
  - HSTS, CSP, X-Frame-Options, etc.
- CORS support for web apps
- Audit trail for sensitive operations

## Integration with RVoIP

Users-Core is a critical component of RVoIP's authentication architecture:

### How It Works

1. **Users authenticate** with Users-Core (username/password)
2. **Users-Core issues** JWT tokens signed with RS256
3. **Clients include** tokens in SIP REGISTER or API calls
4. **Services validate** tokens through Auth-Core (not Users-Core)
5. **Auth-Core trusts** Users-Core via JWKS endpoint

### Integration with Auth-Core

Auth-Core should be configured to trust tokens issued by users-core:

```rust
// In auth-core configuration
auth_core.add_trusted_issuer(TrustedIssuer {
    issuer: "https://users.rvoip.local",
    jwks_uri: Some("http://users-core:8081/auth/jwks.json"),
    audiences: vec!["rvoip-api", "rvoip-sip"],
})?;
```

### Usage in Session-Core-V2

```rust
// SIP REGISTER includes bearer token
REGISTER sip:example.com SIP/2.0
Authorization: Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...
```

Session-Core-V2 then validates this token via Auth-Core, not directly with Users-Core.

## Troubleshooting

**Database already exists error?**
```bash
rm users.db  # Remove old database
cargo run --example rest_api_server  # Start fresh
```

**Can't validate tokens?**
- Check the JWKS endpoint: `http://localhost:8081/auth/jwks.json`
- Ensure auth-core is configured with the correct issuer

**Password requirements failing?**
- Default: 12+ chars, uppercase, lowercase, numbers
- No consecutive characters (e.g., "111", "aaa")
- Cannot contain username
- Needs 6+ unique characters
- Configure in `[users_core.password]` section

## Contributing

We welcome contributions! Please see the main RVoIP contributing guidelines.

## License

Dual-licensed under MIT or Apache 2.0 - choose whichever fits your project!
