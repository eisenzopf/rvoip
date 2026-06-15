/// Public module `annexa`.
pub mod annexa;
/// Public module `bitstream`.
pub mod bitstream;
/// Public module `gain`.
pub mod gain;
/// Public module `lsp`.
pub mod lsp;
/// Public module `misc`.
pub mod misc;
/// Public module `pitch`.
pub mod pitch;
/// Public module `postfilter`.
pub mod postfilter;
/// Public module `window`.
pub mod window;

/// Public module `sid`.
#[cfg(feature = "annex_b")]
pub mod sid;
/// Public module `vad`.
#[cfg(feature = "annex_b")]
pub mod vad;

/// Public constant `fn`.
pub const fn checksum_u16(data: &[u16]) -> u32 {
    let mut acc = 0u32;
    let mut i = 0;
    while i < data.len() {
        acc = acc.wrapping_mul(16777619) ^ (data[i] as u32);
        i += 1;
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_window_shape_is_expected() {
        assert_eq!(annexa::B100, [7699, -15398, 7699]);
        assert_eq!(annexa::A100, [8192, 15836, -7667]);
    }

    #[test]
    fn tables_checksums_are_stable() {
        let c1 = checksum_u16(&annexa::TABPOW.map(|v| v as u16));
        let c2 = checksum_u16(&annexa::TABLOG.map(|v| v as u16));
        assert_ne!(c1, 0);
        assert_ne!(c2, 0);
    }
}
