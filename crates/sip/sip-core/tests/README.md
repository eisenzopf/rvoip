# SIP Core Test Suite

Integration tests for `rvoip-sip-core`. `lib.rs` collects the `parser/` and
`rfc_compliance/` modules; the other top-level files are separate test targets.

## Running

```bash
cargo test -p rvoip-sip-core --tests
# RFC 4475 / RFC 5118 torture tests only
cargo test -p rvoip-sip-core --test lib rfc_compliance::torture_test
```

## Torture tests (`rfc_compliance/torture_test.rs`)

The fixtures in `rfc_compliance/torture/` are the bit-exact messages from the
Appendix A archives of RFC 4475 (SIP Torture Test Messages) and RFC 5118 (SIP
Torture Test Messages for IPv6); see `rfc_compliance/torture/README.md`.
`SHA256SUMS` pins them and a test checks it, so they are never edited. Each
case records the RFC section its expectation comes from.

Every message is parsed in both parse modes:

- **Strict** accepts only messages that conform to the RFC 3261 grammar (with
  the RFC 5954 and RFC 8217 updates). It never infers or repairs anything, and
  rejects octets after the message (one SIP message per WebSocket message,
  RFC 7118 §5).
- **Lenient** is the datagram and stream path. It discards octets after the
  message (RFC 3261 §18.3) and applies only the liberal handling the RFCs
  explicitly allow for a given message (for example,
  ignoring `<>` around a Request-URI or extra SP in the start line), and the
  parsed message no longer contains the error. Where the RFC says a message
  must be rejected, Lenient rejects it too.

Messages that are syntactically valid but wrong at the transaction or
application layer (`badvers`, `mismatch01`, `insuf`, `bext01`, ...) parse in
both modes; the response an element must send is the job of the layers above.

Every accepted message must also re-serialize to bytes that parse in Strict
mode, and serializing again must produce the same bytes: whatever rvoip sends
after accepting a torture message is valid SIP.

The RFC 5118 archive uses LF line endings, two messages have no empty line
after the headers, and two declare a Content-Length longer than their body.
Each RFC 5118 message is checked as published (RFC 3261 applied to the bytes:
Strict rejects the LF framing, Lenient tolerates it, and a body shorter than
Content-Length is an error) and normalized to CRLF with a correct
Content-Length, which tests what RFC 5118 describes.

## Adding tests

Add messages that are not from an RFC archive as ordinary tests next to the
parser code or in `parser/`; keep `rfc_compliance/torture/` for the RFC
archives.
