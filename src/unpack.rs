// Provenance: this file is substantially similar to this project's own code in `src/dxp_and_grp/unpack.rs` of
// the unmerged wt_blk pull requests #28 and #29 (github.com/patodois/wt_blk, commit 498c996), after which it was
// written. That code follows the Dagor Engine sources (BSD-3-Clause, github.com/GaijinEntertainment/DagorEngine).
// No line of wt_blk's upstream author is in it.
// See the README, "Credit and provenance".

//! Unpacking with sizes that come from file headers, which may lie.
//!
//! None of these functions allocates the size a header claims before the stream has shown that it can
//! produce that much: output grows as data comes out, and nothing ever grows past [`MAX_UNPACKED`].

use std::io::Read;

use crate::error::{Error, Result};

/// The most bytes any single block unpacks to (256 MiB). The largest in War Thunder 2.59.0.80 is a texture
/// of 89,128,960 bytes; the largest vertex data of a model is 32,211,452 (docs/FORMATS.md).
pub const MAX_UNPACKED: usize = 256 << 20;

/// Oodle's blocks start every 256 KiB of output, so a stream decoded into a buffer that ends on a
/// block boundary yields the same bytes as the start of the full decode.
const OODLE_BLOCK: usize = 0x4_0000;
/// How much output the first attempt at an Oodle stream has room for.
const OODLE_FIRST_TRY: usize = 16 * OODLE_BLOCK;

fn too_big(size: usize) -> Error {
    Error::Decompress(format!("{size} bytes is more than the {MAX_UNPACKED} a block may unpack to"))
}

/// Unpacks an Oodle stream to exactly `size` bytes.
///
/// Oodle's decoder needs its whole output buffer up front, so a header's `size` is not trusted: the
/// stream is first decoded into a small buffer, and the buffer doubles only while the stream keeps
/// producing output, so a stream that is short or broken fails before the claimed size is allocated.
pub(crate) fn oodle(packed: &[u8], size: usize) -> Result<Vec<u8>> {
    if size > MAX_UNPACKED {
        return Err(too_big(size));
    }
    let mut room = size.min(OODLE_FIRST_TRY);
    loop {
        let mut out = vec![0u8; room];
        // lztans fills the whole buffer or fails; it does not panic on malformed input
        lztans::decode(packed, &mut out).map_err(|e| Error::Decompress(format!("Oodle: {e:?}")))?;
        if room == size {
            return Ok(out);
        }
        room = size.min(room * 2);
    }
}

/// The window a zstd frame asks its decoder to keep, from the frame header alone: the decoder allocates
/// that much when it starts, whatever the frame really holds. `None` when `packed` does not begin with a
/// frame header this function understands.
fn zstd_window(packed: &[u8]) -> Option<u64> {
    if packed.get(..4)? != [0x28, 0xB5, 0x2F, 0xFD] {
        return None;
    }
    let descriptor = *packed.get(4)?;
    let single_segment = descriptor & 0x20 != 0;
    if !single_segment {
        // A window descriptor: a power of two from 1 KiB and an eighth of it times 0 to 7 more
        let w = *packed.get(5)?;
        let base = 1u64 << (10 + u32::from(w >> 3));
        return Some(base + base / 8 * u64::from(w & 7));
    }
    // A single segment: the window is the content size, which follows the (absent) window descriptor and the
    // dictionary id
    let dictionary = [0usize, 1, 2, 4][usize::from(descriptor & 3)];
    let at = 5 + dictionary;
    let size = match descriptor >> 6 {
        0 => u64::from(*packed.get(at)?),
        1 => u64::from(u16::from_le_bytes(packed.get(at..at + 2)?.try_into().ok()?)) + 256,
        2 => u64::from(u32::from_le_bytes(packed.get(at..at + 4)?.try_into().ok()?)),
        _ => u64::from_le_bytes(packed.get(at..at + 8)?.try_into().ok()?),
    };
    Some(size)
}

/// The largest window of a zstd frame that is unpacked into `max` bytes or less: what the decoder allocates
/// for its window follows the frame header, not the data, so it is limited too. Frames that are not
/// compressed in one piece can have windows a good deal larger than their content; 8 MiB is what zstd's
/// default levels use.
fn zstd_window_limit(max: usize) -> u64 {
    (2 * max as u64).max(8 << 20)
}

/// Unpacks zstd data of at most `max` bytes. Only the first frame is read.
pub(crate) fn zstd(packed: &[u8], max: usize) -> Result<Vec<u8>> {
    let max = max.min(MAX_UNPACKED);
    match zstd_window(packed) {
        Some(window) if window <= zstd_window_limit(max) => {}
        Some(window) => {
            return Err(Error::Decompress(format!("a zstd frame with a window of {window} bytes, for at most {max} bytes of data")))
        }
        None => return Err(Error::Decompress("not a zstd frame".to_owned())),
    }
    let mut out = Vec::new();
    let decoder = ruzstd::decoding::StreamingDecoder::new(packed).map_err(|e| Error::Decompress(format!("zstd: {e}")))?;
    decoder.take(max as u64 + 1).read_to_end(&mut out).map_err(|e| Error::Decompress(format!("zstd: {e}")))?;
    if out.len() > max {
        return Err(Error::Decompress(format!("zstd data unpacks to more than {max} bytes")));
    }
    Ok(out)
}

