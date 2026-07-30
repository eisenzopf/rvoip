# rvoip-webrtc-stack 0.3.5 runtime migration

`rvoip-webrtc-stack` now qualifies and exposes Tokio as its only async
integration runtime. The `runtime-smol` feature, `SmolRuntime`,
`smol_runtime()`, and the Smol-specific channel, timer, socket, and
synchronization wrappers have been removed.

Applications that previously selected Smol should use the default feature set
or select Tokio explicitly:

```toml
rvoip-webrtc-stack = { version = "0.3.5", features = ["runtime-tokio"] }
```

Tokio is a mandatory dependency in 0.3.5. The `runtime-tokio` feature is kept
as a source-compatible no-op, so existing Tokio manifests continue to build;
`default-features = false` no longer disables the runtime.

Applications that require a different executor can integrate directly with
`rvoip-rtc`, whose Sans-I/O protocol core remains runtime-independent.
