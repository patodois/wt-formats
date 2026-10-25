// Provenance: this file reads a layout that wt_blk documented first, and it is substantially similar to
// wt_blk's `src/dxp_and_grp/grp_pack.rs` (github.com/Warthunder-Open-Source-Foundation/wt_blk, fork
// github.com/patodois/wt_blk at 498c996). That code was written by this project's author (wt_blk pull requests
// #28 and #29); no line of it is by wt_blk's upstream author. wt_blk declares MIT in its Cargo.toml and
// ships an Apache-2.0 LICENSE file; both are compatible with this crate's MIT OR Apache-2.0. See the README,
// "Provenance", for how the similarity was measured.

//! GRP packs: Dagor's game resource packs (`*.grp`), which hold models, skeletons, collision and
//! placed-object resources by name and class.
//!
//! See `docs/FORMATS.md` for the layout.

use crate::error::{Error, Result};
use crate::reader::{check_table, NameBudget, Reader};

/// Class ids of the resources this crate or its users read.
pub mod class_id {
    /// A [`DynModel`](crate::dynmodel::DynModel): a vehicle's model.
    pub const DYN_MODEL: u32 = 0xB4B7_D9C4;
    /// A [`GeomNodeTree`](crate::geom_node_tree::GeomNodeTree): a skeleton.
    pub const GEOM_NODE_TREE: u32 = 0x56F8_1B6D;
    /// A placed object's model (`RenderableInstanceLodsResource`).
    pub const REND_INST: u32 = 0x77F8_232F;
}

/// One entry of a pack's resource table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpResource {
    /// The resource's name.
    pub name: String,
    /// What kind of resource it is, see [`class_id`].
    pub class_id: u32,
    offset: usize,
    size: usize,
}

/// A parsed GRP file, borrowing the bytes it was parsed from.
pub struct GrpPack<'a> {
    file: &'a [u8],
    /// Resources in the order of the pack's table.
    pub resources: Vec<GrpResource>,
}

impl<'a> GrpPack<'a> {
    /// Reads the name and resource tables of a `GRP2` or `GRP3` file.
    pub fn parse(file: &'a [u8]) -> Result<Self> {
        let mut head = Reader::new(file);
        let label = head.take(4)?;
        if label != b"GRP2" && label != b"GRP3" {
            return Err(Error::BadLabel { found: String::from_utf8_lossy(label).into_owned() });
        }
        // Two tables of {u32 offset, u32 count}, 16 bytes apart
        let table = |at: usize| -> Result<(usize, usize)> {
            let mut r = Reader::at(file, at)?;
            Ok((r.size()?, r.size()?))
        };
        let (names_at, names_count) = table(0x10)?;
        let (res_at, res_count) = table(0x20)?;
        check_table(file, names_at, names_count, 4)?;
        check_table(file, res_at, res_count, 12)?;

        let name_budget = NameBudget::new(file);
        let mut names = Vec::with_capacity(names_count);
        let mut r = Reader::at(file, names_at)?;
        for _ in 0..names_count {
            // An offset with the top bit set marks "no name"
            let at = r.u32()? as i32;
            names.push(match usize::try_from(at) {
                Ok(at) => name_budget.take(Reader::at(file, at)?.cstr()?)?,
                Err(_) => String::new(),
            });
        }

        let copy_budget = NameBudget::new(file);
        let mut starts = Vec::with_capacity(res_count);
        let mut r = Reader::at(file, res_at)?;
        for _ in 0..res_count {
            let class_id = r.u32()?;
            let offset = r.size()?;
            let name_id = usize::from(r.u16()?);
            r.skip(2)?;
            let name =
                names.get(name_id).ok_or_else(|| Error::invalid(format!("resource name {name_id} of {} is missing", names.len())))?;
            starts.push((copy_budget.take(name.clone())?, class_id, offset));
        }

        // A resource runs up to the start of the next one in the table, the last one to the end of the file
        let mut resources = Vec::with_capacity(starts.len());
        for (i, (name, class_id, offset)) in starts.iter().enumerate() {
            let end = starts.get(i + 1).map_or(file.len(), |next| next.2);
            if *offset > end || end > file.len() {
                return Err(Error::invalid(format!("resource {name} lies outside the file")));
            }
            resources.push(GrpResource { name: name.clone(), class_id: *class_id, offset: *offset, size: end - offset });
        }
        Ok(GrpPack { file, resources })
    }

