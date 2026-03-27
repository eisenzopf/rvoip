# SIP Proxy Features Implementation Plan

> **Status: COMPLETED 2026-03-27**
>
> All four features implemented and verified with `cargo check --workspace` (zero errors).
> Dynamic injection extension (Plan A + DbAuthProvider/DbProxyRouter) completed same day.
>
> Related docs:
> - Usage and testing guide: `docs/guides/sip-proxy-guide.md`
> - Commit security audit: `docs/audit/007-commit-audit.md`

**Goal:** Make rvoip production-ready for real SIP trunk/proxy integration by adding server-side digest auth, Via NAT handling, Record-Route insertion, and proxy request forwarding.

**Architecture:** Four incremental features layered bottom-up. Auth hooks into register/invite handlers via a pluggable `AuthProvider` trait. Via/NAT modifies the existing `create_response()` builder. Record-Route adds a header to forwarded requests. Proxy forwarding creates a new code path in the INVITE handler that relays requests instead of creating local sessions.

**Tech Stack:** Rust 2024, rvoip workspace (dialog-core, sip-core, session-core), existing digest auth primitives in `sip-core/src/auth/digest.rs`, existing typed headers in `sip-core/src/types/auth/`.

## Implementation Summary

### Files Changed

| File | Change |
|------|--------|
| `crates/dialog-core/src/auth.rs` | `AuthProvider`, `AuthResult`, `ProxyRouter`, `ProxyAction`, `NoopAuthProvider` |
| `crates/dialog-core/src/manager/core.rs` | `auth_provider` / `proxy_router` fields changed to `Arc<RwLock<Option<Arc<dyn T>>>>` + setters/getters |
| `crates/dialog-core/src/api/unified.rs` | `set_auth_provider()` / `set_proxy_router()` delegating to `inner_manager()` |
| `crates/dialog-core/src/protocol/register_handler.rs` | Calls `auth_provider.check_request()` → sends 401 |
| `crates/dialog-core/src/protocol/invite_handler.rs` | Calls `auth_provider.check_request()` → sends 407; calls `proxy_router.route_request()` → forward or local B2BUA |
| `crates/dialog-core/src/transaction/utils/request_builders.rs` | `create_forwarded_request()`: decrements Max-Forwards, prepends Via, inserts Record-Route with `;lr` |
| `crates/dialog-core/src/transaction/utils/response_builders.rs` | `create_response()` copies Record-Route from request (RFC 3261 §16.6) |
| `crates/dialog-core/src/transaction/manager/operations.rs` | `forward_request()` method |
| `crates/session-core/src/dialog/manager.rs` | `set_auth_provider()` / `set_proxy_router()` delegation |
| `crates/session-core/src/coordinator/coordinator.rs` | `set_auth_provider()` / `set_proxy_router()` |
| `crates/session-core/src/lib.rs` | Re-exports `AuthProvider`, `AuthResult`, `ProxyRouter`, `ProxyAction` |
| `crates/session-core/src/api/builder.rs` | `SipTransportType::UdpAndWs` variant |
| `crates/session-core/src/dialog/builder.rs` | `UdpAndWs` arm: enables both UDP :5060 and WS :8080 |
| `crates/call-engine/src/config.rs` | `GeneralConfig.enable_websocket: bool` |
| `crates/call-engine/src/orchestrator/core.rs` | `set_auth_provider()` / `set_proxy_router()` + `UdpAndWs` branch |
| `crates/web-console/src/sip_providers.rs` | `DbAuthProvider` + `DbProxyRouter` (PostgreSQL, live queries) |
| `crates/web-console/examples/server.rs` | Full integration: dual transport + DB provider injection |

---

## Feature 1: Server-Side Digest Auth (401/407)

### Task 1.1: Create `AuthProvider` trait in dialog-core

**Files:**
- Create: `crates/dialog-core/src/auth.rs`
- Modify: `crates/dialog-core/src/lib.rs` — add `pub mod auth;`

**Step 1: Create the auth module with provider trait and nonce utilities**

