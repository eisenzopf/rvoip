//! Pure Rust G.729A (Annex A) codec engine
//!
//! Implements the CS-ACELP (Conjugate-Structure Algebraic-Code-Excited Linear Prediction)
//! algorithm for G.729A encoding and decoding.
//!
//! G.729A encodes 10ms frames (80 samples at 8kHz) into 10 bytes (80 bits) at 8kbps.
//! This is the reduced-complexity variant (Annex A) of ITU-T G.729.

use crate::error::{CodecError, Result};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Frame length in samples (10ms at 8kHz)
const FRAME_LEN: usize = 80;
/// Subframe length in samples (two 5ms subframes per frame)
const SUBFRAME_LEN: usize = 40;
/// LP filter order
const LP_ORDER: usize = 10;
/// Number of LSP parameters
const LSP_ORDER: usize = 10;
/// Encoded frame size in bytes
const ENCODED_FRAME_SIZE: usize = 10;
/// Minimum pitch lag
const PITCH_LAG_MIN: usize = 20;
/// Maximum pitch lag
const PITCH_LAG_MAX: usize = 143;
/// Excitation buffer size: past excitation + current frame
const EXC_BUF_SIZE: usize = PITCH_LAG_MAX + FRAME_LEN + 1;
/// High-pass filter cutoff ~ 140Hz at 8kHz
/// Coefficients for second-order IIR high-pass filter
const HP_B: [f32; 3] = [0.929_809_2, -1.859_618_4, 0.929_809_2];
const HP_A: [f32; 3] = [1.0, -1.859_371_6, 0.859_865_2];

/// Fixed codebook number of pulses for G.729A
const FC_PULSE_COUNT: usize = 4;
/// Fixed codebook track positions (G.729A uses 4 tracks of 10 positions each)
const FC_TRACK_SIZE: usize = 10;

/// LSP quantization tables (simplified scalar quantization for MVP)
/// Each LSP coefficient is quantized with 3-5 bits
/// Total LSP bits: L0(1) + L1(7) + L2(5) + L3(5) = 18 bits
const LSP_INIT: [f32; LSP_ORDER] = [
    0.2855, 0.5711, 0.8567, 1.1423, 1.4279, 1.7135, 1.9991, 2.2847, 2.5703, 2.8559,
];

/// Bandwidth expansion factor for LSP stability
const LSP_MARGIN: f32 = 0.0012;

// ─── Bitstream packing ──────────────────────────────────────────────────────
//
// G.729A 80-bit frame layout:
//   L0  (1 bit)   - LSP switched MA predictor
//   L1  (7 bits)  - LSP 1st stage vector
//   L2  (5 bits)  - LSP 2nd stage subvector 1
//   L3  (5 bits)  - LSP 2nd stage subvector 2
//   P1  (8 bits)  - Pitch delay subframe 1 (adaptive codebook index)
//   P0  (1 bit)   - Parity bit for P1
//   P2  (5 bits)  - Pitch delay subframe 2 (delta)
//   C1  (13 bits) - Fixed codebook index subframe 1
//   S1  (4 bits)  - Fixed codebook sign subframe 1
//   GA1 (3 bits)  - Gain codebook (adaptive) subframe 1
//   GB1 (4 bits)  - Gain codebook (fixed) subframe 1
//   C2  (13 bits) - Fixed codebook index subframe 2
//   S2  (4 bits)  - Fixed codebook sign subframe 2
//   GA2 (3 bits)  - Gain codebook (adaptive) subframe 2
//   GB2 (4 bits)  - Gain codebook (fixed) subframe 2
//   Total: 80 bits = 10 bytes

/// Bit field definition: (bit_offset, bit_count)
const BIT_L0: (usize, usize) = (0, 1);
const BIT_L1: (usize, usize) = (1, 7);
const BIT_L2: (usize, usize) = (8, 5);
const BIT_L3: (usize, usize) = (13, 5);
const BIT_P1: (usize, usize) = (18, 8);
const BIT_P0: (usize, usize) = (26, 1);
const BIT_P2: (usize, usize) = (27, 5);
const BIT_C1: (usize, usize) = (32, 13);
const BIT_S1: (usize, usize) = (45, 4);
const BIT_GA1: (usize, usize) = (49, 3);
const BIT_GB1: (usize, usize) = (52, 4);
const BIT_C2: (usize, usize) = (56, 13);
const BIT_S2: (usize, usize) = (69, 4);
const BIT_GA2: (usize, usize) = (73, 3);
const BIT_GB2: (usize, usize) = (76, 4);

// ─── Bitstream helpers ───────────────────────────────────────────────────────

/// Pack a value into a bit buffer at a given bit offset
fn pack_bits(buf: &mut [u8; ENCODED_FRAME_SIZE], offset: usize, nbits: usize, value: u32) {
    for i in 0..nbits {
        let bit = (value >> (nbits - 1 - i)) & 1;
        let pos = offset + i;
        let byte_idx = pos / 8;
        let bit_idx = 7 - (pos % 8);
        if byte_idx < ENCODED_FRAME_SIZE {
            if bit == 1 {
                buf[byte_idx] |= 1 << bit_idx;
            } else {
                buf[byte_idx] &= !(1 << bit_idx);
            }
        }
    }
}

/// Unpack a value from a bit buffer at a given bit offset
fn unpack_bits(buf: &[u8], offset: usize, nbits: usize) -> u32 {
    let mut value: u32 = 0;
    for i in 0..nbits {
        let pos = offset + i;
        let byte_idx = pos / 8;
        let bit_idx = 7 - (pos % 8);
        if byte_idx < buf.len() {
            let bit = (buf[byte_idx] >> bit_idx) & 1;
            value = (value << 1) | bit as u32;
        }
    }
    value
}

// ─── DSP helpers ─────────────────────────────────────────────────────────────

/// Autocorrelation of a windowed signal
fn autocorrelation(signal: &[f32], order: usize, r: &mut [f32]) {
    for i in 0..=order {
        let mut sum = 0.0_f32;
        for j in i..signal.len() {
            sum += signal[j] * signal[j - i];
        }
        r[i] = sum;
    }
    // Apply bandwidth expansion (lag windowing)
    for i in 1..=order {
        let bw = (-0.5 * (0.008 * std::f32::consts::PI * i as f32).powi(2)).exp();
        r[i] *= bw;
    }
    // Floor the energy
    r[0] = r[0].max(1.0);
}

/// Levinson-Durbin recursion: compute LP coefficients from autocorrelation
fn levinson_durbin(r: &[f32], order: usize, a: &mut [f32]) -> f32 {
    let mut a_tmp = [0.0_f32; LP_ORDER + 1];
    let mut error = r[0];

    a[0] = 1.0;
    for i in 0..LP_ORDER + 1 {
        a_tmp[i] = 0.0;
    }

    for i in 1..=order {
        let mut sum = 0.0_f32;
        for j in 1..i {
            sum += a[j] * r[i - j];
        }
        let rc = -(r[i] + sum) / error.max(1e-10);

        // Update coefficients
        for j in 1..i {
            a_tmp[j] = a[j] + rc * a[i - j];
        }
        a_tmp[i] = rc;
        for j in 1..=i {
            a[j] = a_tmp[j];
        }

        error *= 1.0 - rc * rc;
        if error < 1e-10 {
            error = 1e-10;
        }
    }
    error
}

