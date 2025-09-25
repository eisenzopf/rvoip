# Rate Limiting Integration Tests

This directory contains integration tests that verify the rate limiting functionality by starting a real HTTP server and making actual HTTP requests.

## Architecture

The integration test setup consists of:

1. **Test Server** - The actual users-core API server with rate limiting middleware
2. **Test Client** - HTTP client (reqwest) that makes requests to test rate limiting
3. **Test Scenarios** - Various rate limiting scenarios tested through HTTP

## Key Test Scenarios

### 1. Login Attempt Lockout (`test_login_attempt_lockout`)
- Tests that failed login attempts trigger account lockout
- Verifies 429 status code after threshold
- Checks Retry-After header
- Verifies lockout expires after configured duration

### 2. API Request Rate Limiting (`test_api_request_rate_limiting`)
- Tests IP-based rate limiting for unauthenticated requests
- Verifies request count limits
- Tests 429 response when limit exceeded

### 3. Authenticated User Rate Limiting (`test_authenticated_user_rate_limiting`)
- Tests per-user rate limiting with JWT tokens
- Verifies authenticated users have separate limits
- Tests that different users don't share rate limits

### 4. Rate Limit Headers (`test_rate_limit_headers`)
- Verifies proper HTTP headers in rate limit responses
- Checks Retry-After header format and values

### 5. Security Headers (`test_security_headers_present`)
- Verifies all security headers are present
- Tests CORS, CSP, X-Frame-Options, etc.

## Running Tests

### Quick Run
```bash
cargo test --test rate_limiting_integration
```

### With Logging
```bash
RUST_LOG=debug cargo test --test rate_limiting_integration -- --nocapture
```

### Using the Script
```bash
./tests/integration_test/run_integration_tests.sh
```

### Extended Tests
```bash
./tests/integration_test/run_integration_tests.sh --full
```

## Test Configuration

The tests use aggressive rate limits for faster testing:
- 10 requests per minute (instead of default 100)
- 3 login attempts per hour (instead of default 5)
- 2 second lockout duration (instead of default 15 minutes)

## Architecture Decisions

1. **Single Server**: We don't need separate middleware/API servers. The rate limiting is integrated into the API as Axum middleware.

2. **Programmatic Server Start**: The server is started programmatically in the test, not via shell script. This gives better control and cleanup.

3. **Temporary Database**: Each test run uses a fresh SQLite database in a temp directory.

4. **Port Assignment**: Tests use port 0 to let the OS assign an available port, avoiding conflicts.

## Adding New Tests

To add a new rate limiting test:

1. Add a new `#[tokio::test]` function in `rate_limiting_integration.rs`
2. Use the `start_test_server()` helper to get a server instance
3. Make HTTP requests using the reqwest client
4. Assert on response status codes and headers

## Troubleshooting

### Tests Failing with "Connection Refused"
- Ensure no firewall is blocking localhost connections
- Check that the server startup delay is sufficient (currently 100ms)

### Rate Limits Not Triggering
- Verify the test configuration has low enough limits
- Check that requests are being counted (use RUST_LOG=debug)

### Database Errors
- Ensure temp directory creation succeeds
- Check file permissions in the test environment
