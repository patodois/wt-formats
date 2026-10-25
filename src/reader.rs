// Provenance: this file is substantially similar to this project's own code in `src/dxp_and_grp/cursor.rs` of
// the unmerged wt_blk pull requests #28 and #29 (github.com/patodois/wt_blk, commit 498c996), after which it was
// written. That code follows the Dagor Engine sources (BSD-3-Clause, github.com/GaijinEntertainment/DagorEngine).
// No line of wt_blk's upstream author is in it.
// See the README, "Credit and provenance".

//! Bounds-checked reading of little-endian structures, and the checks that keep a count read from a
//! file from sizing an allocation before the data behind it is known to exist.

use std::cell::Cell;

use crate::error::{Error, Result};

/// A position in a byte slice. Every read checks the slice's end and fails with
/// [`Error::OutOfBounds`] instead of panicking.
#[derive(Clone)]
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    /// A reader positioned at `pos`, which may be the end of the slice but not past it.
    pub fn at(buf: &'a [u8], pos: usize) -> Result<Self> {
        if pos > buf.len() {
            return Err(Error::OutOfBounds { at: pos, len: buf.len() });
        }
        Ok(Reader { buf, pos })
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n);
        match end.and_then(|end| self.buf.get(self.pos..end)) {
            Some(bytes) => {
                self.pos += n;
                Ok(bytes)
            }
            None => Err(Error::OutOfBounds { at: end.unwrap_or(usize::MAX), len: self.buf.len() }),
        }
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let rest = &self.buf[self.pos..];
        self.pos = self.buf.len();
        rest
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub fn u16(&mut self) -> Result<u16> {
        self.array().map(u16::from_le_bytes)
    }

    pub fn u32(&mut self) -> Result<u32> {
        self.array().map(u32::from_le_bytes)
    }

    pub fn f32(&mut self) -> Result<f32> {
        self.array().map(f32::from_le_bytes)
    }

    /// A `u32` as a `usize`, for sizes and offsets.
    pub fn size(&mut self) -> Result<usize> {
        let v = self.u32()?;
        usize::try_from(v).map_err(|_| Error::invalid("a 32-bit size does not fit in memory"))
    }

    /// A 64-bit field holding an offset or a size. Dagor's dumps are written by a 64-bit program that
    /// stores its pointers there, as offsets once the dump is relocated.
    pub fn wide(&mut self) -> Result<usize> {
        let v = u64::from_le_bytes(self.array()?);
        usize::try_from(v).map_err(|_| Error::invalid(format!("offset {v} does not fit in memory")))
    }

    /// A NUL-terminated string. Bytes that are not UTF-8 become U+FFFD.
    pub fn cstr(&mut self) -> Result<String> {
        let rest = &self.buf[self.pos..];
        let len = rest.iter().position(|&b| b == 0).ok_or_else(|| Error::invalid("a string has no end"))?;
        self.pos += len + 1;
        Ok(String::from_utf8_lossy(&rest[..len]).into_owned())
    }
}

/// Checks that `count` records of `size` bytes starting at `ofs` lie inside `buf`. An empty table may
/// point anywhere.
pub(crate) fn check_table(buf: &[u8], ofs: usize, count: usize, size: usize) -> Result<()> {
    if count == 0 {
        return Ok(());
    }
    match count.checked_mul(size).and_then(|n| n.checked_add(ofs)) {
        Some(end) if end <= buf.len() => Ok(()),
        end => Err(Error::OutOfBounds { at: end.unwrap_or(usize::MAX), len: buf.len() }),
    }
}

/// Caps the bytes of names copied out of a buffer at the buffer's own size.
///
/// A real file stores each name once and refers to it a few times. A hostile one can point every
/// entry of a table at one long string, so that copying names takes memory in proportion to the
/// square of the file.
pub(crate) struct NameBudget {
    size: usize,
    left: Cell<usize>,
}

impl NameBudget {
    pub fn new(buf: &[u8]) -> Self {
        NameBudget { size: buf.len(), left: Cell::new(buf.len()) }
    }

    /// Accounts for one copy of `name`.
    pub fn take(&self, name: String) -> Result<String> {
        match self.left.get().checked_sub(name.len()) {
            Some(left) => {
                self.left.set(left);
                Ok(name)
            }
            None => Err(Error::invalid(format!("names add up to more than the {} bytes they are stored in", self.size))),
        }
    }
}
