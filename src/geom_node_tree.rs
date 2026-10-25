// Provenance: this file is substantially similar to this project's own code in `src/dxp_and_grp/geom_node_tree.rs` of
// the unmerged wt_blk pull requests #28 and #29 (github.com/patodois/wt_blk, commit 498c996), after which it was
// written. That code follows the Dagor Engine sources (BSD-3-Clause, github.com/GaijinEntertainment/DagorEngine).
// No line of wt_blk's upstream author is in it.
// See the README, "Credit and provenance".

//! GeomNodeTrees: the skeleton of a model, a tree of named nodes that place its parts.
//!
//! See `docs/FORMATS.md` for the layout.

use std::borrow::Cow;

use crate::block::{read_block, unpack_block};
use crate::error::{Error, Result};
use crate::reader::{NameBudget, Reader};

/// Bytes of one node in the stream.
const NODE: usize = 160;

/// One node of a skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct GeomNode {
    /// The node's name.
    pub name: String,
    /// Index of the parent node, `None` for the root.
    pub parent: Option<usize>,
    /// The node's transform relative to its parent, a 4x4 matrix in column-major order.
    pub tm: [f32; 16],
    /// The node's transform in model space, column-major.
    pub wtm: [f32; 16],
}

/// A skeleton. A node's parent always comes before it in [`nodes`](Self::nodes).
#[derive(Debug, Clone, PartialEq)]
pub struct GeomNodeTree {
    /// The nodes, root first.
    pub nodes: Vec<GeomNode>,
}

impl GeomNodeTree {
    /// Reads a `GeomNodeTree` resource.
    pub fn parse(resource: &[u8]) -> Result<Self> {
        let mut r = Reader::new(resource);
        let dump_size = (r.u32()? & 0x000F_FFFF) as usize;
        let count_word = r.u32()?;
        let count = (count_word & 0x7FFF_FFFF) as usize;
        let packed = count_word & 0x8000_0000 != 0;
        // The dump is `dump_size` bytes (at most a mebibyte, the field has 20 bits), so a packed one is
        // never unpacked past that
        let dump = if packed { Cow::Owned(unpack_block(read_block(&mut r)?, dump_size)?) } else { Cow::Borrowed(r.rest()) };
        if dump.len() < dump_size || dump_size / NODE < count {
            return Err(Error::invalid("the node tree is cut short"));
        }

        let budget = NameBudget::new(&dump);
        let mut nodes = Vec::with_capacity(count);
        for i in 0..count {
            let mut n = Reader::at(&dump, i * NODE)?;
            let mut matrix = || -> Result<[f32; 16]> {
                let mut m = [0f32; 16];
                for v in &mut m {
                    *v = n.f32()?;
                }
                Ok(m)
            };
            let tm = matrix()?;
            let wtm = matrix()?;
            n.skip(16)?; // the children: an offset and a count
            let parent_at = n.wide()?;
            let name_at = n.wide()?;
            // The root's parent pointer is null, which reads as an offset of 0 like the pointer to the
            // root itself. Every other node's parent must be an earlier node: a loop would make the
            // walk up to the root never end
            let parent = if i == 0 {
                None
            } else if parent_at % NODE == 0 && parent_at / NODE < i {
                Some(parent_at / NODE)
            } else {
                return Err(Error::invalid(format!("node {i} has no valid parent")));
            };
            let name = budget.take(Reader::at(&dump, name_at)?.cstr()?)?;
            nodes.push(GeomNode { name, parent, tm, wtm });
        }
        Ok(GeomNodeTree { nodes })
    }

    /// The first node with this name.
    pub fn find(&self, name: &str) -> Option<&GeomNode> {
        self.nodes.iter().find(|n| n.name == name)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An unpacked tree where node `i` points at node `parents[i]`; node `i` is named by the letter `i`
    pub(crate) fn tree(parents: &[usize]) -> Vec<u8> {
        let names = parents.len() * NODE;
        let mut dump = vec![0u8; names];
        for (i, &parent) in parents.iter().enumerate() {
            let node = &mut dump[i * NODE..];
            node[144..152].copy_from_slice(&((parent * NODE) as u64).to_le_bytes());
            node[152..160].copy_from_slice(&((names + i * 2) as u64).to_le_bytes());
            // The identity as the local transform
            for k in 0..4 {
                node[(k * 4 + k) * 4..(k * 4 + k) * 4 + 4].copy_from_slice(&1f32.to_le_bytes());
            }
            dump.extend_from_slice(&[b'a' + i as u8, 0]);
        }
        let mut out = (dump.len() as u32).to_le_bytes().to_vec();
        out.extend_from_slice(&(parents.len() as u32).to_le_bytes());
        out.extend_from_slice(&dump);
        out
    }

    #[test]
    fn parents_come_first() {
        let t = GeomNodeTree::parse(&tree(&[0, 0, 1])).unwrap();
        let parents: Vec<_> = t.nodes.iter().map(|n| n.parent).collect();
        assert_eq!(parents, [None, Some(0), Some(1)]);
        assert_eq!(t.nodes[2].name, "c");
        assert_eq!(t.nodes[1].tm[5], 1.0);
        assert_eq!(t.find("b").unwrap().parent, Some(0));
        assert!(t.find("z").is_none());
        // A node that is its own parent, two nodes that are each other's, and a parent that does not exist
        assert!(GeomNodeTree::parse(&tree(&[0, 1])).is_err());
        assert!(GeomNodeTree::parse(&tree(&[0, 2, 1])).is_err());
        assert!(GeomNodeTree::parse(&tree(&[0, 5])).is_err());
    }

    #[test]
    fn a_parent_offset_between_nodes_is_refused() {
        let mut t = tree(&[0, 0]);
        let at = 8 + NODE + 144;
        t[at..at + 8].copy_from_slice(&7u64.to_le_bytes());
        assert!(GeomNodeTree::parse(&t).is_err());
    }

    #[test]
    fn a_name_cannot_be_copied_without_limit() {
        // Four nodes named by one 1000-byte string
        let mut t = tree(&[0, 0, 0, 0]);
        let name = t.len() - 8;
        t.extend_from_slice(&[b'a'; 1000]);
        t.push(0);
        for i in 0..4 {
            let at = 8 + i * NODE + 152;
            t[at..at + 8].copy_from_slice(&(name as u64).to_le_bytes());
        }
        assert!(GeomNodeTree::parse(&t).is_err());
    }

    #[test]
    fn counts_are_checked_before_use() {
        // 0x7FFFFFFF nodes of 160 bytes
        let mut t = vec![0u8; 8 + 0x100];
        t[..4].copy_from_slice(&0x100u32.to_le_bytes());
        t[4..8].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        assert!(GeomNodeTree::parse(&t).is_err());
        // A packed tree whose Oodle block claims 4 GiB
        let mut p = Vec::new();
        for v in [0u32, 0x8000_0001, 10, u32::MAX] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(&[0x8C, 0x06, 0, 0, 0, 0]);
        assert!(GeomNodeTree::parse(&p).is_err());
        assert!(GeomNodeTree::parse(&[]).is_err());
    }
}
