# REST API Demo

This demo provides a comprehensive test of all users-core REST API endpoints.

## Structure

- `server.rs` - A minimal REST API server
- `client.rs` - A test client that exercises all endpoints
- `run_demo.sh` - Shell script that orchestrates the demo
- `users_core.toml` - Sample configuration file (for reference)

## Configuration

The demo server uses hardcoded configuration values for simplicity. A sample `users_core.toml` file is included to show all available configuration options. In production, you would typically use environment variables or a configuration file.

## Running the Demo

From the users-core directory:

```bash
./examples/rest_api_demo/run_demo.sh
```

## What the Demo Tests

1. **Health Check** - Verifies server is running
2. **User Creation** - Creates admin and regular users
3. **Authentication** - Login with username/password
4. **User Management**
   - List users
   - Get user details
   - Update user details (email, display name)
   - Update user roles
   - Change password
   - Delete user
5. **API Key Management**
   - Create API key
   - Authenticate with API key
   - List API keys
   - Revoke API key
6. **Token Management**
   - Token refresh
   - Logout
7. **Security Features**
   - Password validation
   - Role-based access control
   - Admin-only endpoints
8. **Utility Endpoints**
   - JWKS endpoint for public keys
   - Metrics endpoint

## Expected Output

The demo will:
1. Start a server on port 8082
2. Create a fresh SQLite database
3. Run through all API tests
4. Report success/failure for each test
5. Shut down the server automatically

## Troubleshooting

If the demo fails:
- Check that port 8082 is available
- Look at `server.log` for server errors
- Ensure you have the required SQLite version (3.35.0+ for IF NOT EXISTS on indices)

## Using as Integration Test

This demo can also serve as an integration test:

```bash
# Run in CI/CD
./examples/rest_api_demo/run_demo.sh || exit 1
```

The script returns exit code 0 on success, 1 on failure.
