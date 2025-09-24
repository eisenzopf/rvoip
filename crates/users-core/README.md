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
        password: "SecurePass123".to_string(),
        email: Some("alice@example.com".to_string()),
        display_name: Some("Alice Smith".to_string()),
        roles: vec!["user".to_string()],
    }).await?;
    
    println!("Created user: {}", user.username);
    
    // Authenticate
    let result = auth.authenticate_password("alice", "SecurePass123").await?;
    println!("Access token: {}", result.access_token);
    
    Ok(())
}
```

### 2. REST API Server

The easiest way to get started is using the built-in REST API:

```bash
# Run the example REST API server
cargo run --example rest_api_server

# Or run the interactive demo
cd examples/rest_api_demo
./run_demo.sh
```

This starts a full REST API on `http://localhost:8081` with all endpoints ready to use.

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
    password: "BobPass123".to_string(),
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
let tokens = auth.authenticate_password("alice", "SecurePass123").await?;

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
min_length = 8
require_uppercase = true
require_lowercase = true
require_numbers = true
require_special = false

# Argon2 settings (advanced)
argon2_memory_cost = 65536      # 64MB
argon2_time_cost = 3
argon2_parallelism = 4
```

## Examples

Check out the `examples/` directory for complete working examples:

- `basic_usage.rs` - Simple user creation and login
- `jwt_validation.rs` - How to validate tokens
- `api_key_service.rs` - Using API keys for services
- `rest_api_server.rs` - Full REST API server
- `rest_api_demo/` - Interactive demo with client/server

## Security Features

✅ **Password Security**
- Argon2id hashing (resistant to GPU attacks)
- Configurable complexity requirements
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
- Rate limiting on auth endpoints
- SQL injection protection
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
- Default: 8+ chars, uppercase, lowercase, numbers
- Configure in `[users_core.password]` section

## Contributing

We welcome contributions! Please see the main RVoIP contributing guidelines.

## License

Dual-licensed under MIT or Apache 2.0 - choose whichever fits your project!