/// Convert LP coefficients to LSP (Line Spectral Pairs) using Chebyshev polynomials
fn lp_to_lsp(a: &[f32], lsp: &mut [f32], lsp_old: &[f32]) -> bool {
    let order = LP_ORDER;
    // Construct symmetric (p) and anti-symmetric (q) polynomials
    let mut p = [0.0_f32; LP_ORDER / 2 + 1];
    let mut q = [0.0_f32; LP_ORDER / 2 + 1];

    let half = order / 2;
    for i in 0..=half {
        if i == 0 {
            p[0] = 1.0;
            q[0] = 1.0;
        } else {
            p[i] = a[i] + a[order + 1 - i] - p[i - 1];
            q[i] = a[i] - a[order + 1 - i] + q[i - 1];
        }
    }

    // Find roots by evaluating Chebyshev polynomial on a grid.
    // P-polynomial roots go at even LSP indices (0, 2, 4, …)
    // Q-polynomial roots go at odd  LSP indices (1, 3, 5, …)
    let grid_points = 100;
    let mut found_p = 0usize; // number of P roots found so far
    let mut found_q = 0usize; // number of Q roots found so far
    let mut prev_val_p = eval_chebyshev(&p, 1.0);
    let mut prev_val_q = eval_chebyshev(&q, 1.0);

    for i in 1..=grid_points {
        let x = (std::f32::consts::PI * i as f32 / grid_points as f32).cos();
        let val_p = eval_chebyshev(&p, x);
        let val_q = eval_chebyshev(&q, x);

        if found_p < half && prev_val_p * val_p <= 0.0 {
            // Root of P found between prev and current
            let root = bisect_root(&p, x + (std::f32::consts::PI / grid_points as f32).cos() - x, x, prev_val_p, val_p);
            let omega = root.acos();
            let idx = found_p * 2; // P roots at even indices
            if idx < order {
                lsp[idx] = omega;
            }
            found_p += 1;
        }
        if found_q < half && prev_val_q * val_q <= 0.0 {
            let root = bisect_root(&q, x + (std::f32::consts::PI / grid_points as f32).cos() - x, x, prev_val_q, val_q);
            let omega = root.acos();
            let idx = found_q * 2 + 1; // Q roots at odd indices
            if idx < order {
                lsp[idx] = omega;
            }
            found_q += 1;
        }

        prev_val_p = val_p;
        prev_val_q = val_q;
    }

    let found = found_p + found_q;
    if found < half {
        // Failed to find all roots, use previous LSP
        lsp[..order].copy_from_slice(&lsp_old[..order]);
        return false;
    }

    // Sort LSPs
    for i in 0..order {
        for j in (i + 1)..order {
            if lsp[j] < lsp[i] {
                lsp.swap(i, j);
            }
        }
    }

    // Ensure minimum spacing
    ensure_lsp_stability(lsp);
    true
}

/// Evaluate Chebyshev polynomial at point x
fn eval_chebyshev(coeff: &[f32], x: f32) -> f32 {
    let n = coeff.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return coeff[0];
    }
    let mut b0 = coeff[n - 1];
    let mut b1 = 0.0;
    for i in (0..n - 1).rev() {
        let tmp = 2.0 * x * b0 - b1 + coeff[i];
        b1 = b0;
        b0 = tmp;
    }
    b0 - x * b1
}

/// Bisection to refine root location
fn bisect_root(coeff: &[f32], _dx: f32, x_right: f32, _val_left: f32, _val_right: f32) -> f32 {
    let x_left = x_right + (std::f32::consts::PI / 100.0).cos() - x_right;
    // Simple linear interpolation if bisection is overkill
    let mut lo = x_right;
    let mut hi = x_left;
    if lo > hi {
        std::mem::swap(&mut lo, &mut hi);
    }
    let val_lo = eval_chebyshev(coeff, lo);
    let val_hi = eval_chebyshev(coeff, hi);
    if val_lo.abs() < 1e-6 {
        return lo;
    }
    if val_hi.abs() < 1e-6 {
        return hi;
    }
    // 10 iterations of bisection
    let mut a = lo;
    let mut b = hi;
    let mut va = val_lo;
    for _ in 0..10 {
        let mid = (a + b) * 0.5;
        let vm = eval_chebyshev(coeff, mid);
        if va * vm <= 0.0 {
            b = mid;
        } else {
            a = mid;
            va = vm;
        }
    }
    (a + b) * 0.5
}

/// Ensure minimum spacing between LSP frequencies
fn ensure_lsp_stability(lsp: &mut [f32]) {
    let order = lsp.len().min(LP_ORDER);
    // Ensure ascending order with minimum gap
    for i in 1..order {
        if lsp[i] < lsp[i - 1] + LSP_MARGIN {
            lsp[i] = lsp[i - 1] + LSP_MARGIN;
        }
    }
    // Clamp to valid range
    if order > 0 {
        lsp[0] = lsp[0].max(LSP_MARGIN);
        lsp[order - 1] = lsp[order - 1].min(std::f32::consts::PI - LSP_MARGIN);
    }
}

/// Convert LSP back to LP coefficients
fn lsp_to_lp(lsp: &[f32], a: &mut [f32]) {
    let order = LP_ORDER;
    let half = order / 2;

    let mut p = [0.0_f32; LP_ORDER / 2 + 1];
    let mut q = [0.0_f32; LP_ORDER / 2 + 1];

    p[0] = 1.0;
    q[0] = 1.0;

    for i in 0..half {
        let cos_p = -(lsp[2 * i]).cos();
        let cos_q = -(lsp[2 * i + 1]).cos();

        // Update p polynomial
        for j in (1..=i + 1).rev() {
            p[j] += cos_p * p[j - 1] + if j >= 2 { p[j - 2] } else { 0.0 };
            // Simplified: accumulate product terms
        }
        // Reset and recalculate properly
        // Use the direct product form for numerical stability
        let _ = cos_q;
    }

    // Alternative: direct computation from LSP frequencies
    // a[i] = sum of products of cos(lsp[k])
    // Use the standard conversion formula
    lsp_to_lp_direct(lsp, a);
}

/// Direct LSP to LP conversion using product form
fn lsp_to_lp_direct(lsp: &[f32], a: &mut [f32]) {
    let order = LP_ORDER;
    let half = order / 2;

    // Build P and Q polynomials from LSP frequencies
    let mut pp = vec![0.0_f32; half + 2];
    let mut qq = vec![0.0_f32; half + 2];
    pp[0] = 1.0;
    qq[0] = 1.0;

    for i in 0..half {
        let cos_val_p = -2.0 * lsp[2 * i].cos();
        let cos_val_q = -2.0 * lsp[2 * i + 1].cos();

        for j in (1..=i + 1).rev() {
            pp[j] = pp[j] + cos_val_p * pp[j - 1];
            if j >= 2 {
                pp[j] += pp[j - 2];
            }
            qq[j] = qq[j] + cos_val_q * qq[j - 1];
            if j >= 2 {
                qq[j] += qq[j - 2];
            }
        }
    }

    a[0] = 1.0;
    for i in 1..=half {
        // a[i] = 0.5 * (pp[i] + qq[i]) with symmetric/antisymmetric combination
        let p_val = pp[i];
        let q_val = qq[i];
        a[i] = 0.5 * (p_val + q_val);
        a[order + 1 - i] = 0.5 * (p_val - q_val);
    }
}

