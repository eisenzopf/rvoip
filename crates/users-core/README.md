# Users-Core

User management and authentication service for RVoIP.

## Overview

Users-Core provides internal user management and JWT token issuance for the RVoIP ecosystem. It works in conjunction with auth-core, which validates tokens from all sources (users-core, OAuth2 providers, etc.).

## Features

- **User Management**: CRUD operations for users stored in SQLite
- **Password Authentication**: Secure password hashing with Argon2
- **JWT Token Issuance**: Issues RS256-signed JWT tokens
- **API Key Management**: Long-lived API keys for service accounts
- **REST API**: Full REST API for user operations
- **Role-Based Access**: Simple RBAC system

## Architecture

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
         │
         ▼
┌─────────────────┐
│     SQLite      │
└─────────────────┘
```

## Quick Start

```rust
use users_core::{UsersConfig, init};

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    let config = UsersConfig::from_env()?;
    
    // Initialize service
    let auth_service = init(config).await?;
    
    // Create a user
    let user = auth_service.create_user(CreateUserRequest {
        username: "alice".to_string(),
        password: "secure_password".to_string(),
        email: Some("alice@example.com".to_string()),
        roles: vec!["user".to_string()],
    }).await?;
    
    // Authenticate
    let result = auth_service.authenticate_password(
        "alice",
        "secure_password"
    ).await?;
    
    println!("Access token: {}", result.access_token);
    Ok(())
}
```

## REST API

### Authentication Endpoints

- `POST /auth/login` - Login with username/password
- `POST /auth/logout` - Logout (revoke tokens)
- `POST /auth/refresh` - Refresh access token
- `GET /auth/jwks.json` - Public keys for token validation

### User Management

- `POST /users` - Create new user
- `GET /users/{id}` - Get user details
- `PUT /users/{id}` - Update user
- `DELETE /users/{id}` - Delete user
- `GET /users` - List users (with pagination)

### API Keys

- `POST /users/{id}/api-keys` - Create API key
- `GET /users/{id}/api-keys` - List user's API keys
- `DELETE /api-keys/{id}` - Revoke API key

## Configuration

```toml
[users_core]
database_url = "sqlite://users.db"

[users_core.jwt]
issuer = "https://users.rvoip.local"
audience = ["rvoip-api", "rvoip-sip"]
access_ttl_seconds = 900
refresh_ttl_seconds = 2592000

[users_core.password]
min_length = 8
require_uppercase = true
argon2_memory_cost = 65536
```

## Integration with Auth-Core

Auth-Core should be configured to trust tokens issued by users-core:

```rust
// In auth-core configuration
auth_core.add_trusted_issuer(TrustedIssuer {
    issuer: "https://users.rvoip.local",
    jwks_uri: Some("http://users-core:8081/auth/jwks.json"),
    audiences: vec!["rvoip-api", "rvoip-sip"],
})?;
```

## Database Schema

The service automatically creates and migrates the SQLite database with tables for:

- `users` - User accounts
- `api_keys` - API key storage
- `refresh_tokens` - Refresh token tracking
- `sessions` - Active sessions (optional)

## Security

- Passwords hashed with Argon2id
- JWT tokens signed with RS256
- API keys stored as hashes
- Rate limiting on authentication endpoints
- Automatic token expiration

## License

Dual-licensed under MIT or Apache 2.0
