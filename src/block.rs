// Provenance: this file reads a layout that wt_blk documented first, and it is substantially similar to
// wt_blk's `src/dxp_and_grp/dynmodel.rs (read_block, unpack_block)` (github.com/Warthunder-Open-Source-Foundation/wt_blk, fork
// github.com/patodois/wt_blk at 498c996). That code was written by this project's author (wt_blk pull requests
// #28 and #29); no line of it is by wt_blk's upstream author. wt_blk declares MIT in its Cargo.toml and
// ships an Apache-2.0 LICENSE file; both are compatible with this crate's MIT OR Apache-2.0. See the README,
// "Provenance", for how the similarity was measured.

//! Length-prefixed blocks, the unit Dagor's resource streams pack their large parts in.

use crate::error::Result;
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
/// Oodle stream.
pub(crate) fn unpack_block(block: &[u8]) -> Result<Vec<u8>> {
    if block.starts_with(&ZSTD_MAGIC) {
        return unpack::zstd(block, unpack::MAX_UNPACKED);
    }
    let mut r = Reader::new(block);
    let size = r.size()?;
    unpack::oodle(r.rest(), size)
}