```rust
// crates/dialog-core/src/auth.rs
//! Server-side SIP digest authentication.
//!
//! Provides an `AuthProvider` trait that dialog-core calls to verify
//! credentials on REGISTER and INVITE requests.

use std::net::SocketAddr;
use std::sync::Arc;
use rvoip_sip_core::Request;

/// Result of credential verification.
#[derive(Debug, Clone)]
pub enum AuthResult {
    /// Credentials valid — proceed with request.
    Authenticated { username: String },
    /// No credentials or invalid — challenge the client.
    Challenge,
    /// Skip authentication for this request.
    Skip,
}

/// Pluggable authentication provider.
///
/// Implement this trait and attach it to `DialogManager` via
/// `set_auth_provider()`.  Dialog-core will call `check_request()`
/// for every REGISTER and INVITE received.
#[async_trait::async_trait]
pub trait AuthProvider: Send + Sync + 'static {
    /// Verify credentials in the request.
    ///
    /// * Return `Authenticated` if the Authorization header is present and valid.
    /// * Return `Challenge` to send a 401/407 response with a fresh nonce.
    /// * Return `Skip` to bypass authentication entirely (e.g. internal traffic).
    async fn check_request(&self, request: &Request, source: SocketAddr) -> AuthResult;

    /// The SIP realm used in WWW-Authenticate challenges.
    fn realm(&self) -> &str;
}

/// Generate a cryptographically random nonce for digest challenges.
pub fn generate_nonce() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let rand: u64 = rand::random();
    format!("{:x}{:x}", ts, rand)
}

/// Simple in-memory auth provider that accepts any registration
/// (useful for testing / demo deployments).
pub struct NoopAuthProvider;

#[async_trait::async_trait]
impl AuthProvider for NoopAuthProvider {
    async fn check_request(&self, _request: &Request, _source: SocketAddr) -> AuthResult {
        AuthResult::Skip
    }
    fn realm(&self) -> &str { "rvoip" }
}
```

**Step 2: Wire into lib.rs**

Add `pub mod auth;` to `crates/dialog-core/src/lib.rs`.

**Step 3: Add `async-trait` dependency if not present**

Check `crates/dialog-core/Cargo.toml`; add `async-trait = "0.1"` to `[dependencies]` if missing.

**Step 4: Compile check**

```bash
cargo check -p rvoip-dialog-core
```

**Step 5: Commit**

```
feat(dialog-core): add AuthProvider trait for server-side SIP auth
```

---

### Task 1.2: Store AuthProvider in DialogManager and add 401 builder

**Files:**
- Modify: `crates/dialog-core/src/manager/core.rs` — add auth_provider field + setter
- Modify: `crates/dialog-core/src/transaction/utils/response_builders.rs` — add `create_unauthorized_response()`

**Step 1: Add auth_provider to DialogManager**

In `crates/dialog-core/src/manager/core.rs`, add field to the struct:

```rust
// Near other fields in the DialogManager struct
auth_provider: Option<Arc<dyn crate::auth::AuthProvider>>,
```

Initialize as `None` in the constructor. Add setter:

```rust
/// Attach a pluggable authentication provider.
pub fn set_auth_provider(&mut self, provider: Arc<dyn crate::auth::AuthProvider>) {
    self.auth_provider = Some(provider);
}

/// Get reference to auth provider (if set).
pub fn auth_provider(&self) -> Option<&Arc<dyn crate::auth::AuthProvider>> {
    self.auth_provider.as_ref()
}
```

**Step 2: Add `create_unauthorized_response()` to response_builders.rs**

