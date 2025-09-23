# Users-Core Creation Summary

## What Was Done

### 1. Created users-core Library
- Created new crate at `/crates/users-core/`
- Set up proper Cargo.toml with all necessary dependencies
- Created module structure for clean separation of concerns

### 2. Established Architecture
- **Separation of Concerns**: 
  - users-core handles user storage, password auth, and JWT issuance
  - auth-core validates tokens from all sources (users-core, OAuth2, etc.)
- **Hybrid Approach**: Local user management with JWT that integrates with auth-core

### 3. Created Implementation Plan
- Comprehensive plan in `crates/users-core/IMPLEMENTATION_PLAN.md`
- Covers all aspects: user management, authentication, JWT issuance, API keys
- Includes database schema, REST API design, and security considerations

### 4. Updated auth-core Plan
- Modified `crates/auth-core/AUTHENTICATION_PLAN.md` to reflect new architecture
- auth-core now focuses on token validation rather than user management
- Clear integration points between users-core and auth-core

### 5. Module Structure Created
```
users-core/
├── src/
│   ├── lib.rs          # Main entry point
│   ├── error.rs        # Error types
│   ├── types.rs        # Core types (User, etc.)
│   ├── auth/           # Authentication service
│   ├── user_store/     # User storage (SQLite)
│   ├── api_keys/       # API key management
│   ├── jwt/            # JWT token issuance
│   ├── api/            # REST API endpoints
│   └── config/         # Configuration
├── Cargo.toml          # Dependencies configured
├── README.md           # Documentation
└── IMPLEMENTATION_PLAN.md  # Detailed plan
```

## Key Design Decisions

1. **SQLite for Storage**: Lightweight, embedded database perfect for user management
2. **Argon2 for Passwords**: Industry-standard password hashing
3. **RS256 JWT**: Asymmetric signing allows auth-core to validate without sharing secrets
4. **Unified Store**: SqliteUserStore implements both UserStore and ApiKeyStore traits

## Integration with Existing System

### For REGISTER/Presence in session-core-v2:
1. Users authenticate with users-core to get JWT
2. JWT is included in SIP REGISTER as Bearer token
3. session-core-v2 uses auth-core to validate the JWT
4. auth-core validates using users-core's public key

### Token Flow:
```
User → users-core (login) → JWT → Client → session-core-v2 → auth-core (validate) → ✓
```

## Next Steps

### Implementation Priority:
1. **Phase 1**: Implement core database operations in SqliteUserStore
2. **Phase 2**: Implement JWT issuance with proper RS256 signing
3. **Phase 3**: Implement password authentication with Argon2
4. **Phase 4**: Create REST API endpoints
5. **Phase 5**: Add JWKS endpoint for auth-core integration
6. **Phase 6**: Testing and security hardening

### To Start Implementation:
```bash
cd crates/users-core
# Implement SqliteUserStore first
# Then JWT issuer
# Then authentication service
# Finally REST API
```

## Benefits of This Approach

1. **Clean Separation**: auth-core remains focused on validation
2. **Flexibility**: Can add OAuth2 users alongside internal users
3. **Production Ready**: SQLite scales well for user management
4. **Security**: Industry-standard crypto throughout
5. **Integration**: Works seamlessly with existing RVoIP architecture

## Configuration Example

```toml
# users-core configuration
[users_core]
database_url = "sqlite://users.db"
api_bind_address = "127.0.0.1:8081"

[users_core.jwt]
issuer = "https://users.rvoip.local"
audience = ["rvoip-api", "rvoip-sip"]

# auth-core configuration
[auth.trusted_issuers.users_core]
issuer = "https://users.rvoip.local"
jwks_uri = "http://localhost:8081/auth/jwks.json"
```

This approach provides a solid foundation for user management in RVoIP while maintaining the flexibility to integrate external authentication providers through auth-core.
