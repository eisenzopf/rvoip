//! G.722 State Management
//!
//! This module defines the state structures used by the G.722 codec.
//! Based on the ITU-T G.722 reference implementation.
//! Updated to match ITU-T G.722 Annex E (Release 3.00, 2014-11) exactly.

/// ADPCM state for a single sub-band (low or high)
/// 
/// This structure contains all the state variables needed for ADPCM 
/// encoding and decoding in one sub-band.
/// Updated to match ITU-T reference implementation g722_state structure.
#[derive(Debug, Clone)]
pub struct AdpcmState {
    /// Predictor coefficients (poles): a[0] unused, a[1] = a1, a[2] = a2
    pub a: [i16; 3],
    
    /// Predictor coefficients (zeros): b[0] unused, b[1] = b1, ..., b[6] = b6
    pub b: [i16; 7],
    
    /// Quantizer scale factor
    pub det: i16,
    
    /// Quantized difference signal: dlt[0] = current, dlt[1] = previous, etc.
    pub dlt: [i16; 7],
    
    /// Logarithmic quantizer scale factor
    pub nb: i16,
    
    /// Partial signal estimate: plt[0] = current, plt[1] = previous, etc.
    pub plt: [i16; 3],
    
    /// Reconstructed signal: rlt[0] = current, rlt[1] = previous, etc.
    pub rlt: [i16; 3],
    
    /// Signal estimate (from ITU-T reference: sl/sh for low/high band)
    pub s: i16,
    
    /// Slow part of signal estimate (from ITU-T reference: spl/sph for low/high band)
    pub sp: i16,
    
    /// Fast part of signal estimate (from ITU-T reference: szl/szh for low/high band)
    pub sz: i16,
    
    // ================ NEW FIELDS FOR ITU-T COMPLIANCE ================
    
    /// Signal estimate for low-band (sl) or high-band (sh)
    /// This is the main signal estimate used in the reference implementation
    pub sl: i16,
    
    /// Slow part of signal estimate for low-band (spl) or high-band (sph)
    pub spl: i16,
    
    /// Fast part of signal estimate for low-band (szl) or high-band (szh)
    pub szl: i16,
}

impl AdpcmState {
    /// Create a new ADPCM state with default initialization
    /// 
    /// Initialize with ITU-T reference default values
    pub fn new() -> Self {
        Self {
            a: [0; 3],
            b: [0; 7],
            det: 32,      // Initial quantizer scale factor
            dlt: [0; 7],
            nb: 0,
            plt: [0; 3],
            rlt: [0; 3],
            s: 0,
            sp: 0,
            sz: 0,
            // New ITU-T reference fields
            sl: 0,
            spl: 0,
            szl: 0,
        }
    }
    
    /// Reset the ADPCM state to initial values
    pub fn reset(&mut self) {
        self.a = [0; 3];
        self.b = [0; 7];
        self.det = 32;
        self.dlt = [0; 7];
        self.nb = 0;
        self.plt = [0; 3];
        self.rlt = [0; 3];
        self.s = 0;
        self.sp = 0;
        self.sz = 0;
        // Reset new ITU-T reference fields
        self.sl = 0;
        self.spl = 0;
        self.szl = 0;
    }
}

impl Default for AdpcmState {
    fn default() -> Self {
        Self::new()
    }
}

/// Complete G.722 codec state
/// 
/// This structure contains all the state variables needed for G.722 
/// encoding and decoding, including ADPCM states for both sub-bands
/// and QMF filter delay lines.
/// Updated to match ITU-T reference implementation g722_state structure.
#[derive(Debug, Clone)]
pub struct G722State {
    /// Low-band ADPCM state
    pub low_band: AdpcmState,
    
    /// High-band ADPCM state  
    pub high_band: AdpcmState,
    
    /// QMF transmit (analysis) delay line - 24 samples
    /// (ITU-T reference: qmf_tx_delayx[24])
    pub qmf_tx_delay: [i16; 24],
    
    /// QMF receive (synthesis) delay line - 24 samples
    /// (ITU-T reference: qmf_rx_delayx[24])
    pub qmf_rx_delay: [i16; 24],
}

impl G722State {
    /// Create a new G.722 state with default initialization
    /// 
    /// Initialize with ITU-T reference default values
    pub fn new() -> Self {
        Self {
            low_band: AdpcmState::new(),
            high_band: AdpcmState::new(),
            qmf_tx_delay: [0; 24],
            qmf_rx_delay: [0; 24],
        }
    }
    
    /// Reset the G.722 state to initial values
    pub fn reset(&mut self) {
        self.low_band.reset();
        self.high_band.reset();
        self.qmf_tx_delay = [0; 24];
        self.qmf_rx_delay = [0; 24];
    }
    
    /// Get mutable reference to low-band state
    pub fn low_band_mut(&mut self) -> &mut AdpcmState {
        &mut self.low_band
    }
    
    /// Get mutable reference to high-band state
    pub fn high_band_mut(&mut self) -> &mut AdpcmState {
        &mut self.high_band
    }
    
    /// Get reference to low-band state
    pub fn low_band(&self) -> &AdpcmState {
        &self.low_band
    }
    