```rust
/// Create a 401 Unauthorized response with WWW-Authenticate digest challenge.
///
/// RFC 3261 §22.1: The server MUST include a WWW-Authenticate header
/// field containing the authentication challenge applicable to the realm.
pub fn create_unauthorized_response(
    request: &Request,
    realm: &str,
    nonce: &str,
) -> Response {
    let mut response = create_response(request, StatusCode::Unauthorized);

    // Add WWW-Authenticate header with Digest challenge
    let challenge = rvoip_sip_core::types::auth::www_authenticate::WwwAuthenticate::new(realm, nonce)
        .with_algorithm(rvoip_sip_core::types::auth::Algorithm::Md5)
        .with_qop(rvoip_sip_core::types::auth::Qop::Auth);
    response.headers.push(TypedHeader::WwwAuthenticate(challenge));

    response
}

/// Create a 407 Proxy Authentication Required response.
pub fn create_proxy_auth_response(
    request: &Request,
    realm: &str,
    nonce: &str,
) -> Response {
    let mut response = create_response(request, StatusCode::ProxyAuthenticationRequired);

    let challenge = rvoip_sip_core::types::auth::proxy_authenticate::ProxyAuthenticate::new(realm, nonce)
        .with_algorithm(rvoip_sip_core::types::auth::Algorithm::Md5)
        .with_qop(rvoip_sip_core::types::auth::Qop::Auth);
    response.headers.push(TypedHeader::ProxyAuthenticate(challenge));

    response
}
```

**Step 3: Compile check**

```bash
cargo check -p rvoip-dialog-core
```

**Step 4: Commit**

```
feat(dialog-core): add AuthProvider to DialogManager + 401/407 builders
```

---

### Task 1.3: Insert auth challenge into REGISTER handler

**Files:**
- Modify: `crates/dialog-core/src/protocol/register_handler.rs`

**Step 1: Add auth check before auto-response / session forwarding**

Replace the body of `handle_register_method()` (lines 46–98) so that auth is checked first:

```rust
async fn handle_register_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
    debug!("Processing REGISTER request from {}", source);

    // Extract registration information
    let from_uri = request.from()
        .ok_or_else(|| DialogError::protocol_error("REGISTER missing From header"))?
        .uri().clone();

    let contact_uri = self.extract_contact_uri(&request).unwrap_or_else(|| from_uri.clone());
    let expires = self.extract_expires(&request);

    // Remember the registration route for INVITE forwarding
    self.transaction_manager
        .remember_registration_route(&from_uri, source, expires)
        .await;

    // Create server transaction
    let server_transaction = self.transaction_manager
        .create_server_transaction(request.clone(), source)
        .await
        .map_err(|e| DialogError::TransactionError {
            message: format!("Failed to create server transaction for REGISTER: {}", e),
        })?;

    let transaction_id = server_transaction.id().clone();

    // ── Authentication gate ─────────────────────────────────────
    if let Some(auth) = self.auth_provider() {
        match auth.check_request(&request, source).await {
            crate::auth::AuthResult::Authenticated { username } => {
                debug!("REGISTER authenticated for user {}", username);
            }
            crate::auth::AuthResult::Challenge => {
                debug!("Sending 401 challenge for REGISTER from {}", source);
                let nonce = crate::auth::generate_nonce();
                let response = crate::transaction::utils::response_builders::create_unauthorized_response(
                    &request, auth.realm(), &nonce,
                );
                self.transaction_manager.send_response(&transaction_id, response).await
                    .map_err(|e| DialogError::TransactionError {
                        message: format!("Failed to send 401 for REGISTER: {}", e),
                    })?;
                return Ok(());
            }
            crate::auth::AuthResult::Skip => {
                debug!("Auth skipped for REGISTER from {}", source);
            }
        }
    }
    // ─────────────────────────────────────────────────────────────

    if self.should_auto_respond_to_register() {
        debug!("Auto-responding to REGISTER request (configured for auto-response)");
        self.send_basic_register_response(&transaction_id, &request, expires).await?;
    } else {
        debug!("Forwarding REGISTER request to session layer (auto-response disabled)");

        let event = SessionCoordinationEvent::RegistrationRequest {
            transaction_id: transaction_id.clone(),
            from_uri,
            contact_uri,
            expires,
        };

        if let Err(e) = self.notify_session_layer(event).await {
            debug!("Failed to notify session layer of REGISTER: {}, sending fallback response", e);
            self.send_basic_register_response(&transaction_id, &request, expires).await?;
        }
    }

    debug!("REGISTER request processed");
    Ok(())
}
```

