// Provenance: this file is substantially similar to this project's own code in `src/dxp_and_grp/dynmodel.rs (read_block, unpack_block)` of
// the unmerged wt_blk pull requests #28 and #29 (github.com/patodois/wt_blk, commit 498c996), after which it was
// written. That code follows the Dagor Engine sources (BSD-3-Clause, github.com/GaijinEntertainment/DagorEngine).
// No line of wt_blk's upstream author is in it.
// See the README, "Credit and provenance".

//! Length-prefixed blocks, the unit Dagor's resource streams pack their large parts in.

use crate::error::{Error, Result};
use crate::reader::Reader;
use crate::unpack;

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// A block's length is in the low 30 bits of its first `u32`; the top two bits are a tag.
const LENGTH_MASK: u32 = 0x3FFF_FFFF;

/// Reads a block: its `u32` length word, then that many bytes.
pub(crate) fn read_block<'a>(r: &mut Reader<'a>) -> Result<&'a [u8]> {
    let word = r.u32()?;
    r.take((word & LENGTH_MASK) as usize)
}

/// Unpacks the contents of a packed block: a zstd frame, or the unpacked size as a `u32` and then an
/// Oodle stream. The contents are at most `max` bytes: a caller that knows how long they have to be
/// says so, and a block that unpacks to more is refused before it is unpacked in full.
pub(crate) fn unpack_block(block: &[u8], max: usize) -> Result<Vec<u8>> {
    if block.starts_with(&ZSTD_MAGIC) {
        return unpack::zstd(block, max);
    }
    let mut r = Reader::new(block);
    let size = r.size()?;
    if size > max {
        return Err(Error::Decompress(format!("a block of {size} bytes where at most {max} are possible")));
    }
    unpack::oodle(r.rest(), size)
}
