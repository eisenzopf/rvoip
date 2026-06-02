# rvoip-vcon

> ⚠️ **Alpha** (`0.1.x`) — early and API-unstable; expect breaking changes before `1.0`.

vCon (Virtualized Conversation) document builder + store per the IETF vCon WG draft. Builds + signs the recording artifacts UCTP / SIP / WebRTC adapters reference via Event::RecordingComplete.vcon_ref.

Part of the [**rvoip**](https://github.com/eisenzopf/rvoip) workspace (the "rvoip 3"
unified real-time-communications stack). Published so the
[`rvoip`](https://crates.io/crates/rvoip) facade can expose it behind the `voip-3`
feature — see the [workspace README](https://github.com/eisenzopf/rvoip) and
`docs/INTERFACE_DESIGN.md` for how it fits into the architecture.

## License

Licensed under the MIT License — see [LICENSE](https://github.com/eisenzopf/rvoip/blob/main/LICENSE).