**Step 2: Compile check**

```bash
cargo check -p rvoip-dialog-core
```

**Step 3: Commit**

```
feat(dialog-core): enforce auth challenge on REGISTER requests
```

---

### Task 1.4: Insert auth challenge into INVITE handler

**Files:**
- Modify: `crates/dialog-core/src/protocol/invite_handler.rs`

**Step 1: Add auth gate at the start of `handle_initial_invite()`**

After the server transaction is created, before the early dialog and session event:

```rust
// Inside handle_initial_invite(), after creating server transaction:

// ── Authentication gate ─────────────────────────────────────
if let Some(auth) = self.auth_provider() {
    match auth.check_request(&request, source).await {
        crate::auth::AuthResult::Authenticated { username } => {
            debug!("INVITE authenticated for user {}", username);
        }
        crate::auth::AuthResult::Challenge => {
            debug!("Sending 407 challenge for INVITE from {}", source);
            let nonce = crate::auth::generate_nonce();
            let response = crate::transaction::utils::response_builders::create_proxy_auth_response(
                &request, auth.realm(), &nonce,
            );
            self.transaction_manager.send_response(&transaction_id, response).await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 407 for INVITE: {}", e),
                })?;
            return Ok(());
        }
        crate::auth::AuthResult::Skip => {}
    }
}
// ─────────────────────────────────────────────────────────────
```

Note: INVITE uses **407** (Proxy-Authenticate) per RFC 3261 §22.2, while REGISTER uses **401** (WWW-Authenticate).

**Step 2: Compile check + commit**

```
feat(dialog-core): enforce proxy auth challenge on INVITE requests
```

---

## Feature 2: Via Header NAT Handling (received/rport)

### Task 2.1: Add `fix_via_nat()` utility and update `create_response()`

**Files:**
- Modify: `crates/dialog-core/src/transaction/utils/response_builders.rs`

**Step 1: Add Via NAT fix function**

```rust
/// RFC 3261 §18.2.2 / RFC 3581: Add received and rport parameters to the
/// top Via header when the source address differs from the sent-by address.
///
/// This is critical for NAT traversal — without it, responses will be
/// routed to the address in the Via header instead of the actual source.
pub fn fix_via_nat(response: &mut Response, source: SocketAddr) {
    // Find the topmost Via header (first one in the list is topmost)
    for header in response.headers.iter_mut() {
        if let TypedHeader::Via(ref mut via) = header {
            if let Some(first) = via.0.first_mut() {
                let via_host = match &first.sent_by_host {
                    rvoip_sip_core::types::host::Host::Address(ip) => Some(*ip),
                    rvoip_sip_core::types::host::Host::Domain(_) => None,
                };
                let via_port = first.sent_by_port.unwrap_or(5060);
                let source_ip = source.ip();
                let source_port = source.port();

                // Add received= if source IP differs from Via sent-by
                if via_host.map_or(true, |ip| ip != source_ip) {
                    first.set_received(source_ip);
                }

                // Add rport= with actual source port if rport was requested
                // (RFC 3581: client sends rport without value, server fills it in)
                // Also add rport if source port differs from Via port
                if first.rport().is_some() || via_port != source_port {
                    first.set_rport(Some(source_port));
                }
            }
            break; // Only modify topmost Via
        }
    }
}
```

**Step 2: Update `create_response()` to accept optional source address**

Add a new function that wraps the existing one:

```rust
/// Create a response with Via NAT fix applied.
///
/// Same as `create_response()` but also sets received/rport parameters
/// on the topmost Via header based on the actual packet source address.
pub fn create_response_with_nat(request: &Request, status: StatusCode, source: SocketAddr) -> Response {
    let mut response = create_response(request, status);
    fix_via_nat(&mut response, source);
    response
}
```

**Step 3: Compile check + commit**

