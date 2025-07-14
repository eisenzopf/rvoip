//! Unit tests for individual G.722 components

#[cfg(test)]
mod tests {
    use crate::codecs::g722::{qmf, adpcm, state::*, tables::*};

    #[test]
    fn test_qmf_coefficients() {
        // Test that coefficients are properly loaded
        assert_eq!(QMF_COEFFS.len(), 24);
        assert_eq!(QMF_COEFFS[0], 6);
        assert_eq!(QMF_COEFFS[1], -22);
    }

    #[test]
    fn test_adpcm_state_initialization() {
        let state = AdpcmState::new();
        assert_eq!(state.det, 32);
        assert_eq!(state.s, 0);
    }

    #[test]
    fn test_g722_state_initialization() {
        let state = G722State::new();
        assert_eq!(state.low_band.det, 32);
        assert_eq!(state.high_band.det, 32);
    }

    #[test]
    fn test_limit_function() {
        assert_eq!(limit(1000), 1000);
        assert_eq!(limit(40000), 32767);
        assert_eq!(limit(-40000), -32768);
    }

    // TODO: Add more comprehensive unit tests for each component
} 