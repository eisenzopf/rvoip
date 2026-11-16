# Cisco Voice Virtual Agent protobuf contract (pins only)

`rvoip-webex-byova`'s optional `grpc` feature compiles Cisco's gRPC contract
for Webex Contact Center BYOVA: `voicevirtualagent.proto` and
`byova_common.proto` from <https://github.com/webex/dataSourceSchemas>, commit
`fca090dfa806e28695acb145e927ad0d557ce7fe`, directory
`Services/VoiceVirtualAgent/5397013b-7920-4ffc-807c-e8a3e0a18f43/Proto`.

Cisco's repository has no licence, so rvoip does not redistribute these files:
they are neither committed nor packaged. This directory holds only their
SHA-256 pins (`SHA256SUMS`, verified byte for byte against that commit on
2026-11-15).

- In the workspace, `python3 scripts/ci/fetch_webex_byova_protos.py` downloads both files at the pinned commit into
  the ignored `target/third_party/cisco-webex-byova-proto/`, verifies the pins,
  and fails closed on a mismatch. The build looks there.
- Anywhere else, download the two files yourself, check them against
  `SHA256SUMS`, and set `RVOIP_WEBEX_BYOVA_PROTO_DIR` to their directory.
- CI (`scripts/ci/run_checks.py`, `scripts/release/gates.py` and the
  `webex-byova.yml` workflow) runs the fetch before every command that builds
  the `grpc` feature, including every `--all-features` workspace build.

Changing the pinned revision means updating `SHA256SUMS`, `COMMIT` in the
fetch script, and the build script's message together.
