// Provenance: this file is substantially similar to this project's own code in `src/dxp_and_grp/dxp.rs` of
// the unmerged wt_blk pull requests #28 and #29 (github.com/patodois/wt_blk, commit 498c996), after which it was
// written. That code follows the Dagor Engine sources (BSD-3-Clause, github.com/GaijinEntertainment/DagorEngine).
// The fork's `dxp.rs` also holds 57 lines by wt_blk's upstream author (a function that lists the names of a
// pack); none of them is used here.
// See the README, "Credit and provenance".

//! DXP packs: Dagor's texture packs (`*.dxp.bin`), which hold [DDSx](crate::ddsx) textures by name.
//!
//! See `docs/FORMATS.md` for the layout.

use crate::ddsx::{DdsxHeader, Texture};
use crate::error::{Error, Result};
use crate::reader::{check_table, NameBudget, Reader};

const LABEL: &[u8; 4] = b"DxP2";
/// Bytes before the description dump that holds the tables
const FILE_HEADER: usize = 0x10;
/// Bytes in a record of the third table
const RECORD: usize = 24;

/// One texture of a pack.
#[derive(Debug, Clone)]
pub struct DxpEntry {
    /// The texture's name, such as `abandoned_wall_tex_d*`; names may carry a `?options` suffix.
    pub name: String,
    /// The header of the texture.
    pub header: DdsxHeader,
    offset: usize,
    size: usize,
}

/// A parsed DXP file, borrowing the bytes it was parsed from.
pub struct DxpPack<'a> {
    file: &'a [u8],
    /// The textures, in the order of the pack's tables.
    pub entries: Vec<DxpEntry>,
}

