# rvoip-vcon

> ⚠️ **Experimental surface** (unified `0.3.x` release) — API-unstable; expect breaking changes before `1.0`.

vCon (Virtualized Conversation) document model, builder, signer, validator, and
store, pinned to `draft-ietf-vcon-vcon-core` working-group commit `2342aba`.
Core Session finalization exposes persisted documents through `VconReady`.
`RecordingComplete.vcon_ref` wiring remains outside the 0.3.3 scope.

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `voip-3`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## Core wire format

New documents include `vcon: "0.4.0"`, a UUIDv8, and `created_at`.
Dialog `duration` is measured in seconds. Participant classification uses the
core `type` property (for example, `person`, `bot`, or `organization`);
conversation roles require a declared extension. The obsolete private `role`
property is not emitted.
Unknown extension properties are preserved during deserialization and declared
through `extensions` and `critical`.

```rust
use chrono::Utc;
use rvoip_vcon::{Party, VconBuilder};

let vcon = VconBuilder::new()
    .with_party(Party {
        name: Some("Alice".into()),
        kind: Some("person".into()), // serializes as "type"
        ..Party::default()
    })
    .recording(Utc::now(), 12.5, vec![0], "audio/opus")
    .build_validated()?;
# Ok::<(), rvoip_vcon::VconError>(())
```

Inline binary bodies use unpadded base64url. External content uses an HTTPS URL
and a `sha512-<base64url-no-padding>` token produced by `content_hash`.

## Signing

Unsigned output is the default. `sign_jws` explicitly creates the JWS General
JSON Serialization required by the draft:

```json
{
  "payload": "<base64url unsigned vCon>",
  "signatures": [{
    "header": { "x5u": "https://keys.example/signer.pem" },
    "protected": "<base64url alg and uuid>",
    "signature": "<base64url signature>"
  }]
}
```

`append_signature` adds another signature over the unchanged payload.
Verification requires a caller-provided trusted key; embedded `x5c` or `x5u`
metadata is never trusted automatically. RS256 is the recommended signing
algorithm for interoperability. HMAC signing is rejected, and JWE encryption
is not implemented.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
