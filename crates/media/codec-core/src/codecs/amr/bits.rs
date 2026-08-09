//! MSB-first bit reader and writer for AMR payload assembly.
//!
//! RFC 4867's bandwidth-efficient mode packs fields and speech data with no
//! octet alignment between them, so both the payload format and (later) the
//! codec's own frame-structure handling need bit-granular I/O.
//!
//! # Bit order convention
//!
//! Bits are written and read **most significant first**, matching network byte
//! order and every bit diagram in RFC 4867. A multi-bit field's most
//! significant bit occupies the earliest bit position in the stream.
//!
//! # Byte-slice convention
//!
//! Where a run of bits is carried in a `[u8]` — a speech frame's payload, for
//! instance — it is **left-aligned**: the first bit sits in bit 7 of byte 0,
//! and any unused trailing bits of the final byte are zero. A run of `n` bits
//! therefore occupies `n.div_ceil(8)` bytes. This is the same layout the
//! octet-aligned mode uses on the wire, so a frame's bytes are identical in
//! both framings and only their placement differs.

use crate::error::{CodecError, Result};

/// Accumulates bits MSB-first into a byte buffer.
#[derive(Debug, Default)]
pub struct BitWriter {
    buf: Vec<u8>,
    /// Number of bits written so far. `buf.len()` is always
    /// `bit_len.div_ceil(8)`.
    bit_len: usize,
}

impl BitWriter {
    /// Create an empty writer.
    pub const fn new() -> Self {
        Self {
            buf: Vec::new(),
            bit_len: 0,
        }
    }

    /// Bits written so far. Used by tests to assert field widths.
    #[cfg(test)]
    pub const fn bit_len(&self) -> usize {
        self.bit_len
    }

    /// Append the low `count` bits of `value`, most significant first.
    ///
    /// # Panics
    ///
    /// Panics if `count` exceeds 32. Callers pass literal widths from the RFC's
    /// bit diagrams, so a wider request is a programming error rather than
    /// something to surface at runtime.
    pub fn write_bits(&mut self, value: u32, count: u8) {
        assert!(count <= 32, "cannot write {count} bits from a u32");
        for i in (0..count).rev() {
            self.write_bit(value >> i & 1 == 1);
        }
    }

    /// Append a single bit.
    pub fn write_bit(&mut self, set: bool) {
        let offset = self.bit_len % 8;
        if offset == 0 {
            self.buf.push(0);
        }
        if set {
            // Fill from the most significant bit of the current byte.
            let last = self.buf.len() - 1;
            self.buf[last] |= 0x80 >> offset;
        }
        self.bit_len += 1;
    }

    /// Append `bit_count` bits taken from the left-aligned slice `data`.
    ///
    /// # Errors
    ///
    /// Returns an error when `data` is too short to supply `bit_count` bits.
    pub fn write_slice_bits(&mut self, data: &[u8], bit_count: usize) -> Result<()> {
        if data.len() < bit_count.div_ceil(8) {
            return Err(CodecError::BufferTooSmall {
                needed: bit_count.div_ceil(8),
                actual: data.len(),
            });
        }
        for index in 0..bit_count {
            let byte = data[index / 8];
            let set = byte >> (7 - index % 8) & 1 == 1;
            self.write_bit(set);
        }
        Ok(())
    }

    /// Pad with zero bits up to the next octet boundary.
    pub fn align_to_octet(&mut self) {
        while !self.bit_len.is_multiple_of(8) {
            self.write_bit(false);
        }
    }

    /// Finish, zero-padding to a whole number of octets.
    pub fn finish(mut self) -> Vec<u8> {
        self.align_to_octet();
        self.buf
    }
}

