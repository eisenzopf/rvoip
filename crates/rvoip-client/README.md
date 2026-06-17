# rvoip-client

> ⚠️ **Experimental (`0.1.x`)** — the first real path is UCTP over QUIC
> signaling. `Client::connect("uctp+quic://...")` performs the QUIC dial and
> bearer handshake, `Client::call(..., SessionMedium::Voice)` sends
> `session.invite`, and `SessionHandle::end()` sends `session.end`.
> SIP and WebRTC client dispatch are explicit future work.

Client-side SDK for mobile / web / desktop / embedded apps that speak the Universal Conversation Transport Protocol (UCTP). The current milestone is a concrete UCTP QUIC happy path behind one `Client` / `SessionHandle` / `InboundEvent` surface.

```rust
use rvoip_client::{CallTarget, Client, Credential, SessionMedium};

# async fn run() -> rvoip_client::Result<()> {
let client = Client::connect(
    "uctp+quic://thelve.example.com:4433",
    Credential::Bearer("alice-token".into()),
).await?;

let session = client
    .call(CallTarget::Participant("part_bob".into()), SessionMedium::Voice)
    .await?;
session.end().await?;
# Ok(()) }
```

Use `Client::connect_with_options(..., ClientOptions)` for pinned/self-signed QUIC TLS in dev and tests. For production SIP softphones today, continue to use [`rvoip-sip`](https://crates.io/crates/rvoip-sip) directly.

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `client`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
