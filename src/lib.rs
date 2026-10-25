//! Readers for the resource formats of the Dagor Engine, as War Thunder ships them.
//!
//! - [`grp`]: `.grp` packs of game resources, found by name and class;
//! - [`dynmodel`] and [`geom_node_tree`]: the models and skeletons of vehicles inside them;
//! - [`dxp`] and [`ddsx`]: `.dxp.bin` texture packs and the textures in them;
//! - [`meshopt`]: the packed index buffers of models.
//!
//! The crate reads bytes you give it and never touches the game. It does not panic on malformed input,
//! and what it allocates is bounded by the input, up to a cap (a block never unpacks to more than [`MAX_UNPACKED`] bytes).
//! The layouts are written down in `docs/FORMATS.md` of the repository, with the evidence for each.
//!
//! ```no_run
//! use wt_formats::{dynmodel::DynModel, geom_node_tree::GeomNodeTree, grp::{class_id, GrpPack}};
//!
//! let file = std::fs::read("usa_m4a1_76w_sherman.grp")?;
//! let pack = GrpPack::parse(&file)?;
//! let model = pack.find("m4a1_76w_sherman", class_id::DYN_MODEL).expect("a model");
//! let model = DynModel::parse(pack.data(model))?;
//! let skeleton = pack.find("m4a1_76w_sherman_skeleton", class_id::GEOM_NODE_TREE).expect("a skeleton");
//! let skeleton = GeomNodeTree::parse(pack.data(skeleton))?;
//! println!("{} LODs, {} nodes", model.lods.len(), skeleton.nodes.len());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod block;
pub mod ddsx;
pub mod dxp;
pub mod dynmodel;
mod error;
pub mod geom_node_tree;
pub mod grp;
pub mod meshopt;
mod reader;
mod unpack;

pub use error::{Error, Result};
pub use unpack::MAX_UNPACKED;

#[cfg(test)]
mod hostile;