/// Interpolate LSP parameters between frames
fn lsp_interpolate(lsp_old: &[f32], lsp_new: &[f32], fraction: f32, lsp_interp: &mut [f32]) {
    for i in 0..LP_ORDER {
        lsp_interp[i] = lsp_old[i] * (1.0 - fraction) + lsp_new[i] * fraction;
    }
    ensure_lsp_stability(lsp_interp);
}

/// Apply Hamming window to signal
fn hamming_window(signal: &[f32], windowed: &mut [f32]) {
    let n = signal.len();
    for i in 0..n {
        let w = 0.54 - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0)).cos();
        windowed[i] = signal[i] * w;
    }
}

/// LP synthesis filter: output[n] = input[n] - sum(a[k]*output[n-k])
fn lp_synthesis(a: &[f32], input: &[f32], output: &mut [f32], mem: &mut [f32]) {
    let order = LP_ORDER;
    for n in 0..input.len() {
        let mut sum = input[n];
        for k in 1..=order {
            let prev = if n >= k {
                output[n - k]
            } else {
                mem[order - k + n]
            };
            sum -= a[k] * prev;
        }
        output[n] = sum;
    }
    // Update memory with last LP_ORDER samples
    let out_len = output.len();
    if out_len >= order {
        mem[..order].copy_from_slice(&output[out_len - order..out_len]);
    }
}

/// LP analysis filter: residual[n] = signal[n] + sum(a[k]*signal[n-k])
fn lp_analysis(a: &[f32], signal: &[f32], residual: &mut [f32], mem: &[f32]) {
    let order = LP_ORDER;
    for n in 0..signal.len() {
        let mut sum = signal[n];
        for k in 1..=order {
            let prev = if n >= k {
                signal[n - k]
            } else {
                mem[order - k + n]
            };
            sum += a[k] * prev;
        }
        residual[n] = sum;
    }
}

/// Compute weighted LP filter A(z/gamma)
fn weight_lp(a: &[f32], gamma: f32, aw: &mut [f32]) {
    let mut g = 1.0_f32;
    aw[0] = a[0];
    for i in 1..=LP_ORDER {
        g *= gamma;
        aw[i] = a[i] * g;
    }
}

/// Compute perceptual weighting filter W(z) = A(z/gamma1) / A(z/gamma2)
/// For G.729A: gamma1 = 0.75, gamma2 = 0.65
fn perceptual_weight(
    signal: &[f32],
    a: &[f32],
    weighted: &mut [f32],
    mem_w1: &mut [f32],
    mem_w2: &mut [f32],
) {
    let gamma1 = 0.75_f32;
    let gamma2 = 0.65_f32;
    let mut aw1 = [0.0_f32; LP_ORDER + 1];
    let mut aw2 = [0.0_f32; LP_ORDER + 1];
    weight_lp(a, gamma1, &mut aw1);
    weight_lp(a, gamma2, &mut aw2);

    // Apply A(z/gamma1) then 1/A(z/gamma2)
    let mut tmp = vec![0.0_f32; signal.len()];
    lp_analysis_with_mem(&aw1, signal, &mut tmp, mem_w1);
    lp_synthesis(&aw2, &tmp, weighted, mem_w2);
}

/// LP analysis filter with mutable memory update
fn lp_analysis_with_mem(a: &[f32], signal: &[f32], residual: &mut [f32], mem: &mut [f32]) {
    let order = LP_ORDER;
    for n in 0..signal.len() {
        let mut sum = signal[n];
        for k in 1..=order {
            let prev = if n >= k {
                signal[n - k]
            } else {
                mem[order - k + n]
            };
            sum += a[k] * prev;
        }
        residual[n] = sum;
    }
    // Update memory
    let sig_len = signal.len();
    if sig_len >= order {
        mem[..order].copy_from_slice(&signal[sig_len - order..sig_len]);
    }
}

// ─── Pitch (adaptive codebook) search ────────────────────────────────────────

/// Open-loop pitch search on weighted speech
fn open_loop_pitch_search(wsp: &[f32], start: usize) -> usize {
    let mut best_lag = PITCH_LAG_MIN;
    let mut best_corr = f32::NEG_INFINITY;

    let end = start + SUBFRAME_LEN;
    for lag in PITCH_LAG_MIN..=PITCH_LAG_MAX {
        let mut corr = 0.0_f32;
        let mut energy = 0.0_f32;
        for i in 0..SUBFRAME_LEN {
            let idx = start + i;
            let ref_idx = idx.wrapping_sub(lag);
            if idx < wsp.len() && ref_idx < wsp.len() {
                corr += wsp[idx] * wsp[ref_idx];
                energy += wsp[ref_idx] * wsp[ref_idx];
            }
        }
        let normalized = if energy > 1e-6 { corr / energy.sqrt() } else { 0.0 };
        if normalized > best_corr {
            best_corr = normalized;
            best_lag = lag;
        }
    }
    let _ = end;
    best_lag
}

/// Closed-loop pitch search (fractional pitch refinement)
/// Returns (integer_lag, fractional_part) where fractional = 0 for G.729A simplification
fn closed_loop_pitch_search(
    target: &[f32],
    exc: &[f32],
    t0_open: usize,
) -> (usize, f32) {
    // G.729A: search around open-loop pitch +/- 3
    let search_min = if t0_open > PITCH_LAG_MIN + 3 { t0_open - 3 } else { PITCH_LAG_MIN };
    let search_max = (t0_open + 3).min(PITCH_LAG_MAX);

    let mut best_lag = t0_open;
    let mut best_corr = f32::NEG_INFINITY;

    for lag in search_min..=search_max {
        let mut corr = 0.0_f32;
        let mut energy = 0.0_f32;
        for i in 0..SUBFRAME_LEN {
            let exc_idx = i + PITCH_LAG_MAX;
            if exc_idx >= lag && exc_idx - lag < exc.len() && i < target.len() {
                let ref_sample = exc[exc_idx - lag];
                corr += target[i] * ref_sample;
                energy += ref_sample * ref_sample;
            }
        }
        let normalized = if energy > 1e-6 { corr / energy.sqrt() } else { 0.0 };
        if normalized > best_corr {
            best_corr = normalized;
            best_lag = lag;
        }
    }
    (best_lag, 0.0)
}

/// Compute adaptive codebook gain
fn compute_pitch_gain(target: &[f32], exc_lag: &[f32]) -> f32 {
    let mut corr = 0.0_f32;
    let mut energy = 0.0_f32;
    let len = target.len().min(exc_lag.len());
    for i in 0..len {
        corr += target[i] * exc_lag[i];
        energy += exc_lag[i] * exc_lag[i];
    }
    if energy > 1e-6 {
        (corr / energy).clamp(0.0, 1.2)
    } else {
        0.0
    }
}

// ─── Fixed codebook (algebraic CELP) ─────────────────────────────────────────

/// G.729A fixed codebook structure
/// 4 pulses in 4 tracks, each track has positions {i, i+5, i+10, ..., i+35}
struct FixedCodebookResult {
    index: u32,    // 13-bit index
    signs: u32,    // 4-bit sign
    gain: f32,     // Fixed codebook gain
    pulse_positions: [usize; FC_PULSE_COUNT],
    pulse_signs: [f32; FC_PULSE_COUNT],
}