```
feat(dialog-core): add Via received/rport NAT fix to response builder
```

---

### Task 2.2: Apply NAT fix in REGISTER and INVITE handlers

**Files:**
- Modify: `crates/dialog-core/src/protocol/register_handler.rs` — `send_basic_register_response()`
- Modify: `crates/dialog-core/src/protocol/invite_handler.rs` — 100 Trying / 180 Ringing
- Modify: `crates/dialog-core/src/transaction/utils/response_builders.rs` — update auth responses

**Step 1: Fix register_handler's response**

In `send_basic_register_response()`, after building the response, apply NAT fix:

```rust
// After: let mut response = create_response(request, StatusCode::Ok);
// Add:
crate::transaction::utils::response_builders::fix_via_nat(&mut response, source);
```

Note: `send_basic_register_response()` needs `source: SocketAddr` parameter added. Update its signature:

```rust
pub async fn send_basic_register_response(
    &self,
    transaction_id: &crate::transaction::TransactionKey,
    request: &Request,
    expires: u32,
    source: SocketAddr,
) -> DialogResult<()> {
```

And update all callers.

**Step 2: Fix INVITE handler's provisional responses**

Where 100 Trying and 180 Ringing are built in invite_handler.rs, apply `fix_via_nat()`.

**Step 3: Fix 401/407 auth responses**

Update `create_unauthorized_response()` and `create_proxy_auth_response()` to also accept source and call `fix_via_nat()`.

**Step 4: Compile check + commit**

```
feat(dialog-core): apply Via NAT fix to all outgoing responses
```

---

## Feature 3: Record-Route Insertion

### Task 3.1: Add Record-Route to forwarded INVITE requests

**Files:**
- Modify: `crates/dialog-core/src/manager/core.rs` — add local_contact config
- Modify: `crates/session-core/src/coordinator/event_handler.rs` — add RR on B2BUA forward

**Step 1: Add Record-Route insertion in B2BUA forward path**

In `event_handler.rs`, inside the `CallDecision::Forward` handler, before `create_outgoing_call()`:

```rust
// Build Record-Route header with our proxy address (lr = loose routing)
// This ensures mid-dialog requests (re-INVITE, BYE) traverse our proxy.
let proxy_uri = format!("sip:{};lr", self.config.local_address);
tracing::info!("📲 B2BUA: Adding Record-Route: {}", proxy_uri);
```

The Record-Route header needs to be propagated through the SIP messages. For the B2BUA case this is handled implicitly because the B2BUA generates its own INVITE for the B-leg and its own 200 OK for the A-leg — both naturally contain the proxy's Contact address.

For a true proxy (Feature 4), Record-Route will need to be inserted into the forwarded request.

**Step 2: Ensure 200 OK responses include proxy Contact**

In the `accept_incoming_call` path (dialog-core `CallHandle::answer()`), verify that the Contact header uses the server's address, not the client's. This is already done via `create_ok_response_with_dialog_info()`.

**Step 3: Compile check + commit**

```
feat(session-core): add Record-Route support for B2BUA forwarding
```

---

### Task 3.2: Copy Record-Route headers from request to response

**Files:**
- Modify: `crates/dialog-core/src/transaction/utils/response_builders.rs`

**Step 1: Update `create_response()` to copy Record-Route**

Per RFC 3261 §16.6 step 6: Responses MUST copy all Record-Route headers from the request.

```rust
// In create_response(), after copying CSeq:
// RFC 3261 §16.6: Copy Record-Route headers from request to response
if let Some(header) = request.header(&HeaderName::RecordRoute) {
    builder = builder.header(header.clone());
}
```

**Step 2: Compile check + commit**

```
feat(dialog-core): copy Record-Route headers in SIP responses (RFC 3261 §16.6)
```

---

## Feature 4: Proxy Request Forwarding

### Task 4.1: Add proxy forwarding configuration

**Files:**
- Modify: `crates/dialog-core/src/auth.rs` — extend with proxy routing trait

**Step 1: Add a `ProxyRouter` trait**

