# Pitch Analysis Implementation Fix Plan

## Executive Summary

The current Rust implementation of `pitch_ol_fast` in `src/encoder/pitch.rs` is significantly different from the C reference implementation and returns incorrect results (all zeros). This document outlines the issues and provides a plan to fix the implementation while minimizing risk to other tests.

## Current Issues

### 1. Incomplete Algorithm Implementation

The Rust version is missing critical parts of the algorithm:
- **Energy normalization**: The C code computes normalized correlations by dividing by sqrt(energy)
- **Three-section search**: C divides the search into 3 sections (20-39, 40-79, 80-143)
- **Pitch multiple detection**: C includes logic to detect and favor non-multiple pitch periods
- **Small lag favoring**: C includes bias towards smaller lag values

### 2. Variable Shadowing Bug

The Rust code has a critical bug where it tries to update variables but creates new shadowed ones:
```rust
let max1 = -1;
// ...
let _max3 = extract_h(t0);  // Creates new variable, doesn't update max3
```

### 3. Missing Helper Functions

The Rust implementation lacks several critical functions used by the C code:
- `inv_sqrt()` - Inverse square root for normalization (exists in basic_operators.rs)
- `Mpy_32()` - 32-bit multiplication of two Q16.16 numbers (exists as `mpy_32` in oper_32b.rs)
- `L_Extract()` - Extract high/low parts of 32-bit value (exists as `l_extract` in oper_32b.rs)
- `mult()` - Q15 multiplication (exists in basic_operators.rs)

### 4. Algorithm Differences

The Rust implementation:
- Uses a single pass search from pit_max down to PIT_MIN
- Doesn't compute energy-normalized correlations
- Doesn't handle pitch multiples
- Uses incorrect comparison logic with THRESHOLD

## Implementation Plan

### Phase 1: Create a New Implementation File (Low Risk)

To avoid breaking existing code, create a new implementation:

1. **File**: `src/encoder/pitch_ol_fast_g729a.rs`
   - Implement the exact G.729A algorithm
   - Include all three search sections
   - Add energy normalization
   - Add pitch multiple detection

### Phase 2: Import Required Functions

Add imports for existing functions:
```rust
use crate::common::basic_operators::{
    add, sub, shl, shr, abs_s, mult, extract_h, extract_l, l_mac, l_sub, l_add,
    Word16, Word32, MIN_32
};
use crate::common::oper_32b::{l_extract, mpy_32};
```

### Phase 3: Implement Missing Constants

Add G.729A specific constants:
```rust
const L_FRAME: usize = 80;
const PIT_MIN: i32 = 20;
const PIT_MAX: i32 = 143;
```

### Phase 4: Implement the Complete Algorithm

Structure the implementation as follows:

```rust
pub fn pitch_ol_fast_g729a(signal: &[Word16], pit_max: i32, l_frame: i32) -> i32 {
    // Step 1: Signal scaling (same as current)
    // Step 2: First section (20-39)
    // Step 3: Second section (40-79)  
    // Step 4: Third section (80-143) with ±1 refinement
    // Step 5: Multiple detection and adjustment
    // Step 6: Final comparison and selection
}
```

### Phase 5: Detailed Implementation Steps

1. **Signal Scaling**
   - Check for overflow risk by computing sum of squares
   - Scale signal by >>3 if overflow, <<3 if small, or leave unchanged

2. **Section Search (for each of 3 sections)**
   - Compute correlation for each lag in section
   - Find maximum correlation and corresponding lag
   - Compute energy at optimal lag
   - Normalize: `max_norm = correlation / sqrt(energy)`

3. **Third Section Refinement**
   - After finding T3, check T3+1 and T3-1
   - Update if better correlation found

4. **Multiple Detection**
   - Check if T2 ≈ 2*T3 or 3*T3 (pitch multiples)
   - If so, boost max2 by 25% of max3
   - Check if T1 ≈ 2*T2 or 3*T2
   - If so, boost max1 by 20% of max2

5. **Final Selection**
   - Compare max1, max2, max3
   - Return lag with highest normalized correlation
   - Favor smaller lags in case of ties

### Phase 6: Testing Strategy

1. **Create Comprehensive Test Suite**
   - Test with synthetic periodic signals
   - Test with noise
   - Test edge cases (very short/long periods)
   - Compare against C reference outputs

2. **Integration Testing**
   - Initially keep both implementations
   - Add feature flag to switch between them
   - Run full test suite with both versions

3. **Gradual Migration**
   - Once new implementation passes all tests
   - Replace calls to old function one by one
   - Monitor test results after each change

### Phase 7: Code Review Checklist

Before integration, verify:
- [ ] All three sections implemented correctly
- [ ] Energy normalization working
- [ ] Multiple detection logic correct
- [ ] No variable shadowing bugs
- [ ] Proper overflow handling
- [ ] Matches C output for test vectors

## Risk Mitigation

1. **Keep Original Code**: Don't modify existing `pitch_ol_fast` initially
2. **Feature Flag**: Use compile-time flag to switch implementations
3. **Extensive Testing**: Test thoroughly before replacing
4. **Incremental Changes**: Replace usage points one at a time
5. **Benchmark**: Ensure performance is acceptable

## Expected Outcomes

After implementation:
- Pitch analysis should return correct pitch periods (not zeros)
- Output should match C reference implementation
- All existing tests should continue to pass
- Performance should be comparable to C version

## Timeline Estimate

- Phase 1-4: 2-3 hours (core implementation)
- Phase 5: 3-4 hours (testing and debugging)
- Phase 6-7: 2-3 hours (integration and verification)

Total: ~8-10 hours of focused development 