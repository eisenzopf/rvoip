# ACELP Fixed Codebook Search Test

This directory contains a test framework for comparing the C reference implementation of the G.729A ACELP (Algebraic Code-Excited Linear Prediction) fixed codebook search against the Rust implementation.

## Overview

The ACELP fixed codebook search is a key component of the G.729A speech codec. It finds an innovation sequence (fixed-codebook vector) that best matches the target signal after the adaptive codebook contribution has been removed.

## Files

- `ACELP_CA.C` - The C reference implementation of ACELP_Code_A function
- `c_test.c` - C test harness that reads test inputs and outputs results
- `rust_test.rs` - Rust test harness (currently contains placeholder until implementation is complete)
- `generate_test_vectors.c` - Generates random test vectors for testing
- `compare.sh` - Shell script that runs both tests and compares outputs
- `Makefile` - Builds the C test programs
- Supporting C files: `BASIC_OP.C`, `OPER_32B.C`, `TAB_LD8A.C`, `COR_FUNC.C`, `FILTER.C`, `DSPFUNC.C`

## Running the Tests

From the `g729a-new` directory, run:

```bash
./tests/acelp/compare.sh
```

This will:
1. Compile the C test programs
2. Generate test vectors (if not already present)
3. Run the C implementation
4. Run the Rust implementation
5. Compare the outputs and generate comparison reports

## Test Outputs

The test compares:
- **index**: The index of pulse positions (main return value)
- **sign**: Signs of the 4 pulses
- **code[]**: The innovative codebook vector (first 10 values)
- **y[]**: The filtered innovative codebook (first 10 values)

## Results

Results are saved to:
- `c_output.csv` - C implementation outputs
- `rust_output.csv` - Rust implementation outputs
- `comparison.csv` - Detailed comparison of all outputs
- `side_by_side.txt` - Side-by-side comparison of first 5 tests

## Implementation Status

**Note**: As of creation, the Rust ACELP implementation (`src/encoder/acelp_codebook.rs`) is not yet implemented. The test framework is ready and will work once the implementation is added. The placeholder in `rust_test.rs` should be replaced with the actual import once available:

```rust
// TODO: Once implemented, replace with:
use g729a_new::encoder::acelp_codebook::acelp_code_a;
```

### Known Issues

1. **C Reference Implementation**: The ACELP_Code_A function from the G.729A reference code causes an infinite loop when called directly. This might be due to:
   - Missing initialization of global state that the function depends on
   - Dependencies on other G.729 components not being properly set up
   - Issues introduced during the round() to g729_round() function renaming
   
   The c_test.c currently uses dummy values to verify the test framework works. Once the issue is resolved, uncomment the actual ACELP_Code_A call in c_test.c.

2. **File Name Case Sensitivity**: The C reference files use uppercase names (e.g., TYPEDEF.H) which can cause warnings on case-sensitive file systems. 