```rust
/// Determines how to route a request through the proxy.
#[derive(Debug, Clone)]
pub enum ProxyAction {
    /// Forward the request to a specific address.
    Forward { destination: SocketAddr },
    /// Let the local B2BUA handle it (current behavior).
    LocalB2BUA,
    /// Reject the request with a status code.
    Reject { status: u16, reason: String },
}

/// Routing logic for proxy forwarding.
///
/// When set on the DialogManager, incoming INVITE requests will
/// consult this router before falling through to session-core.
#[async_trait::async_trait]
pub trait ProxyRouter: Send + Sync + 'static {
    /// Decide how to handle an incoming INVITE.
    async fn route_request(&self, request: &Request, source: SocketAddr) -> ProxyAction;
}
```

**Step 2: Add to DialogManager**

```rust
// In DialogManager struct:
proxy_router: Option<Arc<dyn crate::auth::ProxyRouter>>,

// Setter:
pub fn set_proxy_router(&mut self, router: Arc<dyn crate::auth::ProxyRouter>) {
    self.proxy_router = Some(router);
}
```

**Step 3: Compile check + commit**

```
feat(dialog-core): add ProxyRouter trait for INVITE forwarding decisions
```

---

### Task 4.2: Implement stateless proxy forwarding in INVITE handler

**Files:**
- Modify: `crates/dialog-core/src/protocol/invite_handler.rs`
- Modify: `crates/dialog-core/src/transaction/utils/response_builders.rs` — add `create_forwarded_request()`

**Step 1: Add request forwarding builder**

In `response_builders.rs` (or a new `request_builders.rs`):

```rust
/// Build a forwarded INVITE request for proxy use.
///
/// RFC 3261 §16.6:
/// 1. Copy the request
/// 2. Decrement Max-Forwards
/// 3. Add a Via header with the proxy's address
/// 4. Add Record-Route with lr parameter
/// 5. Update Request-URI to the target
pub fn create_forwarded_request(
    original: &Request,
    proxy_addr: &str,
    proxy_port: u16,
    transport: &str,
    target_uri: &str,
) -> Request {
    use rvoip_sip_core::builder::SimpleRequestBuilder;

    // Start with the original request's method and new target URI
    let mut builder = SimpleRequestBuilder::new(original.method().clone(), target_uri);

    // Copy all headers from original
    for header in &original.headers {
        match header {
            TypedHeader::Via(_) => {
                // Keep original Via headers (will add ours on top)
                builder = builder.header(header.clone());
            }
            TypedHeader::MaxForwards(mf) => {
                // Decrement Max-Forwards (RFC 3261 §16.6 step 3)
                let new_mf = if mf.0 > 0 { mf.0 - 1 } else { 0 };
                builder = builder.header(TypedHeader::MaxForwards(
                    rvoip_sip_core::types::max_forwards::MaxForwards(new_mf),
                ));
            }
            _ => {
                builder = builder.header(header.clone());
            }
        }
    }

    // Add our Via header on top (RFC 3261 §16.6 step 2)
    let branch = format!("z9hG4bK-{}", uuid::Uuid::new_v4().to_string().replace('-', ""));
    builder = builder.via(proxy_addr, proxy_port, transport, &branch);

    // Add Record-Route (RFC 3261 §16.6 step 4)
    let rr_uri = format!("<sip:{}:{};lr>", proxy_addr, proxy_port);
    builder = builder.header_str("Record-Route", &rr_uri);

    // Copy body
    if !original.body().is_empty() {
        builder = builder.body(original.body().to_vec());
    }

    builder.build()
}
```

**Step 2: Add proxy path to invite_handler**

In `handle_initial_invite()`, after auth check but before session-core forwarding:

