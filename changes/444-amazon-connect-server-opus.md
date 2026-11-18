---
section: Fixed
breaking: false
---
`rvoip-amazon-connect`'s `server` feature now enables Opus in rvoip-core's
media graph (`rvoip-core/opus`). Before, building only with `server` (as
`examples/13-sip-to-amazon-connect` does) let the media-core transcoder handle
Opus, but rvoip-core refused the Opus stream, so creating the SIP ⟷ Amazon
Connect media bridge failed with `ConnectError { class: Mapping }`.
