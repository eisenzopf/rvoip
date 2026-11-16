# Cisco Voice Virtual Agent protobuf contract (unpublished input)

These two unmodified files are Cisco's gRPC contract for Webex Contact Center
BYOVA. They come from <https://github.com/webex/dataSourceSchemas>, commit
`fca090dfa806e28695acb145e927ad0d557ce7fe`, directory
`Services/VoiceVirtualAgent/5397013b-7920-4ffc-807c-e8a3e0a18f43/Proto`
(retrieved 2026-10-28; re-verified byte for byte against that commit on
2026-11-15). `SHA256SUMS` pins them.

Cisco's repository has no top-level licence, so these files are **not part of
any published package**. `rvoip-webex-byova` is published without them: its
optional `grpc` feature compiles the contract at build time from the directory
named by `RVOIP_WEBEX_BYOVA_PROTO_DIR`. Inside this workspace checkout the build
falls back to this directory, so workspace builds, tests and CI need no setup.
Outside the workspace, fetch the files yourself:

```sh
mkdir -p webex-byova-proto && cd webex-byova-proto
base=https://raw.githubusercontent.com/webex/dataSourceSchemas/fca090dfa806e28695acb145e927ad0d557ce7fe/Services/VoiceVirtualAgent/5397013b-7920-4ffc-807c-e8a3e0a18f43/Proto
curl -fsSLO "$base/voicevirtualagent.proto"
curl -fsSLO "$base/byova_common.proto"
sha256sum -c /path/to/rvoip/third_party/cisco-webex-byova-proto/SHA256SUMS
export RVOIP_WEBEX_BYOVA_PROTO_DIR="$PWD"
```

Keep the package, service names, field numbers and enum values unchanged.
Builds use a vendored `protoc` (`protoc-bin-vendored`); no system `protoc` is
needed.
