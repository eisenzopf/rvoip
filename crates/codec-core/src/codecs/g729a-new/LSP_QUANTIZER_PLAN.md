# Proposal and Plan for LSP Quantization Implementation

This document outlines the plan to implement the LSP Quantization stage (Step 3) of the G.729A encoder, ensuring it is a bit-exact, algorithmically identical port of the C reference code.

## 1. Proposal

The goal is to implement the LSP Quantization stage as described in the encoder process. This involves creating a new set of functions within `src/encoder/lsp_quantizer.rs` that are a direct port of the C reference code found in `code/QUA_LSP.C`.

The implementation will include:
1.  Porting the large, static Vector Quantizer (VQ) codebook tables (`lspcb1`, `lspcb2`, `fg`, etc.) from `code/TAB_LD8A.C` into the Rust module.
2.  Creating a main public function, `lsp_quantize`, which will serve as the entry point for this stage, equivalent to `Qua_lsp` in the C code.
3.  Implementing all necessary helper functions to support the quantization logic, ensuring every mathematical operation uses the verified bit-exact basic operators from `src/common/basic_operators.rs`.
4.  Adding a new test file in the `tests/` directory to verify the correctness of the quantization output against the C reference.

The primary goal is **100% algorithmic identity** with the C code to ensure interoperability and correctness.

---

## 2. Plan & Pseudocode

The implementation will be broken down into the following steps, all within the `src/encoder/lsp_quantizer.rs` file unless otherwise specified.

### Step 2.1: Port Codebook Tables

First, add the necessary VQ codebook tables and prediction coefficients to `lsp_quantizer.rs` as `const` arrays.

```rust
// In src/encoder/lsp_quantizer.rs

// MA prediction coefficients
const MA_NP: usize = 4;
const PRED: [Word16; MA_NP] = [ 5571, 4751, 2785, 1556 ];

// VQ Codebook 1 (lspcb1)
const NC0: usize = 128; // 2^7
const LSPCB1: [[Word16; 10]; NC0] = [
    // ... 128 rows of 10 columns from TAB_LD8A.C ...
];

// VQ Codebook 2 (lspcb2)
const NC1: usize = 32; // 2^5
const LSPCB2: [[Word16; 10]; NC1] = [
    // ... 32 rows of 10 columns from TAB_LD8A.C ...
];

// Frequency-dependent weighting factors (fg)
const FG: [[[Word16; 10]; MA_NP]; 2] = [
    // ... data from TAB_LD8A.C ...
];

// ... and so on for fg_sum, fg_sum_inv ...
```

### Step 2.2: Implement Helper Functions

Next, implement the helper functions that the main quantization function will rely on.

**Function: `get_weights`** (from `Get_wegt` in C)
*   **Purpose:** Calculates perceptual weighting factors based on the spacing of the input LSP vector. Closer LSPs get higher weights.
*   **Pseudocode:**
    ```rust
    fn get_weights(lsp: &[Word16]) -> [Word16; 10] {
        let mut weights = [0; 10];
        let mut tmp;

        // Calculate distance between adjacent LSPs
        // weights[i] = 1.0 / (lsp[i+1] - lsp[i-1]) with scaling and saturation
        for i in 1..9 {
            let d1 = lsp[i] - lsp[i-1];
            let d2 = lsp[i+1] - lsp[i];
            let d_sum = d1 + d2;
            
            // Invert the sum using fixed-point division
            // Apply scaling factors from the spec
            weights[i] = calculated_weight;
        }
        // Handle edge cases for weights[0] and weights[9]
        
        return weights;
    }
    ```

### Step 2.3: Implement the Main Quantization Function

This is the core of the task, porting `Qua_lsp`. It will be structured as a public struct `LspQuantizer` to hold the state (memory) required between frames.

```rust
// In src/encoder/lsp_quantizer.rs

pub struct LspQuantizer {
    // Memory of past quantized LSPs (lsp_prev[MA_NP][M])
    lsp_memory: [[Word16; 10]; MA_NP],
    // Memory of past MA predictor selections (prev_ma)
    prev_ma: Word16,
}

impl LspQuantizer {
    pub fn new() -> Self { /* ... initialize memories ... */ }

    // Main public function
    pub fn quantize(
        &mut self,
        unquantized_lsp: &[Word16; 10], // Input: lsp[] from az_lsp
        quantized_lsp: &mut [Word16; 10], // Output: lspq[]
        indices: &mut [i32; 2] // Output: ana[]
    ) {
        // 1. Get the MA prediction residual
        //    - Use `self.lsp_memory` and `PRED` coefficients to predict the current LSP
        //    - Subtract prediction from `unquantized_lsp` to get the residual `res[]`

        // 2. Calculate perceptual weights for the residual
        let weights = get_weights(unquantized_lsp);

        // 3. First Stage VQ Search
        //    - Search through `LSPCB1` codebook (128 entries)
        //    - Find the entry that minimizes the weighted distance to the residual `res[]`
        //    - Store the best index in `indices[0]`
        let best_cand_stage1 = find_best_vector(&res, &LSPCB1, &weights);
        indices[0] = best_cand_stage1.index;

        // 4. Second Stage Split-VQ Search
        //    - Update the residual by subtracting the best vector from stage 1
        //    - Split the updated residual into two 5-dimensional vectors
        //    - Search the first half in the first part of `LSPCB2`
        //    - Search the second half in the second part of `LSPCB2`
        //    - Store the two best indices combined into `indices[1]`
        let (index1, index2) = find_best_split_vectors(&res_stage2, &LSPCB2, &weights);
        indices[1] = (index1 << 5) + index2;

        // 5. Reconstruct the quantized LSP vector
        //    - Add the selected codebook vectors from both stages to the prediction
        //    - Store the result in `quantized_lsp`
        
        // 6. Update state for the next frame
        //    - Update `self.lsp_memory` with the newly quantized LSP vector
        //    - Update `self.prev_ma`
    }
}
```

### Step 2.4: Create Verification Test

A new test file `tests/lsp_quantizer.rs` will be created with a test named `test_lsp_quantize`. This test will:
1.  Define a known input `a[]` array (LPC coefficients).
2.  Call the existing `az_lsp` to get the unquantized LSPs.
3.  Call the new `lsp_quantizer::quantize` function.
4.  Compare the output `quantized_lsp` and `indices` against known good values generated by running the same inputs through the C reference executable. This follows the same pattern used to debug `az_lsp`.