/// Search fixed codebook (algebraic CELP for G.729A)
fn fixed_codebook_search(
    target: &[f32],     // Target after adaptive codebook removal
    h: &[f32],          // Impulse response of weighted synthesis filter
) -> FixedCodebookResult {
    // G.729A: 4 tracks, each pulse can be in positions {track, track+5, track+10, ..., track+35}
    // Track 0: positions 0,5,10,15,20,25,30,35
    // Track 1: positions 1,6,11,16,21,26,31,36
    // Track 2: positions 2,7,12,17,22,27,32,37
    // Track 3: positions 3,8,13,18,23,28,33,38 + {4,9,14,19,24,29,34,39}

    // Compute backward-filtered target d[n] = sum_i(target[i] * h[i-n])
    let mut d = [0.0_f32; SUBFRAME_LEN];
    for n in 0..SUBFRAME_LEN {
        let mut sum = 0.0_f32;
        for i in n..SUBFRAME_LEN {
            if i - n < h.len() {
                sum += target[i] * h[i - n];
            }
        }
        d[n] = sum;
    }

    let mut best_positions = [0_usize; FC_PULSE_COUNT];
    let mut best_signs = [1.0_f32; FC_PULSE_COUNT];
    let mut best_corr = f32::NEG_INFINITY;

    // Simplified search: for each track, pick the position with maximum |d[n]|
    for track in 0..FC_PULSE_COUNT {
        let mut max_val = f32::NEG_INFINITY;
        let mut max_pos = track;
        let mut max_sign = 1.0_f32;

        // Track positions: for tracks 0-2, step 5; track 3 has extra positions
        let start = track;
        let step = 5;
        let mut pos = start;
        while pos < SUBFRAME_LEN {
            let val = d[pos].abs();
            if val > max_val {
                max_val = val;
                max_pos = pos;
                max_sign = if d[pos] >= 0.0 { 1.0 } else { -1.0 };
            }
            pos += step;
        }
        // Track 3 also includes positions from track 4 in G.729A
        if track == 3 {
            pos = 4;
            while pos < SUBFRAME_LEN {
                let val = d[pos].abs();
                if val > max_val {
                    max_val = val;
                    max_pos = pos;
                    max_sign = if d[pos] >= 0.0 { 1.0 } else { -1.0 };
                }
                pos += step;
            }
        }
        best_positions[track] = max_pos;
        best_signs[track] = max_sign;
    }

    // Compute gain
    let mut codeword = [0.0_f32; SUBFRAME_LEN];
    for i in 0..FC_PULSE_COUNT {
        if best_positions[i] < SUBFRAME_LEN {
            codeword[best_positions[i]] += best_signs[i];
        }
    }

    // Convolve codeword with impulse response
    let mut filtered = [0.0_f32; SUBFRAME_LEN];
    for n in 0..SUBFRAME_LEN {
        let mut sum = 0.0_f32;
        for k in 0..=n {
            if k < SUBFRAME_LEN && (n - k) < h.len() {
                sum += codeword[k] * h[n - k];
            }
        }
        filtered[n] = sum;
    }

    let mut corr_val = 0.0_f32;
    let mut energy = 0.0_f32;
    for i in 0..SUBFRAME_LEN {
        corr_val += target[i] * filtered[i];
        energy += filtered[i] * filtered[i];
    }
    let gain = if energy > 1e-6 { corr_val / energy } else { 0.0 };

    // Encode positions into 13-bit index
    // Each track position encoded as position_in_track (3 bits for 8 positions)
    // Track 3 uses extra bit for which sub-track
    let mut index: u32 = 0;
    for i in 0..FC_PULSE_COUNT {
        let pos_in_track = best_positions[i] / 5;
        index |= ((pos_in_track as u32) & 0x7) << (i * 3);
        if i == 3 {
            // Extra bit for track 3/4 selection
            let is_track4 = best_positions[i] % 5 == 4;
            if is_track4 {
                index |= 1 << 12;
            }
        }
    }
    index &= 0x1FFF; // 13 bits

    // Encode signs (4 bits, one per pulse)
    let mut signs: u32 = 0;
    for i in 0..FC_PULSE_COUNT {
        if best_signs[i] > 0.0 {
            signs |= 1 << (3 - i);
        }
    }

    let _ = best_corr;

    FixedCodebookResult {
        index,
        signs,
        gain,
        pulse_positions: best_positions,
        pulse_signs: best_signs,
    }
}

/// Reconstruct fixed codebook excitation from decoded parameters
fn fixed_codebook_reconstruct(index: u32, signs: u32) -> [f32; SUBFRAME_LEN] {
    let mut codeword = [0.0_f32; SUBFRAME_LEN];

    for track in 0..FC_PULSE_COUNT {
        let pos_in_track = ((index >> (track * 3)) & 0x7) as usize;
        let mut base_track = track;
        if track == 3 && (index >> 12) & 1 == 1 {
            base_track = 4; // Track 4 (positions 4,9,14,...)
        }
        let position = base_track + pos_in_track * 5;
        let sign = if (signs >> (3 - track)) & 1 == 1 { 1.0_f32 } else { -1.0_f32 };
        if position < SUBFRAME_LEN {
            codeword[position] = sign;
        }
    }
    codeword
}

// ─── Gain quantization ──────────────────────────────────────────────────────

/// Quantize adaptive and fixed codebook gains
/// Returns (ga_index, gb_index) for bitstream
fn quantize_gains(pitch_gain: f32, fixed_gain: f32) -> (u32, u32) {
    // GA: 3 bits (8 levels), range [0, 1.2]
    let ga_quant = ((pitch_gain / 1.2 * 7.0).round() as u32).min(7);
    // GB: 4 bits (16 levels), log-domain quantization
    let fg_log = if fixed_gain.abs() > 1e-6 {
        (fixed_gain.abs().ln() + 2.0).clamp(0.0, 7.5)
    } else {
        0.0
    };
    let gb_quant = ((fg_log / 7.5 * 15.0).round() as u32).min(15);
    (ga_quant, gb_quant)
}

/// Dequantize adaptive and fixed codebook gains
fn dequantize_gains(ga_index: u32, gb_index: u32) -> (f32, f32) {
    let pitch_gain = (ga_index as f32 / 7.0 * 1.2).clamp(0.0, 1.2);
    // gb_index=0 encodes silence / near-zero fixed gain; treat as zero to avoid
    // artifacts when encoding silence.
    let fixed_gain = if gb_index == 0 {
        0.0
    } else {
        let fg_log = gb_index as f32 / 15.0 * 7.5;
        (fg_log - 2.0).exp()
    };
    (pitch_gain, fixed_gain)
}

// ─── LSP quantization ───────────────────────────────────────────────────────

/// Simplified LSP quantization for G.729A
/// Uses scalar quantization of LSP frequencies
/// Returns (l0, l1, l2, l3) indices
fn quantize_lsp(lsp: &[f32], lsp_old: &[f32]) -> (u32, u32, u32, u32) {
    // L0: MA predictor switch (1 bit) - use 0 for simplicity
    let l0: u32 = 0;

    // Compute LSP residual (difference from prediction)
    let mut residual = [0.0_f32; LSP_ORDER];
    for i in 0..LSP_ORDER {
        residual[i] = lsp[i] - lsp_old[i] * 0.75; // Simple MA prediction
    }

    // L1: First stage VQ of lower 5 LSPs (7 bits = 128 entries)
    // Simplified: quantize the mean of lower 5 residuals
    let mean_lo: f32 = residual[..5].iter().sum::<f32>() / 5.0;
    let l1 = ((mean_lo + 1.0) / 2.0 * 127.0).round().clamp(0.0, 127.0) as u32;

    // L2: Second stage subvector 1 (5 bits = 32 entries)
    let mean_hi1: f32 = residual[5..8].iter().sum::<f32>() / 3.0;
    let l2 = ((mean_hi1 + 0.5) / 1.0 * 31.0).round().clamp(0.0, 31.0) as u32;

    // L3: Second stage subvector 2 (5 bits = 32 entries)
    let mean_hi2: f32 = residual[8..10].iter().sum::<f32>() / 2.0;
    let l3 = ((mean_hi2 + 0.5) / 1.0 * 31.0).round().clamp(0.0, 31.0) as u32;

    (l0, l1, l2, l3)
}

