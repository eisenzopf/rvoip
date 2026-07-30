# libSRTP interoperability gate

Run the release gate from the repository root:

```sh
scripts/test_libsrtp_interop.sh
```

The script fetches Cisco libSRTP 2.8.0 at the immutable commit
`24b3bf8f19b6f5ab4cd2bcceb4f4064efca86fd5`, verifies that checkout, builds
it with its bundled AES-CM/HMAC implementation, and compiles the C driver in
this directory. It then exchanges libSRTP's own deterministic AES-CM-128
validation packets for both HMAC-SHA1-80 and HMAC-SHA1-32 in all four
directions:

- rvoip SRTP protect -> libSRTP unprotect
- libSRTP SRTP protect -> rvoip unprotect
- rvoip SRTCP protect -> libSRTP unprotect
- libSRTP SRTCP protect -> rvoip unprotect
- rvoip SRTP `65535 -> 0` rollover -> libSRTP unprotect
- libSRTP SRTP `65535 -> 0` rollover -> rvoip unprotect

Both protect paths must also match libSRTP's published known-answer bytes
exactly. The SHA1-32 profile is verified with a four-byte SRTP tag and the
required ten-byte SRTCP tag. The ordinary Rust test suite repeats those
known-answer checks in `tests/srtp_libsrtp_known_answers.rs` without requiring
a C toolchain or network access.

For an already-fetched source tree, set `RVOIP_LIBSRTP_SOURCE_DIR`. The script
still rejects it unless `HEAD` is exactly the pinned commit.
