# G.729 Annex A Encoder: Detailed Algorithmic Steps

The encoder processes 16-bit PCM audio sampled at 8 kHz, operating on 10 ms frames (80 samples).

---
**A. Frame-Based Operations (Performed once per 10 ms frame)**
---

**1. Pre-processing (`src/encoder/pre_proc.rs`)**
*   **Scaling:** The input speech frame is scaled by a factor of 0.5 (divided by 2) to prevent overflows in the fixed-point arithmetic used in subsequent steps.
*   **High-Pass Filtering:** The scaled signal is passed through a 2nd-order pole-zero high-pass filter with a cutoff frequency of 140 Hz. This removes unwanted low-frequency components and DC offset. The filter is defined by the transfer function `Hh1(z)`.

**2. Linear Prediction (LP) Analysis (`src/encoder/lpc.rs`)**
*   **Windowing:** A 30 ms (240 samples) asymmetric window is applied to the pre-processed speech signal. This window uses 120 past samples, the 80 samples of the current frame, and 40 samples of look-ahead from the next frame.
*   **Autocorrelation:** The first 11 autocorrelation coefficients, `r(k)`, are calculated from the windowed speech using the **Autocorrelation Method**.
*   **Bandwidth Expansion:** The autocorrelation values are slightly expanded by multiplying them with a decaying window (`wlag`). This is known as bandwidth expansion (60 Hz), which helps ensure the stability of the resulting LP filter. A white noise correction factor is also applied to `r(0)`.
*   **LP Coefficient Calculation:** The 10 LP coefficients (`ai`) are computed from the modified autocorrelation coefficients using the **Levinson-Durbin algorithm**.

**3. LP to LSP Conversion and Quantization (`src/encoder/lsp_quantizer.rs`)**
*   **Conversion:** The 10 LP coefficients are converted into 10 Line Spectral Pair (LSP) coefficients. LSPs have better quantization and interpolation properties than direct LP coefficients. This involves finding the roots of two polynomials, `F'1(z)` and `F'2(z)`, derived from the LP filter polynomial `A(z)`.
*   **Quantization:** The LSPs are quantized using a predictive, two-stage Vector Quantizer (VQ) with 18 bits.
    *   A 4th-order switched Moving-Average (MA) predictor is used to predict the current LSPs from previous frames.
    *   The prediction residual (the difference between actual and predicted LSPs) is quantized.
    *   **Stage 1:** A 7-bit, 10-dimensional VQ is used.
    *   **Stage 2:** A 10-bit split-VQ is used (two 5-bit, 5-dimensional VQs).
    *   **Annex A Simplification:** The search for the optimal codebook entries is simplified to reduce computation. It performs a less exhaustive search compared to the full G.729 specification.

**4. Perceptual Weighting Filter Calculation (`src/encoder/perceptual_weighting.rs`)**
*   A perceptual weighting filter, `W(z) = A(z/γ1) / A(z/γ2)`, is computed from the unquantized LP coefficients. This filter is used to shape the quantization error so it is less audible.
*   **Annex A Simplification:** The filter coefficients `γ1` and `γ2` are calculated only **once per 10 ms frame** based on the spectral characteristics of that frame. They are not interpolated for each subframe.

**5. Open-Loop Pitch Analysis (`src/encoder/pitch.rs`)**
*   An initial estimate of the pitch period (`Top`) is calculated once per frame to guide the more detailed search later.
*   **Annex A Simplification:** To reduce complexity, this analysis is performed on a **decimated (2:1 downsampled)** version of the weighted speech signal. The search finds the maximum normalized correlation in three overlapping delay ranges to identify the most likely pitch period.

---
**B. Subframe-Based Operations (Performed twice per frame, for each 5 ms subframe)**
---

**6. Target Signal and Impulse Response Calculation (`src/encoder/target.rs`)**
*   For each subframe, the LP and LSP coefficients are interpolated from the values of the previous and current frames.
*   **Impulse Response:** The impulse response `h(n)` of the weighted synthesis filter `W(z)/Â(z)` is computed. `Â(z)` is the quantized LP filter.
*   **Target Signal:** The LP residual `r(n)` is calculated by filtering the speech signal through the LP analysis filter `A(z)`. The target signal `x(n)` for the codebook searches is then computed by filtering this residual through the weighted synthesis filter `W(z)/Â(z)`.

**7. Adaptive-Codebook Search (Pitch Synthesis) (`src/encoder/pitch.rs`)**
*   This step refines the pitch estimate using a **closed-loop, analysis-by-synthesis search**.
*   The search finds the pitch delay `T` and gain `gp` that minimize the mean squared error between the target signal `x(n)` and the filtered past excitation. The past excitation is the "adaptive codebook."
*   The search is performed around the `Top` estimated in the open-loop analysis.
*   A **fractional pitch delay** with 1/3 resolution is used to accurately model non-integer pitch periods. This is found by upsampling and interpolating the correlation signal.
*   The resulting pitch delay is encoded with 8 bits for the first subframe and a 5-bit relative delay for the second.

**8. Fixed-Codebook Search (ACELP) (`src/encoder/acelp_codebook.rs`)**
*   The target signal is updated by subtracting the adaptive codebook's contribution (the synthesized pitch component).
*   The **Algebraic Code-Excited Linear Prediction (ACELP)** codebook is searched to find an innovation sequence (fixed-codebook vector `c(n)`) that best matches the updated target.
*   The codebook vector `c(n)` consists of 4 non-zero pulses. The search finds the optimal positions and signs (+1/-1) for these pulses.
*   **Annex A Simplification:** The search algorithm is significantly simplified. It avoids the computationally intensive calculation of the `d'H H d` term (correlation matrix) used in the original G.729. Instead, it uses a simpler, non-iterative approach that leverages the algebraic structure of the codebook to find a good excitation vector with much lower complexity.
*   The resulting pulse positions and signs are encoded with 17 bits.

**9. Gain Quantization (`src/encoder/gain_quantizer.rs`)**
*   The adaptive-codebook gain `gp` and the fixed-codebook gain `gc` are jointly quantized using a 7-bit vector quantizer with MA prediction on the fixed-codebook gain. The search finds the codebook entry that minimizes the error between the target signal and the sum of the scaled adaptive and fixed codebook contributions.

**10. Memory Update**
*   The final excitation for the subframe is constructed by summing the scaled adaptive and fixed-codebook vectors.
*   The memories of the LP synthesis filter `1/Â(z)` and the weighting filter `W(z)` are updated by filtering this final excitation. This ensures correct filter states for the start of the next subframe.