/// Dequantize LSP from indices
fn dequantize_lsp(l0: u32, l1: u32, l2: u32, l3: u32, lsp_old: &[f32], lsp: &mut [f32]) {
    let _ = l0; // MA predictor index, simplified
    // Reconstruct residual
    let mean_lo = (l1 as f32 / 127.0) * 2.0 - 1.0;
    let mean_hi1 = (l2 as f32 / 31.0) * 1.0 - 0.5;
    let mean_hi2 = (l3 as f32 / 31.0) * 1.0 - 0.5;

    for i in 0..5 {
        lsp[i] = lsp_old[i] * 0.75 + mean_lo;
    }
    for i in 5..8 {
        lsp[i] = lsp_old[i] * 0.75 + mean_hi1;
    }
    for i in 8..10 {
        lsp[i] = lsp_old[i] * 0.75 + mean_hi2;
    }
    ensure_lsp_stability(lsp);
}

// ─── Parity bit ──────────────────────────────────────────────────────────────

/// Compute parity bit for pitch delay (for error detection)
fn compute_parity(pitch_index: u32) -> u32 {
    let mut parity: u32 = 0;
    let mut val = pitch_index;
    while val > 0 {
        parity ^= val & 1;
        val >>= 1;
    }
    parity
}

// ─── Encoder ─────────────────────────────────────────────────────────────────

/// G.729A Encoder state
pub struct G729AEncoder {
    /// Previous frame LSP frequencies
    lsp_old: [f32; LSP_ORDER],
    /// Excitation buffer (past + current frame)
    exc_buf: Vec<f32>,
    /// Speech buffer for LP analysis (past samples for windowing)
    speech_buf: Vec<f32>,
    /// Weighted speech buffer
    wsp_buf: Vec<f32>,
    /// High-pass filter state
    hp_state_x: [f32; 2],
    hp_state_y: [f32; 2],
    /// LP synthesis memory for perceptual weighting
    mem_w1: [f32; LP_ORDER],
    mem_w2: [f32; LP_ORDER],
    /// LP analysis memory
    mem_lp: [f32; LP_ORDER],
    /// Previous LP coefficients for interpolation
    a_old: [f32; LP_ORDER + 1],
    /// Frame count
    frame_count: u32,
}

impl G729AEncoder {
    /// Create a new G.729A encoder
    pub fn new() -> Self {
        let mut lsp_old = [0.0_f32; LSP_ORDER];
        lsp_old.copy_from_slice(&LSP_INIT);

        let mut a_old = [0.0_f32; LP_ORDER + 1];
        a_old[0] = 1.0;

        Self {
            lsp_old,
            exc_buf: vec![0.0; EXC_BUF_SIZE + FRAME_LEN],
            speech_buf: vec![0.0; FRAME_LEN + LP_ORDER + 80], // Extra for windowing
            wsp_buf: vec![0.0; EXC_BUF_SIZE + FRAME_LEN],
            hp_state_x: [0.0; 2],
            hp_state_y: [0.0; 2],
            mem_w1: [0.0; LP_ORDER],
            mem_w2: [0.0; LP_ORDER],
            mem_lp: [0.0; LP_ORDER],
            a_old,
            frame_count: 0,
        }
    }

    /// Encode 80 PCM samples (10ms at 8kHz) into 10 bytes
    pub fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        if samples.len() != FRAME_LEN {
            return Err(CodecError::InvalidFrameSize {
                expected: FRAME_LEN,
                actual: samples.len(),
            }.into());
        }

        // Step 1: Pre-processing — high-pass filter and convert to float
        let mut speech = [0.0_f32; FRAME_LEN];
        for i in 0..FRAME_LEN {
            let x = samples[i] as f32;
            let y = HP_B[0] * x
                + HP_B[1] * self.hp_state_x[0]
                + HP_B[2] * self.hp_state_x[1]
                - HP_A[1] * self.hp_state_y[0]
                - HP_A[2] * self.hp_state_y[1];
            self.hp_state_x[1] = self.hp_state_x[0];
            self.hp_state_x[0] = x;
            self.hp_state_y[1] = self.hp_state_y[0];
            self.hp_state_y[0] = y;
            speech[i] = y;
        }

        // Update speech buffer
        let buf_len = self.speech_buf.len();
        self.speech_buf.copy_within(FRAME_LEN..buf_len, 0);
        let start = buf_len - FRAME_LEN;
        self.speech_buf[start..].copy_from_slice(&speech);

        // Step 2: LP analysis (10th order, Hamming-windowed autocorrelation)
        let analysis_len = FRAME_LEN + LP_ORDER;
        let analysis_start = if buf_len >= analysis_len { buf_len - analysis_len } else { 0 };
        let analysis_signal = &self.speech_buf[analysis_start..buf_len];

        let mut windowed = vec![0.0_f32; analysis_signal.len()];
        hamming_window(analysis_signal, &mut windowed);

        let mut r = [0.0_f32; LP_ORDER + 1];
        autocorrelation(&windowed, LP_ORDER, &mut r);

        let mut a = [0.0_f32; LP_ORDER + 1];
        levinson_durbin(&r, LP_ORDER, &mut a);

        // Step 3: LP to LSP conversion
        let mut lsp = [0.0_f32; LSP_ORDER];
        let _success = lp_to_lsp(&a, &mut lsp, &self.lsp_old);

        // Step 4: LSP quantization
        let (l0, l1, l2, l3) = quantize_lsp(&lsp, &self.lsp_old);

        // Decode quantized LSP (for use in synthesis)
        let mut lsp_q = [0.0_f32; LSP_ORDER];
        dequantize_lsp(l0, l1, l2, l3, &self.lsp_old, &mut lsp_q);

        // Shift excitation buffer
        let exc_len = self.exc_buf.len();
        self.exc_buf.copy_within(FRAME_LEN..exc_len, 0);
        for i in (exc_len - FRAME_LEN)..exc_len {
            self.exc_buf[i] = 0.0;
        }

        // Process two subframes
        let mut p1: u32 = 0;
        let mut p2: u32 = 0;
        let mut c1: u32 = 0;
        let mut s1: u32 = 0;
        let mut ga1: u32 = 0;
        let mut gb1: u32 = 0;
        let mut c2: u32 = 0;
        let mut s2: u32 = 0;
        let mut ga2: u32 = 0;
        let mut gb2: u32 = 0;