/// Unpacks a zlib stream. Deflate cannot expand data more than 1032 times, which is the limit.
pub(crate) fn zlib(packed: &[u8]) -> Result<Vec<u8>> {
    let max = packed.len().saturating_mul(1032).saturating_add(64).min(MAX_UNPACKED);
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(packed)
        .take(max as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| Error::Decompress(format!("zlib: {e}")))?;
    if out.len() > max {
        return Err(Error::Decompress("zlib data unpacks to more than it can".to_owned()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An Oodle stream of stored blocks: a two-byte block header, then 256 KiB as they are
    fn stored(data: &[u8]) -> Vec<u8> {
        data.chunks(OODLE_BLOCK).flat_map(|block| [&[0x4C, 0x06][..], block].concat()).collect()
    }

    #[test]
    fn oodle_output_grows_with_the_stream() {
        // 20 blocks: tried as 16 blocks, then in full
        let data: Vec<u8> = (0..20 * OODLE_BLOCK + 100).map(|i| (i / 7) as u8).collect();
        let packed = stored(&data);
        assert_eq!(oodle(&packed, data.len()).unwrap(), data);
        assert_eq!(oodle(&packed, 3).unwrap(), data[..3]);
        // A stream that stops after 17 blocks while the header says 40 fails
        let short = &packed[..17 * (OODLE_BLOCK + 2)];
        assert!(oodle(short, 40 * OODLE_BLOCK).is_err());
        assert!(oodle(&[0x4C, 0x06, 0, 0, 0, 0], OODLE_BLOCK + 1).is_err());
        assert!(oodle(&[0x8C, 0x06, 0, 0, 0, 0], usize::MAX).is_err());
        assert!(oodle(&[], 1).is_err());
        // A stored block of one byte is a whole, valid stream
        assert_eq!(oodle(&[0x4C, 0x06, 0x41], 1).unwrap(), [0x41]);
    }

    #[test]
    fn zstd_is_bounded() {
        let packed = zstd::bulk::compress(&[7u8; 1000], 0).unwrap();
        assert_eq!(zstd(&packed, 1000).unwrap(), [7u8; 1000]);
        assert_eq!(zstd(&packed, 5000).unwrap(), [7u8; 1000]);
        assert!(zstd(&packed, 999).is_err());
        assert!(zstd(&packed[..packed.len() - 3], 1000).is_err());
    }

    #[test]
    fn zstd_windows_come_from_the_frame_header() {
        let magic = [0x28, 0xB5, 0x2F, 0xFD];
        let frame = |rest: &[u8]| [&magic[..], rest].concat();
        // Window descriptor: exponent 0 is 1 KiB; exponent 12 with mantissa 4 is 4 MiB and a half of it more
        assert_eq!(zstd_window(&frame(&[0x00, 0x00])), Some(1024));
        assert_eq!(zstd_window(&frame(&[0x00, 12 << 3 | 4])), Some(4 << 20).map(|b| b + b / 2));
        // Single segment: the window is the content size, 1 byte, 2 bytes plus 256, 4 bytes, 8 bytes
        assert_eq!(zstd_window(&frame(&[0x20, 200])), Some(200));
        assert_eq!(zstd_window(&frame(&[0x60, 0x01, 0x02])), Some(0x201 + 256));
        assert_eq!(zstd_window(&frame(&[0xA0, 0x00, 0x00, 0x10, 0x00])), Some(0x10_0000));
        assert_eq!(zstd_window(&frame(&[0xE0, 0, 0, 0, 0, 1, 0, 0, 0])), Some(1 << 32));
        // With a dictionary id of 1 byte in between
        assert_eq!(zstd_window(&frame(&[0x21, 7, 200])), Some(200));
        // Not frames, or cut
        assert_eq!(zstd_window(&[]), None);
        assert_eq!(zstd_window(&frame(&[])), None);
        assert_eq!(zstd_window(&frame(&[0x60, 0x01])), None);
        assert_eq!(zstd_window(b"not a frame"), None);
        // A real frame of a megabyte made in one go
        let packed = zstd::bulk::compress(&vec![5u8; 1 << 20], 3).unwrap();
        assert!(zstd_window(&packed).unwrap() <= zstd_window_limit(1 << 20));
        assert_eq!(zstd(&packed, 1 << 20).unwrap().len(), 1 << 20);
    }

    #[test]
    fn zlib_is_bounded() {
        use std::io::Write;
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        enc.write_all(&vec![1u8; 100_000]).unwrap();
        let packed = enc.finish().unwrap();
        assert_eq!(zlib(&packed).unwrap(), vec![1u8; 100_000]);
        assert!(zlib(&packed[..10]).is_err());
        assert!(zlib(b"not a stream").is_err());
    }
}
