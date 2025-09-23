# Users-Core Examples

This directory contains practical examples demonstrating how to use users-core, with a focus on integration with session-core-v2.

## Running the Examples

To run any example:

```bash
cargo run --example <example_name>
```

For example:
```bash
cargo run --example basic_usage
```

## Examples Overview

### 1. `basic_usage.rs` - Getting Started
Demonstrates the fundamentals:
- Creating users with password requirements
- Password authentication
- Token refresh workflow
- Password changes
- User listing

**Key concepts**: User lifecycle, authentication basics, token management

### 2. `sip_register_flow.rs` - SIP REGISTER with JWT
Shows how session-core-v2 would handle SIP REGISTER:
- JWT authentication for SIP users
- Bearer token extraction from SIP headers
- Token validation workflow
- Multi-device registration
- 401 Unauthorized handling

**Key concepts**: SIP integration, bearer tokens, multi-device support

### 3. `token_validation.rs` - Auth-Core Integration
Demonstrates how auth-core validates tokens:
- JWT validation with public key
- JWKS endpoint simulation
- Token introspection
- Error handling (expired, tampered tokens)
- Caching strategies

**Key concepts**: Token validation, public key distribution, auth-core integration

### 4. `api_key_service.rs` - API Keys for Services
Shows API key usage for automated systems:
- Creating service accounts
- API key generation with permissions
- Key rotation workflows
- Different permission scopes
- Security best practices

**Key concepts**: Service authentication, API keys, permission management

### 5. `multi_device_presence.rs` - Presence Aggregation
Demonstrates multi-device scenarios:
- Multiple device registrations per user
- Presence state aggregation
- PIDF format hints
- Device-specific presence
- Subscription handling

**Key concepts**: Presence, multi-device support, state aggregation

### 6. `session_core_v2_integration.rs` - Complete Integration
Full integration example showing:
- Users-core + auth-core + session-core-v2 + registrar-core
- Complete SIP REGISTER flow
- Presence subscription
- Configuration setup
- Error handling

**Key concepts**: Full system integration, component interaction

## Integration Patterns

### Authentication Flow
```
User → users-core (login) → JWT → Client → session-core-v2 → auth-core (validate) → ✓
```

### Token Validation
```rust
// 1. Extract bearer token
let token = auth_header.strip_prefix("Bearer ")?;

// 2. Validate via auth-core (using users-core public key)
let user_context = auth_core.validate_token(token).await?;

// 3. Check permissions
if !user_context.scope.contains("sip.register") {
    return Err("Insufficient permissions");
}
```

### Multi-Device Registration
- Each device authenticates separately
- Gets unique JWT token
- Registers with unique contact
- Presence aggregated across all devices

## Key Takeaways

1. **Separation of Concerns**
   - users-core: Issues tokens
   - auth-core: Validates tokens
   - session-core-v2: Uses validated tokens

2. **Security First**
   - RS256 signed JWTs
   - Argon2id password hashing
   - API key hashing
   - Token expiration

3. **Scalability**
   - Connection pooling
   - Token caching
   - Async operations
   - Multi-device support

4. **Standards Compliance**
   - OAuth2 compatible scopes
   - JWT standard claims
   - PIDF for presence
   - SIP RFC compliance

## Common Integration Tasks

### Adding users-core to session-core-v2

1. Initialize users-core:
```rust
let users_config = UsersConfig::default();
let auth_service = users_core::init(users_config).await?;
```

2. Configure auth-core with users-core's public key:
```rust
let public_key = auth_service.jwt_issuer().public_key_pem()?;
auth_core.add_trusted_issuer(TrustedIssuer {
    issuer: "https://users.rvoip.local",
    public_key_pem: public_key,
    audiences: vec!["rvoip-sip"],
});
```

3. Handle SIP REGISTER:
```rust
// Extract token from Authorization header
let token = extract_bearer_token(&sip_request)?;

// Validate via auth-core
let user_context = auth_core.validate_token(token).await?;

// Create registration
registrar.register(user_context, contact).await?;
```

## Troubleshooting

### Common Issues

1. **Token Validation Fails**
   - Check issuer matches configuration
   - Verify audience includes required values
   - Ensure token hasn't expired

2. **Database Connection**
   - SQLite URL must include `?mode=rwc` for creation
   - Check file permissions
   - Ensure parent directory exists

3. **API Key Issues**
   - Keys are one-way hashed
   - Store raw key securely after creation
   - Check expiration dates

### Debug Tips

- Enable tracing: `RUST_LOG=debug cargo run --example <name>`
- Check JWT contents at jwt.io (development only!)
- Verify public key format is PEM
- Test with shorter token TTLs during development