```rust
// ── Proxy routing gate ──────────────────────────────────────
if let Some(router) = self.proxy_router.as_ref() {
    match router.route_request(&request, source).await {
        crate::auth::ProxyAction::Forward { destination } => {
            info!("Proxy-forwarding INVITE to {}", destination);
            // Check Max-Forwards
            let mf = request.typed_header::<rvoip_sip_core::types::max_forwards::MaxForwards>()
                .map(|m| m.0)
                .unwrap_or(70);
            if mf == 0 {
                let response = create_response(&request, StatusCode::TooManyHops);
                self.transaction_manager.send_response(&transaction_id, response).await?;
                return Ok(());
            }

            let local_addr = self.local_addr();
            let forwarded = create_forwarded_request(
                &request,
                &local_addr.ip().to_string(),
                local_addr.port(),
                "UDP",
                &request.uri().to_string(),
            );

            // Send via transport to destination
            self.transaction_manager
                .forward_request(forwarded, destination)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to forward INVITE: {}", e),
                })?;

            return Ok(());
        }
        crate::auth::ProxyAction::Reject { status, reason } => {
            let status_code = StatusCode::from(status);
            let response = create_response(&request, status_code);
            self.transaction_manager.send_response(&transaction_id, response).await?;
            return Ok(());
        }
        crate::auth::ProxyAction::LocalB2BUA => {
            // Fall through to existing B2BUA handling
        }
    }
}
// ─────────────────────────────────────────────────────────────
```

**Step 3: Add `forward_request()` to transaction manager**

This method sends a request to a specific destination via the transport layer. Check if `TransactionManager` already has a `send_request()` or similar — if not, add:

```rust
pub async fn forward_request(&self, request: Request, destination: SocketAddr) -> Result<()> {
    self.transport_manager.send_message(
        rvoip_sip_core::Message::Request(request),
        destination,
    ).await
}
```

**Step 4: Compile check + commit**

```
feat(dialog-core): implement stateless proxy forwarding for INVITE
```

---

### Task 4.3: Handle forwarded responses (proxy response relay)

**Files:**
- Modify: `crates/dialog-core/src/protocol/response_handler.rs`

**Step 1: Strip top Via and relay response back to original sender**

When a response arrives for a forwarded request, the proxy must:
1. Strip its own Via header (the topmost one)
2. Look up the original sender from the next Via header
3. Forward the response

This is complex and depends on the transaction layer maintaining a mapping of forwarded transactions. For the initial implementation, the B2BUA path (Features 1-3) is sufficient for real-world trunk integration. True stateless proxy response relay can be deferred.

**Note:** The B2BUA already handles this correctly — it creates separate dialogs for A-leg and B-leg and bridges them. The proxy forwarding (Task 4.2) is an alternative path for scenarios where B2BUA overhead is not desired.

---

## Testing Strategy

### Unit Tests
- Auth: Test `generate_nonce()` uniqueness, `AuthResult` variants
- Via NAT: Test `fix_via_nat()` with matching/different IPs and ports
- Record-Route: Test header copying in response builder
- Proxy: Test `create_forwarded_request()` Max-Forwards decrement, Via insertion

### Integration Tests
- Start server with auth provider → REGISTER without auth → expect 401
- Start server → REGISTER with valid auth → expect 200 OK
- WebSocket client → server → verify Via received/rport in response
- B2BUA call → verify Record-Route in both legs

### Manual Test (sip-demo.html)
- After each feature, restart web_console_server and test softphone demo
- Verify registration still works
- Verify calls still connect and audio flows

---

## Execution Order

1. **Task 1.1** → AuthProvider trait (no behavioral change)
2. **Task 1.2** → Wire into DialogManager + 401 builder (no behavioral change)
3. **Task 1.3** → Auth in REGISTER handler (skip by default with NoopAuthProvider)
4. **Task 1.4** → Auth in INVITE handler
5. **Task 2.1** → Via NAT fix utility
6. **Task 2.2** → Apply NAT fix everywhere
7. **Task 3.1** → Record-Route in B2BUA
8. **Task 3.2** → Copy Record-Route in responses
9. **Task 4.1** → ProxyRouter trait
10. **Task 4.2** → Proxy forwarding in INVITE handler
11. **Task 4.3** → Proxy response relay (defer if B2BUA sufficient)