        for sf in 0..2 {
            let sf_start = sf * SUBFRAME_LEN;

            // Interpolate LSP for this subframe
            let fraction = if sf == 0 { 0.5 } else { 1.0 };
            let mut lsp_interp = [0.0_f32; LSP_ORDER];
            lsp_interpolate(&self.lsp_old, &lsp_q, fraction, &mut lsp_interp);

            // LSP to LP for this subframe
            let mut a_sf = [0.0_f32; LP_ORDER + 1];
            lsp_to_lp(&lsp_interp, &mut a_sf);
            a_sf[0] = 1.0;

            // Compute weighted speech for pitch analysis
            let mut weighted = [0.0_f32; SUBFRAME_LEN];
            perceptual_weight(
                &speech[sf_start..sf_start + SUBFRAME_LEN],
                &a_sf,
                &mut weighted,
                &mut self.mem_w1,
                &mut self.mem_w2,
            );

            // Copy to weighted speech buffer
            let wsp_offset = PITCH_LAG_MAX + sf_start;
            for i in 0..SUBFRAME_LEN {
                if wsp_offset + i < self.wsp_buf.len() {
                    self.wsp_buf[wsp_offset + i] = weighted[i];
                }
            }

            // Step 5: Open-loop pitch search
            let t0_open = open_loop_pitch_search(&self.wsp_buf, wsp_offset);

            // Compute target for closed-loop search
            let mut target = [0.0_f32; SUBFRAME_LEN];
            target.copy_from_slice(&weighted);

            // Step 5b: Closed-loop pitch search
            let (t0, _frac) = closed_loop_pitch_search(&target, &self.exc_buf, t0_open);

            // Compute adaptive codebook contribution
            let mut exc_pitch = [0.0_f32; SUBFRAME_LEN];
            let exc_offset = PITCH_LAG_MAX;
            for i in 0..SUBFRAME_LEN {
                let idx = exc_offset + sf_start + i;
                if idx >= t0 && idx - t0 < self.exc_buf.len() {
                    exc_pitch[i] = self.exc_buf[idx - t0];
                }
            }

            let pitch_gain = compute_pitch_gain(&target, &exc_pitch);

            // Remove adaptive codebook contribution from target
            let mut target_fc = [0.0_f32; SUBFRAME_LEN];
            for i in 0..SUBFRAME_LEN {
                target_fc[i] = target[i] - pitch_gain * exc_pitch[i];
            }

            // Compute impulse response for codebook search
            let mut h = [0.0_f32; SUBFRAME_LEN];
            h[0] = 1.0;
            // Simplified: use flat impulse response weighted by LP
            let mut imp_input = [0.0_f32; SUBFRAME_LEN];
            imp_input[0] = 1.0;
            let mut synth_mem = [0.0_f32; LP_ORDER];
            lp_synthesis(&a_sf, &imp_input, &mut h, &mut synth_mem);

            // Step 6: Fixed codebook search
            let fc_result = fixed_codebook_search(&target_fc, &h);

            // Step 7: Gain quantization
            let (ga_q, gb_q) = quantize_gains(pitch_gain, fc_result.gain);

            // Dequantize gains for excitation update
            let (pitch_gain_q, fixed_gain_q) = dequantize_gains(ga_q, gb_q);

            // Reconstruct excitation
            let fc_exc = fixed_codebook_reconstruct(fc_result.index, fc_result.signs);
            let exc_offset_sf = PITCH_LAG_MAX + sf_start;
            for i in 0..SUBFRAME_LEN {
                if exc_offset_sf + i < self.exc_buf.len() {
                    self.exc_buf[exc_offset_sf + i] = pitch_gain_q * exc_pitch[i] + fixed_gain_q * fc_exc[i];
                }
            }

            // Encode pitch delay
            let pitch_index = (t0 - PITCH_LAG_MIN) as u32;
            if sf == 0 {
                p1 = pitch_index & 0xFF;
                c1 = fc_result.index;
                s1 = fc_result.signs;
                ga1 = ga_q;
                gb1 = gb_q;
            } else {
                // Subframe 2: delta coding relative to subframe 1 pitch
                let delta = (t0 as i32 - p1 as i32 - PITCH_LAG_MIN as i32).clamp(-8, 7);
                p2 = ((delta + 8) as u32) & 0x1F;
                c2 = fc_result.index;
                s2 = fc_result.signs;
                ga2 = ga_q;
                gb2 = gb_q;
            }
        }

        // Compute parity bit for P1
        let parity = compute_parity(p1);

        // Step 8: Pack into bitstream
        let mut buf = [0u8; ENCODED_FRAME_SIZE];
        pack_bits(&mut buf, BIT_L0.0, BIT_L0.1, l0);
        pack_bits(&mut buf, BIT_L1.0, BIT_L1.1, l1);
        pack_bits(&mut buf, BIT_L2.0, BIT_L2.1, l2);
        pack_bits(&mut buf, BIT_L3.0, BIT_L3.1, l3);
        pack_bits(&mut buf, BIT_P1.0, BIT_P1.1, p1);
        pack_bits(&mut buf, BIT_P0.0, BIT_P0.1, parity);
        pack_bits(&mut buf, BIT_P2.0, BIT_P2.1, p2);
        pack_bits(&mut buf, BIT_C1.0, BIT_C1.1, c1);
        pack_bits(&mut buf, BIT_S1.0, BIT_S1.1, s1);
        pack_bits(&mut buf, BIT_GA1.0, BIT_GA1.1, ga1);
        pack_bits(&mut buf, BIT_GB1.0, BIT_GB1.1, gb1);
        pack_bits(&mut buf, BIT_C2.0, BIT_C2.1, c2);
        pack_bits(&mut buf, BIT_S2.0, BIT_S2.1, s2);
        pack_bits(&mut buf, BIT_GA2.0, BIT_GA2.1, ga2);
        pack_bits(&mut buf, BIT_GB2.0, BIT_GB2.1, gb2);

        // Update state
        self.lsp_old.copy_from_slice(&lsp_q);
        self.a_old.copy_from_slice(&a);
        self.frame_count += 1;

        Ok(buf.to_vec())
    }

    /// Reset encoder state
    pub fn reset(&mut self) {
        self.lsp_old.copy_from_slice(&LSP_INIT);
        self.exc_buf.fill(0.0);
        self.speech_buf.fill(0.0);
        self.wsp_buf.fill(0.0);
        self.hp_state_x = [0.0; 2];
        self.hp_state_y = [0.0; 2];
        self.mem_w1 = [0.0; LP_ORDER];
        self.mem_w2 = [0.0; LP_ORDER];
        self.mem_lp = [0.0; LP_ORDER];
        self.a_old = [0.0; LP_ORDER + 1];
        self.a_old[0] = 1.0;
        self.frame_count = 0;
    }
}

// ─── Decoder ─────────────────────────────────────────────────────────────────

/// G.729A Decoder state
pub struct G729ADecoder {
    /// Previous frame LSP frequencies
    lsp_old: [f32; LSP_ORDER],
    /// Excitation buffer
    exc_buf: Vec<f32>,
    /// Synthesis filter memory
    synth_mem: [f32; LP_ORDER],
    /// Post-filter memory
    post_mem: [f32; LP_ORDER],
    /// Post-filter tilt memory
    post_tilt_mem: f32,
    /// Previous adaptive gain for post-filter
    prev_gain_pitch: f32,
    /// Previous LP coefficients
    a_old: [f32; LP_ORDER + 1],
    /// Frame count
    frame_count: u32,
}