impl<'a> DxpPack<'a> {
    /// Reads the tables of a `DxP2` file. No texture data is read or unpacked.
    pub fn parse(file: &'a [u8]) -> Result<Self> {
        let mut head = Reader::new(file);
        let label = head.take(4)?;
        if label != LABEL {
            return Err(Error::BadLabel { found: String::from_utf8_lossy(label).into_owned() });
        }
        let version = head.u32()?;
        if version != 2 {
            return Err(Error::unsupported(format!("DXP version {version}")));
        }
        head.skip(4)?;
        let dump_size = head.size()?;
        let dump = file
            .get(FILE_HEADER..)
            .and_then(|rest| rest.get(..dump_size))
            .ok_or(Error::OutOfBounds { at: dump_size.saturating_add(FILE_HEADER), len: file.len() })?;

        // Three tables of {u32 offset, u32 count}, 16 bytes apart, offsets from the start of the dump
        let table = |at: usize| -> Result<(usize, usize)> {
            let mut r = Reader::at(dump, at)?;
            Ok((r.size()?, r.size()?))
        };
        let (names_at, count) = table(0x00)?;
        let (headers_at, header_count) = table(0x10)?;
        let (records_at, record_count) = table(0x20)?;
        if header_count != count || record_count != count {
            return Err(Error::unsupported(format!(
                "tables of different lengths: {count} names, {header_count} headers, {record_count} records"
            )));
        }
        check_table(dump, names_at, count, 8)?;
        check_table(dump, headers_at, count, DdsxHeader::SIZE)?;
        check_table(dump, records_at, count, RECORD)?;

        let budget = NameBudget::new(dump);
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let name_at = Reader::at(dump, names_at + i * 8)?.wide()?;
            let name = budget.take(Reader::at(dump, name_at)?.cstr()?)?;
            let header = DdsxHeader::parse(Reader::at(dump, headers_at + i * DdsxHeader::SIZE)?.rest())?;
            // The record holds where the texture's data is in the file, and how long it is packed
            let mut record = Reader::at(dump, records_at + i * RECORD + 0x0C)?;
            let offset = record.size()?;
            let packed = record.size()?;
            let size = if packed > 0 { packed } else { header.mem_size as usize };
            entries.push(DxpEntry { name, header, offset, size });
        }
        Ok(DxpPack { file, entries })
    }

    /// Unpacks the texture at `index` of [`entries`](Self::entries).
    pub fn texture(&self, index: usize) -> Result<Texture> {
        let entry = self.entries.get(index).ok_or(Error::OutOfBounds { at: index, len: self.entries.len() })?;
        let end = entry.offset.saturating_add(entry.size);
        let raw = self.file.get(entry.offset..end).ok_or(Error::OutOfBounds { at: end, len: self.file.len() })?;
        Texture::from_ddsx(entry.header, raw)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn put(buf: &mut [u8], at: usize, v: u32) {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// A DDSx header for a stored 4x4 DXT1 texture (8 bytes)
    fn header() -> [u8; 32] {
        let mut h = [0u8; 32];
        h[..4].copy_from_slice(b"DDSx");
        h[4..8].copy_from_slice(b"DXT1");
        h[0x0C..0x0E].copy_from_slice(&4u16.to_le_bytes());
        h[0x0E..0x10].copy_from_slice(&4u16.to_le_bytes());
        h[0x10] = 1;
        h[0x17] = 3;
        h[0x18..0x1C].copy_from_slice(&8u32.to_le_bytes());
        h
    }

    /// A pack of stored 4x4 textures with these names and fill bytes
    pub(crate) fn pack(items: &[(&str, u8)]) -> Vec<u8> {
        let n = items.len();
        // The dump: 0x30 bytes of table heads, name pointers, headers, records, then the names
        let names_at = 0x30;
        let headers_at = names_at + 8 * n;
        let records_at = headers_at + 32 * n;
        let strings_at = records_at + RECORD * n;
        let mut dump = vec![0u8; strings_at];
        for (table, at) in [(0x00, names_at), (0x10, headers_at), (0x20, records_at)] {
            put(&mut dump, table, at as u32);
            put(&mut dump, table + 4, n as u32);
        }
        let mut strings = Vec::new();
        for (i, (name, _)) in items.iter().enumerate() {
            let at = (strings_at + strings.len()) as u64;
            dump[names_at + 8 * i..names_at + 8 * i + 8].copy_from_slice(&at.to_le_bytes());
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
        }
        dump.extend_from_slice(&strings);
        let data_at = FILE_HEADER + dump.len();
        for i in 0..n {
            dump[headers_at + 32 * i..headers_at + 32 * i + 32].copy_from_slice(&header());
            put(&mut dump, records_at + RECORD * i + 0x0C, (data_at + 8 * i) as u32);
        }
        let mut file = LABEL.to_vec();
        for v in [2, n as u32, dump.len() as u32] {
            file.extend_from_slice(&v.to_le_bytes());
        }
        file.extend_from_slice(&dump);
        for (_, fill) in items {
            file.extend_from_slice(&[*fill; 8]);
        }
        file
    }

    #[test]
    fn entries_and_textures() {
        let file = pack(&[("one*", 1), ("two*?u03", 2)]);
        let p = DxpPack::parse(&file).unwrap();
        let names: Vec<_> = p.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["one*", "two*?u03"]);
        assert_eq!(p.entries[1].header.width, 4);
        assert_eq!(p.texture(0).unwrap().data, [1; 8]);
        assert_eq!(p.texture(1).unwrap().data, [2; 8]);
        assert!(p.texture(2).is_err());
        assert_eq!(DxpPack::parse(&pack(&[])).unwrap().entries.len(), 0);
    }

    #[test]
    fn rejects_bad_files() {
        let good = pack(&[("a", 1)]);
        assert!(DxpPack::parse(b"").is_err());
        assert!(DxpPack::parse(b"GRP2").is_err());
        assert!(DxpPack::parse(&good[..0x20]).is_err());
        let mut f = good.clone();
        put(&mut f, 4, 3);
        assert!(DxpPack::parse(&f).is_err());
        // A dump bigger than the file
        let mut f = good.clone();
        put(&mut f, 0xC, u32::MAX);
        assert!(DxpPack::parse(&f).is_err());
        // Counts that disagree, and counts that do not fit
        for table in [0x00, 0x10, 0x20] {
            let mut f = good.clone();
            put(&mut f, FILE_HEADER + table + 4, 2);
            assert!(DxpPack::parse(&f).is_err());
            let mut f = good.clone();
            for t in [0x00, 0x10, 0x20] {
                put(&mut f, FILE_HEADER + t + 4, u32::MAX);
            }
            assert!(DxpPack::parse(&f).is_err());
        }
        // A texture whose data is past the end of the file
        let mut f = good.clone();
        let records_at = FILE_HEADER + 0x30 + 8 + 32;
        put(&mut f, records_at + 0x0C, good.len() as u32);
        assert!(DxpPack::parse(&f).unwrap().texture(0).is_err());
    }

    #[test]
    fn one_long_name_cannot_be_copied_without_limit() {
        // Four textures whose names are all the same 1000-byte string
        let mut dump = vec![0u8; 0x30];
        for (table, at) in [(0x00, 0x30), (0x10, 0x50), (0x20, 0xD0)] {
            put(&mut dump, table, at);
            put(&mut dump, table + 4, 4);
        }
        for _ in 0..4 {
            dump.extend_from_slice(&0x130u64.to_le_bytes());
        }
        for _ in 0..4 {
            dump.extend_from_slice(b"DDSx");
            dump.extend_from_slice(&[0; 28]);
        }
        dump.extend_from_slice(&[0; 4 * RECORD]);
        dump.extend_from_slice(&[b'a'; 1000]);
        dump.push(0);
        let mut file = LABEL.to_vec();
        for v in [2, 4, dump.len() as u32] {
            file.extend_from_slice(&v.to_le_bytes());
        }
        file.extend_from_slice(&dump);
        assert!(DxpPack::parse(&file).is_err());
    }
}
