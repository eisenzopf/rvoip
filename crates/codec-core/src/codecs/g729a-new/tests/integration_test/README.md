# G.729A Encoder Integration Test

This directory contains an integration test that compares the C reference encoder with the Rust encoder implementation of the G.729A codec using the official ITU-T test vectors.

**Note: Only encoder testing is currently supported since the Rust decoder is not yet implemented.**

## Structure

- `c_test.c` - C test program that uses the reference implementation
- `rust_test_main.rs` - Rust test program entry point
- `Makefile` - Builds the C test executable
- `Cargo.toml` - Rust project configuration for the test
- `compare.sh` - Main test script that runs both implementations and compares outputs
- `*.C`, `*.H` - C reference implementation files (copied from `../code/`)

## Test Vectors

The test uses the official G.729A test vectors from `../test_vectors/`:

- `SPEECH.IN/PST` - General speech test
- `ALGTHM.IN/PST` - Algorithmic test covering conditional parts
- `FIXED.IN/PST` - Fixed codebook search test
- `LSP.IN/PST` - LSP quantization test
- `PITCH.IN/PST` - Pitch search test
- `TAME.IN/PST` - Taming procedure test

Each test vector consists of:
- `.IN` files - Input speech data (16-bit PCM, 8kHz)
- `.PST` files - Expected output from reference decoder
- `.BIT` files - Expected bitstream output from reference encoder

## Running the Tests

### Prerequisites

- GCC compiler for C code
- Rust/Cargo for Rust code
- Unix-like environment with bash

### Quick Start

```bash
cd integration_test
./compare.sh
```

This script will:
1. Build both C and Rust encoder implementations
2. Run encoding tests on all test vectors
3. Compare bitstream outputs between C and Rust encoders
4. Compare outputs against reference bitstream files
5. Generate detailed comparison files if differences are found

### Manual Testing

You can also run the implementations manually:

```bash
# Build C implementation
make c_test

# Build Rust implementation  
cargo build --release

# Test C encoder
./c_test encode ../test_vectors/SPEECH.IN output_c.bit

# Test Rust encoder
./target/release/rust_test encode ../test_vectors/SPEECH.IN output_rust.bit

# Compare encoder outputs
cmp output_c.bit output_rust.bit

# Compare with reference bitstream
cmp output_c.bit ../test_vectors/SPEECH.BIT
```

## Current Status

⚠️ **Note**: The Rust encoder implementation is currently incomplete and serves as a placeholder. The current Rust test:

- Implements only basic pre-processing (high-pass filtering)
- Uses placeholder values for most encoder parameters
- Does not perform actual G.729A encoding

As the Rust encoder implementation is completed, this test will provide bit-exact validation against the C reference.

## Expected Results

Currently, the tests will **fail** because:
1. The Rust encoder produces placeholder parameters, not real G.729A encoding
2. The encoder logic is incomplete and doesn't match the C reference

Once the Rust encoder implementation is complete, the tests should:
- ✅ Produce identical bitstreams (C vs Rust encoding)
- ✅ Match reference bitstream outputs from test vectors
- ✅ Pass all encoder conformance tests

## Output Files

Test results are saved in `./output/`:
- `./output/c/` - C implementation outputs
- `./output/rust/` - Rust implementation outputs
- `./output/*_hex` - Hexdump files for detailed comparison when outputs differ

## Troubleshooting

### Compilation Issues

If C compilation fails:
```bash
# Check if all C files are present
ls *.C *.H

# Try building manually
gcc -O2 -Wall *.C c_test.c -o c_test -lm
```

If Rust compilation fails:
```bash
# Check Cargo.toml path
cargo check

# Build with verbose output
cargo build --release --verbose
```

### Runtime Issues

If test files are missing:
```bash
# Verify test vectors exist
ls ../test_vectors/*.IN ../test_vectors/*.PST
```

If executables don't run:
```bash
# Check permissions
chmod +x c_test
chmod +x target/release/rust_test
``` 