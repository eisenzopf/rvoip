# Plan for Perceptual Weighting Filter Calculation (Step 4)

This document outlines the plan to implement the Perceptual Weighting Filter calculation for the G.729A encoder, as per the user's request.

## 1. Analysis

-   **Goal**: Implement the perceptual weighting stage described in `G729A_ENCODER_STEPS.md`.
-   **Reference**: The implementation will be bit-exact with the reference C code provided in the `code/` directory.
-   **Core Function**: The C code in `LPCFUNC.C` contains the function `Weight_Az` which performs the spectral expansion: `ap[i] = a[i] * (gamma ** i)`.
-   **Annex A Simplification**: The `COD_LD8A.C` file shows that for Annex A, this weighting is applied with fixed constants. The `G729.txt` (Annex A) specifies `gamma1 = 0.94` and `gamma2 = 0.6`. The C code uses `GAMMA1` for the numerator of the weighting filter (`Ap_t`). The denominator part using `GAMMA2` is handled differently or simplified in the subsequent steps of the encoder.
-   **Rust Implementation**: The existing Rust code in `src/encoder/perceptual_weighting.rs` is incorrect and will be replaced. The new implementation will follow the logic of the C `Weight_Az` function.

## 2. Implementation Steps

### Step 2.1: Update Rust Source Code

-   **File**: `src/encoder/perceptual_weighting.rs`
-   **Action**:
    1.  Modify the `perceptual_weighting` function to correctly calculate the weighted LPC coefficients.
    2.  The function will compute `p` using `GAMMA1` (0.94) and `f` using `GAMMA2` (0.6), matching the function signature.
    3.  The calculation for each coefficient `ap[i]` will be `round(L_mult(a[i], fac))`, where `fac` is `gamma` raised to the power of `i`.
    4.  Correct the existing unit test within the file to reflect the correct behavior (e.g., `p[0]` and `f[0]` should be `a[0]`, which is 4096).

### Step 2.2: Create Test Infrastructure

A new test suite will be created in `tests/perceptual_weighting/` to perform a black-box comparison between the reference C code and the new Rust implementation.

1.  **Create Directory**:
    ```bash
    mkdir tests/perceptual_weighting
    ```

2.  **Create `c_test.c`**:
    -   **File**: `tests/perceptual_weighting/c_test.c`
    -   **Purpose**: A C program that reads test input vectors, calls the reference `Weight_Az` function from `LPCFUNC.C`, and prints the output. It will be called twice (once for `GAMMA1`, once for `GAMMA2`) to generate the equivalent of `p` and `f`.

3.  **Create `Makefile`**:
    -   **File**: `tests/perceptual_weighting/Makefile`
    -   **Purpose**: To compile `c_test.c` along with the necessary C source files from the `code/` directory (`LPCFUNC.C`, `BASIC_OP.C`, `OPER_32B.C`).

4.  **Create `rust_test.rs`**:
    -   **File**: `tests/perceptual_weighting/rust_test.rs`
    -   **Purpose**: A Rust integration test that reads the same test input vectors, calls the `perceptual_weighting` function, and prints the output in the identical format as `c_test`.

5.  **Create `compare.sh`**:
    -   **File**: `tests/perceptual_weighting/compare.sh`
    -   **Purpose**: A shell script to automate the testing process:
        -   Build the C test using the `Makefile`.
        -   Run the C test and redirect output to `c_output.txt`.
        -   Run the Rust test using `cargo test` and redirect output to `rust_output.txt`.
        -   Compare `c_output.txt` and `rust_output.txt` using `diff`.
        -   Report success or failure.

## 3. Execution

I will now proceed with the implementation based on this plan.
