# g729

Pure Rust ITU-T G.729AB codec crate (Annex A + Annex B), with bit-exact conformance tooling and `no_std` support when default features are disabled.

## Features

| Feature flag | Default | Description |
|---|---|---|
| `std` | yes | Standard library support (required for binaries) |
| `annex_b` | yes | VAD / DTX / comfort-noise behavior |
| `itu_serial` | no | ITU serial framing helpers and parameter encode/decode APIs (`std` implied) |

Use `--no-default-features` for `no_std` builds.

## Core API

```rust
use g729::{DecoderConfig, EncoderConfig, FrameType, G729Decoder, G729Encoder};

let mut encoder = G729Encoder::new(EncoderConfig::default());
let mut decoder = G729Decoder::new(DecoderConfig::default());

let pcm_in = [0i16; 80];
let mut bitstream = [0u8; 10];
let frame_type = encoder.encode(&pcm_in, &mut bitstream);
assert_eq!(frame_type, FrameType::Speech);

let mut pcm_out = [0i16; 80];
decoder.decode(&bitstream, &mut pcm_out);
```

## Config Ergonomics

`G729Config` is a shared convenience config and converts into both runtime configs:

```rust
use g729::{G729Config, G729Decoder, G729Encoder};

let cfg = G729Config { annex_b: true };
let mut encoder = G729Encoder::new(cfg);
let mut decoder = G729Decoder::new(cfg);
```

## Strict vs Tolerant Decode

- `decode(&[u8], &mut [i16; 80])` is tolerant: unknown payload lengths are treated as erasures.
- `decode_frame(&[u8]) -> Result<[i16; 80], CodecError>` is strict: invalid lengths return `CodecError::InvalidBitstreamLength`.

## Frame Types

`FrameType` values:
- `Speech` (10 bytes / 80 bits)
- `Sid` (2 bytes / 15 bits)
- `NoData` (0 bytes / 0 bits)

Helpers:
- `FrameType::byte_len()`
- `FrameType::bit_len()`

## ITU Parameter APIs (`itu_serial`)

When `itu_serial` is enabled:
- `G729Encoder::encode_parm(&[i16; 80], &mut [i16]) -> Result<(FrameType, usize), CodecError>`
- `G729Decoder::decode_parm(&mut [i16], &mut [i16; 80]) -> Result<(), CodecError>`
- `g729::bitstream::itu_serial::{read_serial_frame, write_serial_frame}`

## CLI Binaries

Build and run:

```bash
cargo run --features std --bin g729-cli -- help
cargo run --features std --bin g729-cli -- encode input.pcm output.g729
cargo run --features std --bin g729-cli -- decode output.g729 output.pcm
```

ITU serial commands require `itu_serial`:

```bash
cargo run --features "std,itu_serial" --bin g729-cli -- itu-encode input.pcm output.bit 1
cargo run --features "std,itu_serial" --bin g729-cli -- itu-decode output.bit output.pcm
cargo run --features "std,itu_serial" --bin g729-cli -- test-vectors vector.bit expected.pcm annex-a
```

Standalone ITU binaries:

```bash
cargo run --features "std,itu_serial" --bin encoder -- input.pcm output.bit 1
cargo run --features "std,itu_serial" --bin decoder -- output.bit output.pcm
```

## Build and Verify

```bash
cargo test --lib --bins --tests --features "std,annex_b,itu_serial"
cargo clippy --all-targets --features "std,annex_b,itu_serial" -- -D warnings
cargo check --all-targets --no-default-features
cargo bench --bench codec --bench dsp_ops --features "std,annex_b"
```

## License

Dual licensed under either MIT or Apache-2.0 at your option.
See `../LICENSE-MIT` and `../LICENSE-APACHE`.
