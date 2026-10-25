// Provenance: this file reads a layout that wt_blk documented first, and it is substantially similar to
// wt_blk's `src/dxp_and_grp/meshopt.rs` (github.com/Warthunder-Open-Source-Foundation/wt_blk, fork
// github.com/patodois/wt_blk at 498c996). That code was written by this project's author (wt_blk pull requests
// #28 and #29); no line of it is by wt_blk's upstream author. wt_blk declares MIT in its Cargo.toml and
// ships an Apache-2.0 LICENSE file; both are compatible with this crate's MIT OR Apache-2.0. See the README,
// "Provenance", for how the similarity was measured.

// The format is meshoptimizer's `meshopt_encodeIndexSequence` (MIT, Arseny Kapoulkine).

//! Packed index buffers: the format of meshoptimizer's `meshopt_encodeIndexSequence`.
//!
//! Dagor stores the index buffers of many models this way (vertex buffers flagged `0x200`).
//! The layout is documented in [`docs/FORMATS.md`](https://github.com/patodois/wt-formats/blob/main/docs/FORMATS.md):
//! a header byte `0xD0 | version`, one variable-length integer per index, and a 4-byte tail.

use crate::error::{Error, Result};

const HEADER_TAG: u8 = 0xD0;
const NEWEST_VERSION: u8 = 1;
const TAIL: usize = 4;

/// Decodes `count` indices. With `wide` they are 32-bit; otherwise they wrap to 16 bits, as the
/// buffer they belong to stores them.
pub fn decode_index_sequence(data: &[u8], count: usize, wide: bool) -> Result<Vec<u32>> {
    let bad = |what: &str| Error::invalid(format!("packed index buffer: {what}"));
    // The header byte, at least one byte per index, and the tail
    let body = data.len().checked_sub(1 + TAIL).ok_or_else(|| bad("too short"))?;
    if body < count {
        return Err(bad("too short"));
    }
    if data[0] & 0xF0 != HEADER_TAG || data[0] & 0x0F > NEWEST_VERSION {
        return Err(bad("unknown header"));
    }

    let stream = &data[1..1 + body];
    let mut at = 0;
    // The two bases a delta can be taken from: the last index coded against each
    let mut base = [0u32; 2];
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        // Seven bits per byte, low group first, five bytes at most: the fifth byte's top bit is not a
        // continuation bit
        let mut value = 0u32;
        for group in 0..5 {
            let byte = *stream.get(at).ok_or_else(|| bad("truncated"))?;
            at += 1;
            value |= u32::from(byte & 0x7F) << (7 * group);
            if byte < 0x80 {
                break;
            }
        }
        // Bit 0 picks the base, the rest is a zigzag-coded delta
        let which = (value & 1) as usize;
        let zigzag = value >> 1;
        let delta = (zigzag >> 1) ^ (zigzag & 1).wrapping_neg();
        let index = base[which].wrapping_add(delta);
        base[which] = index;
        out.push(if wide { index } else { index & 0xFFFF });
    }
    if at != stream.len() {
        return Err(bad("trailing data"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The encoder, to round-trip against: each index is coded against whichever base is closer
    fn encode(indices: &[u32]) -> Vec<u8> {
        let mut out = vec![HEADER_TAG | 1];
        let mut base = [0u32; 2];
        for &index in indices {
            let d = [index.wrapping_sub(base[0]) as i32, index.wrapping_sub(base[1]) as i32];
            let which = usize::from(d[1].unsigned_abs() < d[0].unsigned_abs());
            let zigzag = ((d[which] << 1) ^ (d[which] >> 31)) as u32;
            let mut v = (zigzag << 1) | which as u32;
            loop {
                let low = (v & 0x7F) as u8;
                v >>= 7;
                if v == 0 {
                    out.push(low);
                    break;
                }
                out.push(low | 0x80);
            }
            base[which] = index;
        }
        out.extend_from_slice(&[0; TAIL]);
        out
    }

    #[test]
    fn round_trip() {
        let indices = [0, 1, 2, 2, 1, 3, 100, 5, 70_000, 69_999, 4, 0, u32::MAX, 0];
        let packed = encode(&indices);
        assert_eq!(decode_index_sequence(&packed, indices.len(), true).unwrap(), indices);
        let narrow: Vec<u32> = indices.iter().map(|i| i & 0xFFFF).collect();
        assert_eq!(decode_index_sequence(&packed, indices.len(), false).unwrap(), narrow);
        assert_eq!(decode_index_sequence(&encode(&[]), 0, true).unwrap(), Vec::<u32>::new());
    }

    #[test]
    fn a_known_stream() {
        // Four integers: 0x00 is base 0 with delta 0 (index 0); 0x04 is base 0 with zigzag 2, delta +1, so each of
        // the next three is one more than the last index coded against base 0
        let packed = [0xD1, 0x00, 0x04, 0x04, 0x04, 0, 0, 0, 0];
        assert_eq!(decode_index_sequence(&packed, 4, true).unwrap(), [0, 1, 2, 3]);
    }

    #[test]
    fn rejects_bad_input() {
        let packed = encode(&[0, 1, 2]);
        assert!(decode_index_sequence(&packed[..packed.len() - 1], 3, true).is_err());
        assert!(decode_index_sequence(&packed, 2, true).is_err());
        assert!(decode_index_sequence(&packed, 4, true).is_err());
        for bad in [0xE1, 0xD2, 0x00] {
            let mut p = packed.clone();
            p[0] = bad;
            assert!(decode_index_sequence(&p, 3, true).is_err());
        }
        for count in [usize::MAX, usize::MAX - 4, usize::MAX / 2] {
            assert!(decode_index_sequence(&packed, count, true).is_err());
        }
        assert!(decode_index_sequence(&[], 0, true).is_err());
        // A value that runs on past the end of the stream
        assert!(decode_index_sequence(&[0xD1, 0x80, 0x80, 0, 0, 0, 0], 1, true).is_err());
    }
}