/// Reads bits MSB-first from a byte slice.
#[derive(Debug)]
pub struct BitReader<'a> {
    buf: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    /// Create a reader positioned at the first bit of `buf`.
    pub const fn new(buf: &'a [u8]) -> Self {
        Self { buf, bit_pos: 0 }
    }

    /// Bits not yet consumed.
    pub const fn remaining_bits(&self) -> usize {
        self.buf.len() * 8 - self.bit_pos
    }

    /// Read `count` bits into the low bits of a `u32`, most significant first.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::InvalidPayload`] when fewer than `count` bits
    /// remain — a truncated payload, which is the common malformed-input case.
    ///
    /// # Panics
    ///
    /// Panics if `count` exceeds 32, which would be a programming error.
    pub fn read_bits(&mut self, count: u8) -> Result<u32> {
        assert!(count <= 32, "cannot read {count} bits into a u32");
        if self.remaining_bits() < count as usize {
            return Err(CodecError::InvalidPayload {
                details: format!(
                    "truncated AMR payload: needed {count} bits, {} remain",
                    self.remaining_bits()
                ),
            });
        }
        let mut value = 0u32;
        for _ in 0..count {
            value = value << 1 | u32::from(self.read_bit_unchecked());
        }
        Ok(value)
    }

    /// Read one bit. Callers must have checked availability.
    fn read_bit_unchecked(&mut self) -> u8 {
        let byte = self.buf[self.bit_pos / 8];
        let bit = byte >> (7 - self.bit_pos % 8) & 1;
        self.bit_pos += 1;
        bit
    }

    /// Read `bit_count` bits into a fresh left-aligned buffer of
    /// `bit_count.div_ceil(8)` bytes, with trailing bits zeroed.
    ///
    /// # Errors
    ///
    /// Returns [`CodecError::InvalidPayload`] when fewer than `bit_count` bits
    /// remain.
    pub fn read_slice_bits(&mut self, bit_count: usize) -> Result<Vec<u8>> {
        if self.remaining_bits() < bit_count {
            return Err(CodecError::InvalidPayload {
                details: format!(
                    "truncated AMR payload: needed {bit_count} bits, {} remain",
                    self.remaining_bits()
                ),
            });
        }
        let mut out = vec![0u8; bit_count.div_ceil(8)];
        for index in 0..bit_count {
            if self.read_bit_unchecked() == 1 {
                out[index / 8] |= 0x80 >> (index % 8);
            }
        }
        Ok(out)
    }

    /// Skip forward to the next octet boundary.
    pub const fn align_to_octet(&mut self) {
        self.bit_pos = self.bit_pos.div_ceil(8) * 8;
    }

    /// Whether every remaining bit is zero.
    ///
    /// Test-only. RFC 4867 says a receiver must ignore padding, so this never
    /// rejects a payload — it exists to assert that *our writer* emits the zero
    /// padding the RFC requires.
    #[cfg(test)]
    pub fn remaining_bits_are_zero(&self) -> bool {
        let mut probe = Self {
            buf: self.buf,
            bit_pos: self.bit_pos,
        };
        (0..probe.remaining_bits()).all(|_| probe.read_bit_unchecked() == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_bits_most_significant_first() {
        let mut w = BitWriter::new();
        // 0b1010 then 0b11 -> 1010_11 then zero padding -> 0b1010_1100
        w.write_bits(0b1010, 4);
        w.write_bits(0b11, 2);
        assert_eq!(w.bit_len(), 6);
        assert_eq!(w.finish(), vec![0b1010_1100]);
    }

    #[test]
    fn writes_across_byte_boundaries() {
        let mut w = BitWriter::new();
        w.write_bits(0b1111_1111_1111, 12);
        assert_eq!(w.bit_len(), 12);
        // 12 ones, then 4 zero pad bits.
        assert_eq!(w.finish(), vec![0xFF, 0xF0]);
    }

    #[test]
    fn only_the_low_count_bits_are_written() {
        let mut w = BitWriter::new();
        // High bits above `count` must be ignored, not corrupt the stream.
        w.write_bits(0xFFFF_FFF5, 4);
        assert_eq!(w.finish(), vec![0b0101_0000]);
    }

    #[test]
    fn reads_back_what_was_written() {
        let mut w = BitWriter::new();
        w.write_bits(0b1010, 4);
        w.write_bits(0b110_0110, 7);
        w.write_bits(1, 1);
        let bytes = w.finish();

        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_bits(4).unwrap(), 0b1010);
        assert_eq!(r.read_bits(7).unwrap(), 0b110_0110);
        assert_eq!(r.read_bits(1).unwrap(), 1);
        // Only zero padding should remain.
        assert!(r.remaining_bits_are_zero());
    }

    #[test]
    fn reading_past_the_end_is_an_error_not_a_panic() {
        let bytes = [0xFFu8];
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_bits(8).unwrap(), 0xFF);
        assert_eq!(r.remaining_bits(), 0);
        let err = r.read_bits(1).unwrap_err();
        assert!(matches!(err, CodecError::InvalidPayload { .. }));
        assert!(r.read_slice_bits(1).is_err());
    }

    #[test]
    fn slice_bits_round_trip_at_non_octet_lengths() {
        // AMR frame sizes are mostly not multiples of 8, which is the whole
        // reason this module exists. 95 and 477 are the smallest NB and largest
        // WB speech frames.
        for bit_count in [1usize, 7, 8, 9, 39, 40, 95, 244, 477] {
            let byte_len = bit_count.div_ceil(8);
            // A recognisable pattern, with trailing bits of the last byte
            // zeroed as the left-aligned convention requires.
            let mut src = vec![0u8; byte_len];
            for (i, byte) in src.iter_mut().enumerate() {
                *byte = u8::try_from(i % 256).unwrap_or(0).wrapping_mul(37) | 0x81;
            }
            let tail = bit_count % 8;
            if tail != 0 {
                let mask = 0xFFu8 << (8 - tail);
                let last = byte_len - 1;
                src[last] &= mask;
            }

            let mut w = BitWriter::new();
            // Offset by 4 bits so the payload is not byte-aligned in the
            // stream, which is the bandwidth-efficient case.
            w.write_bits(0b1011, 4);
            w.write_slice_bits(&src, bit_count).unwrap();
            let bytes = w.finish();

            let mut r = BitReader::new(&bytes);
            assert_eq!(r.read_bits(4).unwrap(), 0b1011);
            let got = r.read_slice_bits(bit_count).unwrap();
            assert_eq!(got, src, "round trip failed at {bit_count} bits");
            assert!(
                r.remaining_bits_are_zero(),
                "padding not zero at {bit_count} bits"
            );
        }
    }

    #[test]
    fn write_slice_bits_rejects_a_short_source() {
        let mut w = BitWriter::new();
        let err = w.write_slice_bits(&[0xFF], 9).unwrap_err();
        assert!(matches!(err, CodecError::BufferTooSmall { .. }));
    }

    #[test]
    fn align_skips_to_the_octet_boundary() {
        let mut w = BitWriter::new();
        w.write_bits(0b101, 3);
        w.align_to_octet();
        assert_eq!(w.bit_len(), 8);
        w.write_bits(0xAB, 8);
        assert_eq!(w.finish(), vec![0b1010_0000, 0xAB]);

        let bytes = [0b1010_0000u8, 0xAB];
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_bits(3).unwrap(), 0b101);
        r.align_to_octet();
        assert_eq!(r.read_bits(8).unwrap(), 0xAB);

        // Aligning when already aligned must not consume a byte.
        let mut r = BitReader::new(&bytes);
        r.align_to_octet();
        assert_eq!(r.remaining_bits(), 16);
    }

    #[test]
    fn non_zero_padding_is_detectable() {
        // A sender that miscounts frame lengths typically strands bits in the
        // padding; that is cheap to notice.
        let bytes = [0b1010_0001u8];
        let mut r = BitReader::new(&bytes);
        r.read_bits(4).unwrap();
        assert!(!r.remaining_bits_are_zero());

        let bytes = [0b1010_0000u8];
        let mut r = BitReader::new(&bytes);
        r.read_bits(4).unwrap();
        assert!(r.remaining_bits_are_zero());
    }

    #[test]
    fn probing_padding_does_not_advance_the_reader() {
        let bytes = [0b1010_0000u8];
        let mut r = BitReader::new(&bytes);
        r.read_bits(4).unwrap();
        let before = r.remaining_bits();
        assert!(r.remaining_bits_are_zero());
        assert_eq!(r.remaining_bits(), before);
        assert_eq!(r.read_bits(4).unwrap(), 0);
    }
}