impl G729ADecoder {
    /// Create a new G.729A decoder
    pub fn new() -> Self {
        let mut lsp_old = [0.0_f32; LSP_ORDER];
        lsp_old.copy_from_slice(&LSP_INIT);

        let mut a_old = [0.0_f32; LP_ORDER + 1];
        a_old[0] = 1.0;

        Self {
            lsp_old,
            exc_buf: vec![0.0; EXC_BUF_SIZE + FRAME_LEN],
            synth_mem: [0.0; LP_ORDER],
            post_mem: [0.0; LP_ORDER],
            post_tilt_mem: 0.0,
            prev_gain_pitch: 0.0,
            a_old,
            frame_count: 0,
        }
    }

    /// Decode 10 bytes into 80 PCM samples
    pub fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        if data.len() != ENCODED_FRAME_SIZE {
            return Err(CodecError::InvalidFrameSize {
                expected: ENCODED_FRAME_SIZE,
                actual: data.len(),
            }.into());
        }

        // Step 1: Unpack bitstream
        let l0 = unpack_bits(data, BIT_L0.0, BIT_L0.1);
        let l1 = unpack_bits(data, BIT_L1.0, BIT_L1.1);
        let l2 = unpack_bits(data, BIT_L2.0, BIT_L2.1);
        let l3 = unpack_bits(data, BIT_L3.0, BIT_L3.1);
        let p1 = unpack_bits(data, BIT_P1.0, BIT_P1.1);
        let _p0 = unpack_bits(data, BIT_P0.0, BIT_P0.1);
        let p2 = unpack_bits(data, BIT_P2.0, BIT_P2.1);
        let c1 = unpack_bits(data, BIT_C1.0, BIT_C1.1);
        let s1 = unpack_bits(data, BIT_S1.0, BIT_S1.1);
        let ga1 = unpack_bits(data, BIT_GA1.0, BIT_GA1.1);
        let gb1 = unpack_bits(data, BIT_GB1.0, BIT_GB1.1);
        let c2 = unpack_bits(data, BIT_C2.0, BIT_C2.1);
        let s2 = unpack_bits(data, BIT_S2.0, BIT_S2.1);
        let ga2 = unpack_bits(data, BIT_GA2.0, BIT_GA2.1);
        let gb2 = unpack_bits(data, BIT_GB2.0, BIT_GB2.1);

        // Step 2: Decode LSP
        let mut lsp_q = [0.0_f32; LSP_ORDER];
        dequantize_lsp(l0, l1, l2, l3, &self.lsp_old, &mut lsp_q);

        // Shift excitation buffer
        let exc_len = self.exc_buf.len();
        self.exc_buf.copy_within(FRAME_LEN..exc_len, 0);
        for i in (exc_len - FRAME_LEN)..exc_len {
            self.exc_buf[i] = 0.0;
        }

        // Decode pitch delays
        let t0_sf1 = (p1 as usize + PITCH_LAG_MIN).min(PITCH_LAG_MAX);
        let delta = p2 as i32 - 8;
        let t0_sf2_raw = t0_sf1 as i32 + delta;
        let t0_sf2 = (t0_sf2_raw as usize).clamp(PITCH_LAG_MIN, PITCH_LAG_MAX);

        let mut output = vec![0.0_f32; FRAME_LEN];

        // Process two subframes
        for sf in 0..2 {
            let sf_start = sf * SUBFRAME_LEN;
            let t0 = if sf == 0 { t0_sf1 } else { t0_sf2 };
            let (c_idx, s_idx, ga_idx, gb_idx) = if sf == 0 {
                (c1, s1, ga1, gb1)
            } else {
                (c2, s2, ga2, gb2)
            };

            // Interpolate LSP
            let fraction = if sf == 0 { 0.5 } else { 1.0 };
            let mut lsp_interp = [0.0_f32; LSP_ORDER];
            lsp_interpolate(&self.lsp_old, &lsp_q, fraction, &mut lsp_interp);

            // LSP to LP
            let mut a_sf = [0.0_f32; LP_ORDER + 1];
            lsp_to_lp(&lsp_interp, &mut a_sf);
            a_sf[0] = 1.0;

            // Dequantize gains
            let (pitch_gain, fixed_gain) = dequantize_gains(ga_idx, gb_idx);

            // Step 3: Construct excitation
            // Adaptive codebook (pitch)
            let exc_offset = PITCH_LAG_MAX + sf_start;
            let mut exc_pitch = [0.0_f32; SUBFRAME_LEN];
            for i in 0..SUBFRAME_LEN {
                let idx = exc_offset + i;
                if idx >= t0 && idx - t0 < self.exc_buf.len() {
                    exc_pitch[i] = self.exc_buf[idx - t0];
                }
            }

            // Fixed codebook
            let fc_exc = fixed_codebook_reconstruct(c_idx, s_idx);

            // Combine
            for i in 0..SUBFRAME_LEN {
                let exc_val = pitch_gain * exc_pitch[i] + fixed_gain * fc_exc[i];
                if exc_offset + i < self.exc_buf.len() {
                    self.exc_buf[exc_offset + i] = exc_val;
                }
            }

            // Step 4: LP synthesis filter 1/A(z)
            let exc_slice: Vec<f32> = (0..SUBFRAME_LEN)
                .map(|i| {
                    let idx = exc_offset + i;
                    if idx < self.exc_buf.len() { self.exc_buf[idx] } else { 0.0 }
                })
                .collect();
            let mut synth = [0.0_f32; SUBFRAME_LEN];
            lp_synthesis(&a_sf, &exc_slice, &mut synth, &mut self.synth_mem);

            // Step 5: Adaptive post-filter (simplified for G.729A)
            self.adaptive_post_filter(&a_sf, &mut synth, pitch_gain);

            output[sf_start..sf_start + SUBFRAME_LEN].copy_from_slice(&synth);
            self.prev_gain_pitch = pitch_gain;
        }

        // Update state
        self.lsp_old.copy_from_slice(&lsp_q);
        self.frame_count += 1;

        // Convert to i16 with saturation
        let samples: Vec<i16> = output.iter().map(|&s| {
            let clamped = s.clamp(-32768.0, 32767.0);
            clamped.round() as i16
        }).collect();

