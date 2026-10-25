// Provenance: this file reads a layout that wt_blk documented first, and it is substantially similar to
// wt_blk's `src/dxp_and_grp/unpack.rs` (github.com/Warthunder-Open-Source-Foundation/wt_blk, fork
// github.com/patodois/wt_blk at 498c996). That code was written by this project's author (wt_blk pull requests
// #28 and #29); no line of it is by wt_blk's upstream author. wt_blk declares MIT in its Cargo.toml and
// ships an Apache-2.0 LICENSE file; both are compatible with this crate's MIT OR Apache-2.0. See the README,
// "Provenance", for how the similarity was measured.

//! Unpacking with sizes that come from file headers, which may lie.
//!
//! None of these functions allocates the size a header claims before the stream has shown that it can
//! produce that much: output grows as data comes out, and nothing ever grows past [`MAX_UNPACKED`].

use std::io::Read;
use std::panic::{self, AssertUnwindSafe};

use crate::error::{Error, Result};

/// The most bytes any single block unpacks to. Keeps a hostile header from asking for 4 GiB. The largest
/// texture in War Thunder 2.59.0.80 unpacks to 89,128,960 bytes (docs/FORMATS.md).
pub const MAX_UNPACKED: usize = 1 << 30;

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
        // The decoder panics on some malformed streams (a slice end past its input in the LZNA and
        // Bitknit paths, an assertion while building a Huffman table), so a panic is an error here
        let written = panic::catch_unwind(AssertUnwindSafe(|| oozextract::Extractor::new().read_from_slice(packed, &mut out)))
            .map_err(|_| Error::Decompress("the Oodle decoder panicked".to_owned()))?
            .map_err(|e| Error::Decompress(format!("Oodle: {e:?}")))?;
        if written != room {
            return Err(Error::Decompress(format!("Oodle stream gave {written} bytes, not {room}")));
        }
        if room == size {
            return Ok(out);
        }
        room = size.min(room * 2);
    }
}

/// Unpacks zstd data of at most `max` bytes.
pub(crate) fn zstd(packed: &[u8], max: usize) -> Result<Vec<u8>> {
    let max = max.min(MAX_UNPACKED);
    let mut out = Vec::new();
    zstd::stream::read::Decoder::new(packed)
        .and_then(|d| d.take(max as u64 + 1).read_to_end(&mut out))
        .map_err(|e| Error::Decompress(format!("zstd: {e}")))?;
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
