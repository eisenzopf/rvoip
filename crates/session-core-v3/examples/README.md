# Examples

Every example runs standalone with a single command. No second terminal needed.

```
cargo run --example <name>
```

For verbose logging, add `RUST_LOG=rvoip_session_core_v3=debug` before the command.

## Getting Started

| Command | Description |
|---------|-------------|
| `cargo run --example hello` | Make and receive a SIP call between two peers |

## StreamPeer (sequential / client-side)

Use `StreamPeer` for clients, scripts, and test tools. Call methods, await results.

| Command | Description |
|---------|-------------|
| `cargo run --example streampeer_audio` | Bidirectional audio exchange with WAV output |
| `cargo run --example streampeer_dtmf` | Send DTMF digits during a call |
| `cargo run --example streampeer_hold_resume` | Put a call on hold and resume it |
| `cargo run --example streampeer_registration` | Register with a SIP registrar server |
| `cargo run --example streampeer_blind_transfer` | Three-party blind transfer (REFER) |

## CallbackPeer (reactive / server-side)

Use `CallbackPeer` for servers, proxies, and IVR systems. Implement the `CallHandler` trait or use a built-in handler.

| Command | Description |
|---------|-------------|
| `cargo run --example callbackpeer_auto_answer` | Auto-answer every call (simplest server) |
| `cargo run --example callbackpeer_closure` | Closure-based handler, no trait needed |
| `cargo run --example callbackpeer_routing` | Route calls by URI pattern matching |
| `cargo run --example callbackpeer_ivr` | IVR menu with DTMF navigation |
| `cargo run --example callbackpeer_queue` | Call center queue with deferred accept |
| `cargo run --example callbackpeer_custom` | Full `CallHandler` trait (all 5 methods) |

## Advanced

| Command | Description |
|---------|-------------|
| `cargo run --example advanced_concurrent_calls` | 5 concurrent callers + 1 answerer |
| `cargo run --example advanced_registrar_server` | Registrar server with digest auth (standalone, use with `streampeer_registration`) |