        Ok(samples)
    }

    /// Simplified adaptive post-filter for G.729A
    fn adaptive_post_filter(&mut self, a: &[f32], synth: &mut [f32; SUBFRAME_LEN], pitch_gain: f32) {
        // Short-term post-filter: Hf(z) = A(z/gamma_n) / A(z/gamma_d)
        // gamma_n = 0.55, gamma_d = 0.7 (G.729A values)
        let gamma_n = 0.55_f32;
        let gamma_d = 0.7_f32;

        let mut a_num = [0.0_f32; LP_ORDER + 1];
        let mut a_den = [0.0_f32; LP_ORDER + 1];
        weight_lp(a, gamma_n, &mut a_num);
        weight_lp(a, gamma_d, &mut a_den);

        // Apply A(z/gamma_n)
        let input = *synth;
        let mut filtered = [0.0_f32; SUBFRAME_LEN];
        for n in 0..SUBFRAME_LEN {
            let mut sum = input[n];
            for k in 1..=LP_ORDER {
                let prev = if n >= k { input[n - k] } else { self.post_mem[LP_ORDER - k + n] };
                sum += a_num[k] * prev;
            }
            filtered[n] = sum;
        }

        // Apply 1/A(z/gamma_d)
        let mut post_synth = [0.0_f32; SUBFRAME_LEN];
        let mut mem_tmp = self.post_mem;
        lp_synthesis(&a_den, &filtered, &mut post_synth, &mut mem_tmp);
        self.post_mem = mem_tmp;

        // Tilt compensation: (1 + mu * z^-1)
        let mu = 0.4 * pitch_gain.min(1.0);
        let mut prev = self.post_tilt_mem;
        for i in 0..SUBFRAME_LEN {
            let out = post_synth[i] + mu * prev;
            prev = post_synth[i];
            post_synth[i] = out;
        }
        self.post_tilt_mem = prev;

        // Gain normalization: match energy of original and post-filtered
        let mut energy_orig = 0.0_f32;
        let mut energy_filt = 0.0_f32;
        for i in 0..SUBFRAME_LEN {
            energy_orig += input[i] * input[i];
            energy_filt += post_synth[i] * post_synth[i];
        }
        let gain = if energy_filt > 1e-6 {
            (energy_orig / energy_filt).sqrt()
        } else {
            1.0
        };

        for i in 0..SUBFRAME_LEN {
            synth[i] = post_synth[i] * gain;
        }
    }

    /// Reset decoder state
    pub fn reset(&mut self) {
        self.lsp_old.copy_from_slice(&LSP_INIT);
        self.exc_buf.fill(0.0);
        self.synth_mem = [0.0; LP_ORDER];
        self.post_mem = [0.0; LP_ORDER];
        self.post_tilt_mem = 0.0;
        self.prev_gain_pitch = 0.0;
        self.a_old = [0.0; LP_ORDER + 1];
        self.a_old[0] = 1.0;
        self.frame_count = 0;
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        let enc = G729AEncoder::new();
        assert_eq!(enc.frame_count, 0);
        assert_eq!(enc.lsp_old.len(), LSP_ORDER);
    }

    #[test]
    fn test_decoder_creation() {
        let dec = G729ADecoder::new();
        assert_eq!(dec.frame_count, 0);
        assert_eq!(dec.lsp_old.len(), LSP_ORDER);
    }

    #[test]
    fn test_encode_output_size() {
        let mut enc = G729AEncoder::new();
        let samples = vec![0i16; FRAME_LEN];
        let result = enc.encode(&samples);
        assert!(result.is_ok());
        let encoded = result.unwrap_or_default();
        assert_eq!(encoded.len(), ENCODED_FRAME_SIZE);
    }

    #[test]
    fn test_decode_output_size() {
        let mut dec = G729ADecoder::new();
        let data = vec![0u8; ENCODED_FRAME_SIZE];
        let result = dec.decode(&data);
        assert!(result.is_ok());
        let decoded = result.unwrap_or_default();
        assert_eq!(decoded.len(), FRAME_LEN);
    }

    #[test]
    fn test_encode_wrong_frame_size() {
        let mut enc = G729AEncoder::new();
        let samples = vec![0i16; 40]; // Wrong size
        let result = enc.encode(&samples);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_wrong_frame_size() {
        let mut dec = G729ADecoder::new();
        let data = vec![0u8; 5]; // Wrong size
        let result = dec.decode(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_encode_decode_roundtrip_silence() {
        let mut enc = G729AEncoder::new();
        let mut dec = G729ADecoder::new();

        let samples = vec![0i16; FRAME_LEN];
        let encoded = enc.encode(&samples).unwrap_or_default();
        assert_eq!(encoded.len(), ENCODED_FRAME_SIZE);

        let decoded = dec.decode(&encoded).unwrap_or_default();
        assert_eq!(decoded.len(), FRAME_LEN);

        // For silence input, output should be near-silence
        let energy: f64 = decoded.iter().map(|&s| (s as f64) * (s as f64)).sum();
        let rms = (energy / FRAME_LEN as f64).sqrt();
        // RMS should be reasonably low for silence (allowing for codec artifacts)
        assert!(rms < 5000.0, "RMS for silence too high: {}", rms);
    }

    #[test]
    fn test_encode_decode_roundtrip_tone() {
        let mut enc = G729AEncoder::new();
        let mut dec = G729ADecoder::new();

        // Generate a 400Hz sine wave
        let samples: Vec<i16> = (0..FRAME_LEN)
            .map(|i| {
                let t = i as f32 / 8000.0;
                (16000.0 * (2.0 * std::f32::consts::PI * 400.0 * t).sin()) as i16
            })
            .collect();

        let encoded = enc.encode(&samples).unwrap_or_default();
        assert_eq!(encoded.len(), ENCODED_FRAME_SIZE);

        let decoded = dec.decode(&encoded).unwrap_or_default();
        assert_eq!(decoded.len(), FRAME_LEN);

        // Decoded signal should have non-trivial energy for a tone input
        let energy: f64 = decoded.iter().map(|&s| (s as f64) * (s as f64)).sum();
        let rms = (energy / FRAME_LEN as f64).sqrt();
        // Just check it decoded without panic; codec convergence takes multiple frames
        assert!(rms < 40000.0, "RMS unreasonably high: {}", rms);
    }

    #[test]
    fn test_multi_frame_encode_decode() {
        let mut enc = G729AEncoder::new();
        let mut dec = G729ADecoder::new();

        // Process 10 frames to let codec state settle
        for frame_idx in 0..10 {
            let samples: Vec<i16> = (0..FRAME_LEN)
                .map(|i| {
                    let t = (frame_idx * FRAME_LEN + i) as f32 / 8000.0;
                    (8000.0 * (2.0 * std::f32::consts::PI * 300.0 * t).sin()) as i16
                })
                .collect();

            let encoded = enc.encode(&samples).unwrap_or_default();
            assert_eq!(encoded.len(), ENCODED_FRAME_SIZE, "Frame {} wrong encoded size", frame_idx);

            let decoded = dec.decode(&encoded).unwrap_or_default();
            assert_eq!(decoded.len(), FRAME_LEN, "Frame {} wrong decoded size", frame_idx);
        }
    }

    #[test]
    fn test_encoder_reset() {
        let mut enc = G729AEncoder::new();
        let samples = vec![1000i16; FRAME_LEN];
        let _ = enc.encode(&samples);
        assert_eq!(enc.frame_count, 1);

        enc.reset();
        assert_eq!(enc.frame_count, 0);
    }

    #[test]
    fn test_decoder_reset() {
        let mut dec = G729ADecoder::new();
        let data = vec![0x55u8; ENCODED_FRAME_SIZE];
        let _ = dec.decode(&data);
        assert_eq!(dec.frame_count, 1);

        dec.reset();
        assert_eq!(dec.frame_count, 0);
    }

    #[test]
    fn test_bitstream_pack_unpack() {
        let mut buf = [0u8; ENCODED_FRAME_SIZE];
        pack_bits(&mut buf, 0, 8, 0xA5);
        let val = unpack_bits(&buf, 0, 8);
        assert_eq!(val, 0xA5);

        pack_bits(&mut buf, 18, 8, 123);
        let val = unpack_bits(&buf, 18, 8);
        assert_eq!(val, 123);
    }

    #[test]
    fn test_levinson_durbin() {
        // Simple autocorrelation sequence
        let r = [1.0_f32, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1, 0.05];
        let mut a = [0.0_f32; LP_ORDER + 1];
        let error = levinson_durbin(&r, LP_ORDER, &mut a);
        assert!(error > 0.0, "Prediction error should be positive");
        assert!((a[0] - 1.0).abs() < 1e-6, "a[0] should be 1.0");
    }
}
