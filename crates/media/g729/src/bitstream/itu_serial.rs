use std::io::{Read, Result, Write};
use std::vec::Vec;

use crate::bitstream::itu_params::{BITSNO, BITSNO2};

/// ITU-T serial sync word.
pub const SYNC_WORD: u16 = 0x6B21;
/// ITU-T serial `0` bit marker.
pub const BIT_0: u16 = 0x007F;
/// ITU-T serial `1` bit marker.
pub const BIT_1: u16 = 0x0081;

/// G.729 speech payload bit count.
pub const RATE_8000: i16 = 80;
/// G.729 Annex B SID payload bit count.
pub const RATE_SID: i16 = 15;
/// G.729 Annex B SID payload bit count in octet mode.
pub const RATE_SID_OCTET: i16 = 16;
/// G.729 Annex B no-data payload bit count.
pub const RATE_0: i16 = 0;

/// Convert packed payload bytes into ITU serial words with sync and size prefix.
pub fn frame_to_words(payload: &[u8], n_bits: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(2 + n_bits);
    out.push(SYNC_WORD);
    out.push(n_bits as u16);
    for i in 0..n_bits {
        let byte = payload.get(i / 8).copied().unwrap_or(0);
        let bit = (byte >> (7 - (i % 8))) & 1;
        out.push(if bit == 1 { BIT_1 } else { BIT_0 });
    }
    out
}

/// Convert ITU serial bit words into packed payload bytes.
pub fn words_to_frame(words: &[u16], n_bits: usize) -> Vec<u8> {
    let mut out = vec![0u8; n_bits.div_ceil(8)];
    for i in 0..n_bits {
        let word = words.get(i).copied().unwrap_or(BIT_0);
        let bit = if word == BIT_1 || word == 1 { 1u8 } else { 0u8 };
        out[i / 8] |= bit << (7 - (i % 8));
    }
    out
}

/// Parse a raw ITU serial stream into `(sync, bits)` frame tuples.
pub fn parse_stream(raw: &[u8]) -> Vec<(u16, Vec<u16>)> {
    let mut words = Vec::with_capacity(raw.len() / 2);
    for chunk in raw.chunks_exact(2) {
        words.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }

    let mut frames = Vec::new();
    let mut idx = 0usize;
    while idx + 1 < words.len() {
        let sync = words[idx];
        let size = words[idx + 1] as usize;
        idx += 2;
        if idx + size > words.len() {
            break;
        }
        frames.push((sync, words[idx..idx + size].to_vec()));
        idx += size;
    }
    frames
}

/// Serialize `(sync, bits)` tuples into raw ITU serial bytes.
pub fn serialize_stream(frames: &[(u16, Vec<u16>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (sync, bits) in frames {
        out.extend_from_slice(&sync.to_le_bytes());
        out.extend_from_slice(&(bits.len() as u16).to_le_bytes());
        for bit in bits {
            out.extend_from_slice(&bit.to_le_bytes());
        }
    }
    out
}

/// Read one frame from an ITU serial reader.
///
/// On success returns:
/// - `Ok(0)` on EOF
/// - `Ok(1)` when a frame was read
///
/// Output layout in `parm`:
/// - `parm[0]`: bad frame indicator (BFI)
/// - `parm[1]`: frame type code (0=no-data, 1=speech, 2=SID)
/// - `parm[2..]`: frame parameters
pub fn read_serial_frame<R: Read>(reader: &mut R, parm: &mut [i16], bfi: &mut i16) -> Result<i16> {
    let sync = match read_i16_le(reader) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(0),
        Err(e) => return Err(e),
    };
    let size = match read_i16_le(reader) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(0),
        Err(e) => return Err(e),
    };

    let num_bits = size as usize;
    let mut serial = [0i16; 82];
    serial[0] = size;
    for i in 0..num_bits {
        serial[i + 1] = read_i16_le(reader)?;
    }

    bits2prm_ld8k_full(&serial, parm);

    parm[0] = 0;
    if size != 0 {
        for i in 0..num_bits {
            if serial[i + 1] == 0 {
                parm[0] = 1;
                break;
            }
        }
    } else if sync != SYNC_WORD as i16 {
        parm[0] = 1;
    }
    *bfi = parm[0];

    Ok(1)
}

/// Write one frame in ITU serial format.
pub fn write_serial_frame<W: Write>(writer: &mut W, parm: &[i16], frame_size: i16) -> Result<()> {
    write_i16_le(writer, SYNC_WORD as i16)?;
    write_i16_le(writer, frame_size)?;

    if frame_size == RATE_8000 {
        let mut bits = [0i16; 80];
        let mut bit_offset = 0usize;
        for i in 0..11 {
            let nbits = BITSNO[i] as usize;
            int2bin_local(parm[i], BITSNO[i], &mut bits[bit_offset..]);
            bit_offset += nbits;
        }
        for bit in bits {
            write_i16_le(writer, bit)?;
        }
    } else if frame_size == RATE_SID || frame_size == RATE_SID_OCTET {
        let mut bits = [BIT_0 as i16; 16];
        let mut bit_offset = 0usize;
        for i in 0..4 {
            let nbits = BITSNO2[i] as usize;
            int2bin_local(parm[i], BITSNO2[i], &mut bits[bit_offset..]);
            bit_offset += nbits;
        }
        for bit in bits.iter().take(frame_size as usize) {
            write_i16_le(writer, *bit)?;
        }
    }

    Ok(())
}

fn bits2prm_ld8k_full(serial: &[i16], parm: &mut [i16]) {
    let nb_bits = serial[0];
    if nb_bits == RATE_8000 {
        parm[1] = 1;
        let mut bit_offset = 1usize;
        for i in 0..11 {
            parm[i + 2] = bin2int_local(BITSNO[i], &serial[bit_offset..]);
            bit_offset += BITSNO[i] as usize;
        }
    } else if nb_bits == RATE_SID || nb_bits == RATE_SID_OCTET {
        parm[1] = 2;
        let mut bit_offset = 1usize;
        for i in 0..4 {
            parm[i + 2] = bin2int_local(BITSNO2[i], &serial[bit_offset..]);
            bit_offset += BITSNO2[i] as usize;
        }
    } else {
        parm[1] = 0;
    }
}

fn bin2int_local(n_bits: u8, bits: &[i16]) -> i16 {
    let mut value = 0i16;
    for bit in bits.iter().take(n_bits as usize) {
        value <<= 1;
        if *bit == BIT_1 as i16 {
            value += 1;
        }
    }
    value
}

fn int2bin_local(value: i16, n_bits: u8, bits: &mut [i16]) {
    let mut v = value;
    for i in (0..n_bits as usize).rev() {
        bits[i] = if (v & 1) == 0 {
            BIT_0 as i16
        } else {
            BIT_1 as i16
        };
        v >>= 1;
    }
}

fn read_i16_le<R: Read>(reader: &mut R) -> Result<i16> {
    let mut buf = [0u8; 2];
    reader.read_exact(&mut buf)?;
    Ok(i16::from_le_bytes(buf))
}

fn write_i16_le<W: Write>(writer: &mut W, value: i16) -> Result<()> {
    writer.write_all(&value.to_le_bytes())
}

#[cfg(test)]
#[path = "itu_serial_tests.rs"]
mod tests;
