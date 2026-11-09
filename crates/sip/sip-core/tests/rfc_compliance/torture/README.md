# SIP torture test messages

Bit-exact test messages from:

- `rfc4475/`: RFC 4475, "Session Initiation Protocol (SIP) Torture Test
  Messages", Appendix A.1 (49 messages described in Section 3, plus
  `test.dat`, which is in the archive but not described in the RFC).
- `rfc5118/`: RFC 5118, "Session Initiation Protocol (SIP) Torture Test
  Messages for Internet Protocol Version 6 (IPv6)", Appendix A.1 (12
  messages). The archive has no file extensions; `.dat` is added here.

They were extracted from the RFC texts at https://www.rfc-editor.org with the
procedure each Appendix A gives (base64-decode the block between
`-- BEGIN MESSAGE ARCHIVE --` and `-- END MESSAGE ARCHIVE --`, then gunzip and
untar), and are unmodified. `SHA256SUMS` pins every file and is checked by
`fixtures_are_the_bit_exact_rfc_archives`.

As published, the RFC 5118 messages use LF line endings, `ipv6-bug-abnf-3-colons`
and `ipv6-correct-abnf-2-colons` have no empty line ending the header section,
and `ipv6-in-sdp` and `mult-ip-in-sdp` declare a Content-Length larger than
their body. The suite applies RFC 3261 to the files as published and tests a
normalized copy (built in the test, not stored here) for what RFC 5118
describes. See `../torture_test.rs`.