    /// Get reference to high-band state
    pub fn high_band(&self) -> &AdpcmState {
        &self.high_band
    }
    
    /// Get mutable reference to QMF transmit delay line
    pub fn qmf_tx_delay_mut(&mut self) -> &mut [i16; 24] {
        &mut self.qmf_tx_delay
    }
    
    /// Get mutable reference to QMF receive delay line
    pub fn qmf_rx_delay_mut(&mut self) -> &mut [i16; 24] {
        &mut self.qmf_rx_delay
    }
    
    /// Get reference to QMF transmit delay line
    pub fn qmf_tx_delay(&self) -> &[i16; 24] {
        &self.qmf_tx_delay
    }
    
    /// Get reference to QMF receive delay line
    pub fn qmf_rx_delay(&self) -> &[i16; 24] {
        &self.qmf_rx_delay
    }
}

impl Default for G722State {
    fn default() -> Self {
        Self::new()
    }
}

/// G.722 encoder state
/// 
/// Wraps the common G.722 state with encoder-specific functionality
#[derive(Debug, Clone)]
pub struct G722EncoderState {
    /// Common G.722 state
    pub state: G722State,
    
    /// Input buffer for processing pairs of samples
    pub input_buffer: [i16; 2],
    
    /// Buffer index for input processing
    pub buffer_index: usize,
}

impl G722EncoderState {
    /// Create a new G.722 encoder state
    pub fn new() -> Self {
        Self {
            state: G722State::new(),
            input_buffer: [0; 2],
            buffer_index: 0,
        }
    }
    
    /// Reset the encoder state
    pub fn reset(&mut self) {
        self.state.reset();
        self.input_buffer = [0; 2];
        self.buffer_index = 0;
    }
    
    /// Get mutable reference to the underlying state
    pub fn state_mut(&mut self) -> &mut G722State {
        &mut self.state
    }
    
    /// Get reference to the underlying state
    pub fn state(&self) -> &G722State {
        &self.state
    }
}

impl Default for G722EncoderState {
    fn default() -> Self {
        Self::new()
    }
}

/// G.722 decoder state
/// 
/// Wraps the common G.722 state with decoder-specific functionality
#[derive(Debug, Clone)]
pub struct G722DecoderState {
    /// Common G.722 state
    pub state: G722State,
    
    /// Output buffer for reconstructed sample pairs
    pub output_buffer: [i16; 2],
    
    /// Buffer index for output processing
    pub buffer_index: usize,
}

impl G722DecoderState {
    /// Create a new G.722 decoder state
    pub fn new() -> Self {
        Self {
            state: G722State::new(),
            output_buffer: [0; 2],
            buffer_index: 0,
        }
    }
    
    /// Reset the decoder state
    pub fn reset(&mut self) {
        self.state.reset();
        self.output_buffer = [0; 2];
        self.buffer_index = 0;
    }
    
    /// Get mutable reference to the underlying state
    pub fn state_mut(&mut self) -> &mut G722State {
        &mut self.state
    }
    
    /// Get reference to the underlying state
    pub fn state(&self) -> &G722State {
        &self.state
    }
}

impl Default for G722DecoderState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adpcm_state_creation() {
        let state = AdpcmState::new();
        assert_eq!(state.det, 32);
        assert_eq!(state.s, 0);
        assert_eq!(state.sp, 0);
        assert_eq!(state.sz, 0);
        assert_eq!(state.sl, 0);
        assert_eq!(state.spl, 0);
        assert_eq!(state.szl, 0);
    }

    #[test]
    fn test_adpcm_state_reset() {
        let mut state = AdpcmState::new();
        state.s = 1000;
        state.det = 100;
        state.sl = 500;
        state.szl = 200;
        
        state.reset();
        
        assert_eq!(state.det, 32);
        assert_eq!(state.s, 0);
        assert_eq!(state.sl, 0);
        assert_eq!(state.szl, 0);
    }

    #[test]
    fn test_g722_state_creation() {
        let state = G722State::new();
        assert_eq!(state.low_band.det, 32);
        assert_eq!(state.high_band.det, 32);
        assert_eq!(state.qmf_tx_delay.len(), 24);
        assert_eq!(state.qmf_rx_delay.len(), 24);
    }

    #[test]
    fn test_g722_state_reset() {
        let mut state = G722State::new();
        state.low_band.s = 1000;
        state.high_band.s = 2000;
        state.qmf_tx_delay[0] = 500;
        state.low_band.sl = 300;
        state.high_band.szl = 400;
        
        state.reset();
        
        assert_eq!(state.low_band.s, 0);
        assert_eq!(state.high_band.s, 0);
        assert_eq!(state.qmf_tx_delay[0], 0);
        assert_eq!(state.low_band.sl, 0);
        assert_eq!(state.high_band.szl, 0);
    }

    #[test]
    fn test_encoder_state_creation() {
        let state = G722EncoderState::new();
        assert_eq!(state.state.low_band.det, 32);
        assert_eq!(state.buffer_index, 0);
    }

    #[test]
    fn test_decoder_state_creation() {
        let state = G722DecoderState::new();
        assert_eq!(state.state.low_band.det, 32);
        assert_eq!(state.buffer_index, 0);
    }
} 