    /// The bytes of a resource of this pack.
    pub fn data(&self, resource: &GrpResource) -> &'a [u8] {
        &self.file[resource.offset..resource.offset + resource.size]
    }

    /// The first resource with this name and class.
    pub fn find(&self, name: &str, class_id: u32) -> Option<&GrpResource> {
        self.resources.iter().find(|r| r.name == name && r.class_id == class_id)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn put(buf: &mut [u8], at: usize, v: u32) {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// A pack with the given (name, class, payload) resources
    pub(crate) fn pack(items: &[(&str, u32, &[u8])]) -> Vec<u8> {
        let mut out = vec![0u8; 0x40];
        out[..4].copy_from_slice(b"GRP2");
        let mut name_ofs = Vec::new();
        for (name, ..) in items {
            name_ofs.push(out.len() as u32);
            out.extend_from_slice(name.as_bytes());
            out.push(0);
        }
        while out.len() % 4 != 0 {
            out.push(0);
        }
        let names_at = out.len();
        for o in &name_ofs {
            out.extend_from_slice(&o.to_le_bytes());
        }
        let res_at = out.len();
        let data_at = res_at + 12 * items.len();
        let mut at = data_at;
        for (i, (_, class, data)) in items.iter().enumerate() {
            out.extend_from_slice(&class.to_le_bytes());
            out.extend_from_slice(&(at as u32).to_le_bytes());
            out.extend_from_slice(&(i as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            at += data.len();
        }
        for (.., data) in items {
            out.extend_from_slice(data);
        }
        put(&mut out, 0x10, names_at as u32);
        put(&mut out, 0x14, items.len() as u32);
        put(&mut out, 0x20, res_at as u32);
        put(&mut out, 0x24, items.len() as u32);
        out
    }

    #[test]
    fn resources_end_where_the_next_begins() {
        let file = pack(&[("a", class_id::DYN_MODEL, b"1234"), ("b", class_id::GEOM_NODE_TREE, b"56"), ("c", 7, b"")]);
        let p = GrpPack::parse(&file).unwrap();
        assert_eq!(p.resources.len(), 3);
        assert_eq!(p.data(p.find("a", class_id::DYN_MODEL).unwrap()), b"1234");
        assert_eq!(p.data(p.find("b", class_id::GEOM_NODE_TREE).unwrap()), b"56");
        assert_eq!(p.data(&p.resources[2]), b"");
        assert!(p.find("a", class_id::GEOM_NODE_TREE).is_none());
    }

    #[test]
    fn a_missing_name_reads_as_empty() {
        let mut file = pack(&[("a", 1, b"x")]);
        let names_at = u32::from_le_bytes(file[0x10..0x14].try_into().unwrap()) as usize;
        put(&mut file, names_at, u32::MAX);
        assert_eq!(GrpPack::parse(&file).unwrap().resources[0].name, "");
    }

    #[test]
    fn rejects_bad_files() {
        let good = pack(&[("a", 1, b"xyz")]);
        assert!(GrpPack::parse(b"").is_err());
        assert!(GrpPack::parse(b"GRP2").is_err());
        assert!(GrpPack::parse(b"DxP2....").is_err());
        assert!(GrpPack::parse(&good[..40]).is_err());
        // Counts that do not fit in the file
        for at in [0x14, 0x24] {
            let mut f = good.clone();
            put(&mut f, at, u32::MAX);
            assert!(GrpPack::parse(&f).is_err());
        }
        // A resource that starts past the end, and one that starts after the next
        let mut f = good.clone();
        let res_at = u32::from_le_bytes(f[0x20..0x24].try_into().unwrap()) as usize;
        let past = f.len() as u32 + 1;
        put(&mut f, res_at + 4, past);
        assert!(GrpPack::parse(&f).is_err());
        let mut f = pack(&[("a", 1, b"xyz"), ("b", 1, b"xyz")]);
        let res_at = u32::from_le_bytes(f[0x20..0x24].try_into().unwrap()) as usize;
        let late = f.len() as u32;
        put(&mut f, res_at + 4, late);
        assert!(GrpPack::parse(&f).is_err());
    }

    #[test]
    fn one_long_name_cannot_be_copied_without_limit() {
        // Eight name slots that all point at one 1000-byte string
        let mut f = vec![0u8; 0x40];
        f[..4].copy_from_slice(b"GRP2");
        put(&mut f, 0x10, 0x40);
        put(&mut f, 0x14, 8);
        for _ in 0..8 {
            f.extend_from_slice(&0x60u32.to_le_bytes());
        }
        f.extend_from_slice(&[b'a'; 1000]);
        f.push(0);
        assert!(GrpPack::parse(&f).is_err());
        // One name given to eight resources
        put(&mut f, 0x14, 1);
        let res_at = f.len() as u32;
        put(&mut f, 0x20, res_at);
        put(&mut f, 0x24, 8);
        let end = f.len() as u32 + 8 * 12;
        for _ in 0..8 {
            for v in [0, end, 0] {
                f.extend_from_slice(&v.to_le_bytes());
            }
        }
        assert!(GrpPack::parse(&f).is_err());
        put(&mut f, 0x24, 1);
        assert_eq!(GrpPack::parse(&f).unwrap().resources.len(), 1);
    }
}
