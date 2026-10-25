// Provenance: this file reads a layout that wt_blk documented first, and it is substantially similar to
// wt_blk's `src/dxp_and_grp/dynmodel.rs` (github.com/Warthunder-Open-Source-Foundation/wt_blk, fork
// github.com/patodois/wt_blk at 498c996). That code was written by this project's author (wt_blk pull requests
// #28 and #29); no line of it is by wt_blk's upstream author. wt_blk declares MIT in its Cargo.toml and
// ships an Apache-2.0 LICENSE file; both are compatible with this crate's MIT OR Apache-2.0. See the README,
// "Provenance", for how the similarity was measured.

//! DynModels: the models of vehicles, with their level-of-detail meshes and skinned parts.
//!
//! A model holds vertex buffers, and LODs made of rigid parts (placed by the node of a
//! [skeleton](crate::geom_node_tree) with the same name) and skinned parts (tracks, cables). Materials
//! are not in the model: elements name them by number, and the numbers refer to the vehicle's
//! `dynModelDesc.bin`. See `docs/FORMATS.md` for the layout.

use crate::block::{read_block, unpack_block};
use crate::error::{Error, Result};
use crate::meshopt::decode_index_sequence;
use crate::reader::{check_table, NameBudget, Reader};
use crate::unpack;

/// Vertex channel types (`VSDT_*`) and usages (`SCUSAGE_*`).
pub mod channel {
    /// Two 32-bit floats.
    pub const FLOAT2: u32 = 1;
    /// Three 32-bit floats.
    pub const FLOAT3: u32 = 2;
    /// Four 32-bit floats.
    pub const FLOAT4: u32 = 3;
    /// Four bytes stored blue, green, red, alpha.
    pub const E3DCOLOR: u32 = 4;
    /// Four unsigned bytes.
    pub const UBYTE4: u32 = 5;
    /// Two signed 16-bit integers.
    pub const SHORT2: u32 = 6;
    /// Four signed 16-bit integers.
    pub const SHORT4: u32 = 7;
    /// Two signed 16-bit integers, normalized.
    pub const SHORT2N: u32 = 9;
    /// Four signed 16-bit integers, normalized.
    pub const SHORT4N: u32 = 10;
    /// Two half floats.
    pub const HALF2: u32 = 15;
    /// Four half floats.
    pub const HALF4: u32 = 16;

    /// Vertex position.
    pub const POSITION: u32 = 0;
    /// Vertex normal.
    pub const NORMAL: u32 = 1;
    /// Vertex color.
    pub const COLOR: u32 = 2;
    /// Texture coordinates; the channel's `index` says which set.
    pub const TEXCOORD: u32 = 3;
    /// The extra channels of skinned meshes (bone slots, then their weights) are declared as texture
    /// coordinate sets numbered from this.
    pub const FIRST_EXTRA_TEXCOORD: u16 = 4;
}
use channel::*;

const INDEX_16: u32 = 0x20;
const INDEX_32: u32 = 0x40;
const INDEX_PACKED: u32 = 0x200;
/// Texture coordinates stored as `SHORT2` are in units of 1/4096.
const TEXCOORD_UNIT: f32 = 1.0 / 4096.0;
/// The slot weights of a skinned vertex are the texture coordinate set after the slots.
const WEIGHTS_TEXCOORD: u16 = FIRST_EXTRA_TEXCOORD + 1;
/// The most shader constants a variant of the skinned meshes may be made for. The limit is the one
/// wt_blk chose; the variant in the game's files is made for 768.
const SKIN_CONSTANTS_LIMIT: u32 = 4096;
/// Set in the size of a skinned mesh that is stored in the current format.
const SKIN_CURRENT_FORMAT: usize = 0x4000_0000;

fn invalid(what: impl Into<String>) -> Error {
    Error::invalid(what)
}

/// One attribute of a vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    /// The data type, a [`channel`] constant such as `SHORT4N`.
    pub ty: u32,
    /// The meaning, a [`channel`] constant such as `NORMAL`.
    pub usage: u32,
    /// Which one of that usage (the texture coordinate set).
    pub index: u16,
    /// Bytes from the start of the vertex.
    pub offset: usize,
}

/// A vertex buffer and its index buffer.
#[derive(Debug, Clone)]
pub struct VData {
    /// Bytes per vertex.
    pub stride: usize,
    /// The vertices, `stride` bytes each.
    pub vertices: Vec<u8>,
    /// Indices into the vertices, to which an element's `base_vertex` is added.
    pub indices: Vec<u32>,
    /// The attributes of a vertex, in order.
    pub channels: Vec<Channel>,
}

impl VData {
    /// How many vertices the buffer holds.
    pub fn vertex_count(&self) -> usize {
        self.vertices.len() / self.stride.max(1)
    }

    /// How many groups of four bone influences each vertex carries; 0 for vertices of rigid parts.
    pub fn bone_groups(&self) -> usize {
        self.channels.iter().filter(|c| c.usage == TEXCOORD && c.index >= FIRST_EXTRA_TEXCOORD && c.index % 2 == 1).count()
    }

    /// Whether vertices carry a second set of texture coordinates (camouflage and occlusion maps use it).
    pub fn has_uv1(&self) -> bool {
        self.channels.iter().any(|c| c.usage == TEXCOORD && c.index == 1)
    }
}

/// A draw call: a range of triangles of one vertex buffer with one material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elem {
    /// The material, a number from the vehicle's `dynModelDesc.bin` (`matR` for rigid parts, `matS` for skins).
    pub material: usize,
    /// The vertex buffer: an index into [`DynModel::vdata`] for rigid parts, [`DynModel::skin_vdata`] for skins.
    pub vdata: usize,
    /// Where the element's indices start in the buffer's index list.
    pub first_index: usize,
    /// How many triangles; the element has three times as many indices.
    pub triangles: usize,
    /// Added to every index of the element.
    pub base_vertex: usize,
}

/// A part that moves rigidly with a skeleton node.
#[derive(Debug, Clone)]
pub struct Rigid {
    /// The skeleton node that places the part.
    pub node: String,
    /// What to draw.
    pub elems: Vec<Elem>,
}

/// A bone of a skinned mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    /// The skeleton node that moves the bone.
    pub node: String,
    /// The transform from model space into the bone's space in the bind pose, column-major. A vertex
    /// at `p` in the bind pose goes to `node_world_matrix * inverse_bind * p` once the node has moved.
    pub inverse_bind: [f32; 16],
}

/// The bones that vertices of one skinned material can name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinMaterial {
    /// The material, a `matS` number as in [`Elem::material`].
    pub material: usize,
    /// For each slot a vertex can name, the index into the bone table. Empty when slots are indices already.
    pub palette: Vec<u16>,
}

/// A skinned mesh (tracks, cables), stored in the bind pose in model space.
#[derive(Debug, Clone)]
pub struct Skin {
    /// The skeleton node the mesh hangs from.
    pub node: String,
    /// What to draw, from [`DynModel::skin_vdata`].
    pub elems: Vec<Elem>,
    /// The bones its vertices are blended between.
    pub bones: Vec<Bone>,
    /// The palette of each skinned material.
    pub materials: Vec<SkinMaterial>,
}

impl Skin {
    /// The bone a vertex of `material` (an [`Elem::material`]) means by palette `slot`.
    pub fn bone(&self, material: usize, slot: usize) -> Option<&Bone> {
        let palette = &self.materials.iter().find(|m| m.material == material)?.palette;
        let index = if palette.is_empty() { slot } else { usize::from(*palette.get(slot)?) };
        self.bones.get(index)
    }
}

/// One level of detail.
#[derive(Debug, Clone)]
pub struct Lod {
    /// The distance up to which this level is used.
    pub range: f32,
    /// The rigid parts.
    pub rigids: Vec<Rigid>,
    /// The skinned parts.
    pub skins: Vec<Skin>,
}

/// A model.
#[derive(Debug, Clone)]
pub struct DynModel {
    /// The vertex buffers of the rigid parts.
    pub vdata: Vec<VData>,
    /// The vertex buffers of the skinned parts.
    pub skin_vdata: Vec<VData>,
    /// Packed positions are `short4n * pos_scale + pos_offset`.
    pub pos_offset: [f32; 3],
    /// See [`pos_offset`](Self::pos_offset).
    pub pos_scale: [f32; 3],
    /// The levels of detail, most detailed first.
    pub lods: Vec<Lod>,
}

/// A decoded vertex.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vertex {
    /// Position in the space of the vertex's part (model space for skins).
    pub pos: [f32; 3],
    /// Unit normal.
    pub normal: [f32; 3],
    /// First texture coordinate set.
    pub uv: [f32; 2],
    /// Second texture coordinate set; zero when the buffer has none.
    pub uv1: [f32; 2],
}

/// The bones that move one skinned vertex.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SkinWeights {
    /// The palette slots of four bones (see [`Skin::bone`]). A slot whose weight is 0 is not used.
    pub joints: [u16; 4],
    /// Their weights, which add up to 1 up to rounding (they are stored as bytes). Only the first of
    /// the groups of four is read; [`VData::bone_groups`] says when a buffer has more.
    pub weights: [f32; 4],
}

// ---------------------------------------------------------------------------------------------
// Vertex data

/// The size in bytes of the packed index buffer an entry of the buffer table describes. Packed
/// buffers keep their stored size split over two words; plain ones are as long as their indices.
fn index_bytes(w1: u32, w2: u32, flags: u32) -> usize {
    if flags & INDEX_PACKED != 0 {
        ((w1 >> 8) | ((w2 >> 28) << 24)) as usize
    } else {
        (w2 & 0x0FFF_FFFF) as usize
    }
}

fn channel_bytes(ty: u32) -> Result<usize> {
    Ok(match ty {
        E3DCOLOR | UBYTE4 | SHORT2 | SHORT2N | HALF2 => 4,
        FLOAT2 | SHORT4 | SHORT4N | HALF4 => 8,
        FLOAT3 => 12,
        FLOAT4 => 16,
        other => return Err(Error::unsupported(format!("vertex channel type {other}"))),
    })
}

/// Reads the vertex data that follows the counts of a model or of its skins. `word` is the last of
/// those counts: it says how the data is stored.
fn read_vertex_data(r: &mut Reader, word: u32) -> Result<Vec<VData>> {
    if word & 0xE000_0000 == 0xC000_0000 {
        // Packed: header and buffers together in one block, the header being `word & 0x3FFFFFFF` bytes
        let all = unpack_block(read_block(r)?)?;
        return read_buffers(&all, (word & 0x3FFF_FFFF) as usize);
    }
    if word & 0xC000_0000 != 0 {
        return Err(Error::unsupported("vertex data packed with zlib or lzma"));
    }
    // Stored: the header, then the buffers; the header says how long they are
    let header_size = word as usize;
    let header = r.take(header_size)?;
    let (at, count) = {
        let mut h = Reader::at(header, 16)?;
        (h.size()?, h.size()?)
    };
    check_table(header, at, count, 32)?;
    let mut payload = 0usize;
    for i in 0..count {
        let mut e = Reader::at(header, at + i * 32)?;
        let (vertices, w1, w2, flags) = (e.size()?, e.u32()?, e.u32()?, e.u32()?);
        // Saturating: a total that does not fit in memory is past the end of the data anyway
        payload = payload.saturating_add(vertices.saturating_mul((w1 & 0xFF) as usize)).saturating_add(index_bytes(w1, w2, flags));
    }
    let mut all = header.to_vec();
    all.extend_from_slice(r.take(payload)?);
    read_buffers(&all, header_size)
}

/// The buffers described by the header at the start of `data`, whose vertex and index bytes start at
/// `payload_at`.
fn read_buffers(data: &[u8], payload_at: usize) -> Result<Vec<VData>> {
    let mut h = Reader::new(data);
    h.skip(16)?; // a material table, empty when the materials are in dynModelDesc.bin
    let (table_at, count) = (h.size()?, h.size()?);
    check_table(data, table_at, count, 32)?;
    let mut payload = Reader::at(data, payload_at)?;
    let mut buffers = Vec::with_capacity(count);
    for i in 0..count {
        let mut e = Reader::at(data, table_at + i * 32)?;
        let vertex_count = e.size()?;
        let (w1, w2, flags) = (e.u32()?, e.u32()?, e.u32()?);
        let (decl_at, decl_count) = (e.size()?, e.size()?);
        let stride = (w1 & 0xFF) as usize;
        // More than 64k vertices means 32-bit indices, unless the buffer says it keeps 16-bit ones
        let wide = flags & INDEX_32 != 0 || (vertex_count > 65536 && flags & INDEX_16 == 0);
        let index_count = (w2 & 0x0FFF_FFFF) as usize / if wide { 4 } else { 2 };

        check_table(data, decl_at, decl_count, 12)?;
        let mut channels = Vec::with_capacity(decl_count);
        let mut d = Reader::at(data, decl_at)?;
        let mut offset = 0;
        for _ in 0..decl_count {
            let ty = d.u32()? >> 16;
            let usage = d.u32()?;
            let index = d.u16()?;
            d.skip(2)?;
            channels.push(Channel { ty, usage, index, offset });
            offset += channel_bytes(ty)?;
        }
        if offset != stride {
            return Err(invalid(format!("the vertex declaration covers {offset} of {stride} bytes")));
        }

        let vertices = payload.take(vertex_count.saturating_mul(stride))?.to_vec();
        let raw = payload.take(index_bytes(w1, w2, flags))?;
        let indices = if flags & INDEX_PACKED != 0 {
            decode_index_sequence(raw, index_count, wide)?
        } else if wide {
            raw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
        } else {
            raw.chunks_exact(2).map(|c| u32::from(u16::from_le_bytes([c[0], c[1]]))).collect()
        };
        buffers.push(VData { stride, vertices, indices, channels });
    }
    if payload.remaining() != 0 {
        return Err(invalid("the vertex data has bytes left over"));
    }
    Ok(buffers)
}

// ---------------------------------------------------------------------------------------------
// Meshes

/// The elements of the `ShaderMesh` at `at` in `dump` (pointers in it are relative to the start of
/// `dump`). A mesh has an element table and a second one for its see-through parts; a flag in the
/// word at +0x20 (bit 31) says the second table is not used.
fn read_mesh(dump: &[u8], at: usize, buffers: &[VData]) -> Result<Vec<Elem>> {
    let mut r = Reader::at(dump, at)?;
    let main = (r.size()?, r.size()?);
    r.skip(8)?;
    let second = (r.size()?, r.size()?);
    r.skip(8)?;
    let tables: &[(usize, usize)] = if r.u32()? & 0x8000_0000 != 0 { &[main] } else { &[main, second] };

    let mut elems = Vec::new();
    for &(table_at, count) in tables {
        check_table(dump, table_at, count, 48)?;
        elems.reserve(count);
        for i in 0..count {
            let mut e = Reader::at(dump, table_at + i * 48)?;
            e.skip(8)?; // the shader element, filled in when the game runs
            let material = e.wide()?;
            let vdata = e.wide()?;
            e.skip(12)?; // the vertex range, a hint for old Direct3D
            let elem = Elem { material, vdata, first_index: e.size()?, triangles: e.size()?, base_vertex: e.size()? };
            check_elem(&elem, buffers)?;
            elems.push(elem);
        }
    }
    Ok(elems)
}

/// Makes every index an element names land on a vertex, so that users need not check.
fn check_elem(elem: &Elem, buffers: &[VData]) -> Result<()> {
    let buffer =
        buffers.get(elem.vdata).ok_or_else(|| invalid(format!("an element uses vertex buffer {}, which does not exist", elem.vdata)))?;
    let indices = elem
        .triangles
        .checked_mul(3)
        .and_then(|n| n.checked_add(elem.first_index))
        .and_then(|end| buffer.indices.get(elem.first_index..end))
        .ok_or_else(|| invalid("an element's indices are outside its buffer"))?;
    let vertices = buffer.vertex_count();
    if indices.iter().any(|&i| (i as usize).checked_add(elem.base_vertex).is_none_or(|v| v >= vertices)) {
        return Err(invalid("an element names a vertex that does not exist"));
    }
    Ok(())
}

impl DynModel {
    /// Reads a `DynModel` resource.
    pub fn parse(resource: &[u8]) -> Result<Self> {
        let mut r = Reader::new(resource);
        let lods_dump_size = r.size()?;
        let texture_count = r.u32()?;
        let _material_count = r.u32()?;
        let _buffer_count = r.u32()?;
        let vertex_word = r.u32()?;
        if texture_count != u32::MAX {
            return Err(Error::unsupported("a model with its own materials"));
        }
        let vdata = read_vertex_data(&mut r, vertex_word)?;

        let lods_dump = r.take(lods_dump_size)?;
        let vec3 = |at: usize| -> Result<[f32; 3]> {
            let mut c = Reader::at(lods_dump, at)?;
            Ok([c.f32()?, c.f32()?, c.f32()?])
        };
        let (pos_offset, pos_scale) = (vec3(0x28)?, vec3(0x38)?);

        let names = read_node_names(&mut r)?;
        // Every part and skin takes a copy of the name of its node
        let copies = NameBudget::new(resource);
        let node_name = |id: usize| {
            let name = names.get(id).ok_or_else(|| invalid(format!("node {id} has no name")))?;
            copies.take(name.clone())
        };

        let mut table = Reader::new(lods_dump);
        let (table_at, lod_count) = (table.size()?, table.size()?);
        check_table(lods_dump, table_at, lod_count, 16)?;
        let mut lods = Vec::with_capacity(lod_count);
        let mut skin_counts = Vec::with_capacity(lod_count);
        for l in 0..lod_count {
            let mut entry = Reader::at(lods_dump, table_at + l * 16)?;
            let scene_size = entry.wide()?;
            let range = entry.f32()?;
            let scene = r.take(scene_size)?;
            let mut s = Reader::new(scene);
            let (rigids_at, rigid_count) = (s.size()?, s.size()?);
            s.skip(12)?;
            let skin_count = s.size()?;
            check_table(scene, rigids_at, rigid_count, 32)?;
            let mut rigids = Vec::with_capacity(rigid_count);
            for i in 0..rigid_count {
                let mut e = Reader::at(scene, rigids_at + i * 32)?;
                let mesh_size = e.wide()?;
                e.skip(16)?; // the bounding sphere
                let node = node_name(e.size()?)?;
                // The meshes follow the scene, one per rigid part
                rigids.push(Rigid { node, elems: read_mesh(r.take(mesh_size)?, 0, &vdata)? });
            }
            // The skins are in a section of their own further on
            lods.push(Lod { range, rigids, skins: Vec::new() });
            skin_counts.push(skin_count);
        }

        let skin_vdata =
            if skin_counts.iter().any(|&n| n > 0) { read_skins(&mut r, &mut lods, &skin_counts, &node_name)? } else { Vec::new() };
        Ok(DynModel { vdata, skin_vdata, pos_offset, pos_scale, lods })
    }

    /// Decodes vertex `index` of `buffer`, one of this model's vertex buffers (rigid or skinned).
    pub fn vertex(&self, buffer: &VData, index: usize) -> Result<Vertex> {
        let v = vertex_bytes(buffer, index)?;
        let i16_at = |o: usize| f32::from(i16::from_le_bytes([v[o], v[o + 1]]));
        let f32_at = |o: usize| f32::from_le_bytes([v[o], v[o + 1], v[o + 2], v[o + 3]]);
        let mut out = Vertex::default();
        for c in &buffer.channels {
            let o = c.offset;
            match (c.usage, c.index, c.ty) {
                (POSITION, _, SHORT4N) => {
                    for k in 0..3 {
                        out.pos[k] = i16_at(o + 2 * k) / 32767.0 * self.pos_scale[k] + self.pos_offset[k];
                    }
                }
                (POSITION, _, FLOAT3 | FLOAT4) => out.pos = [f32_at(o), f32_at(o + 4), f32_at(o + 8)],
                // E3DCOLOR is blue, green, red, alpha in memory; the normal is x = red, y = green, z = blue
                (NORMAL, _, E3DCOLOR) => out.normal = [v[o + 2], v[o + 1], v[o]].map(|b| f32::from(b) / 127.5 - 1.0),
                (TEXCOORD, 0, SHORT2) => out.uv = [i16_at(o), i16_at(o + 2)].map(|x| x * TEXCOORD_UNIT),
                (TEXCOORD, 1, SHORT2) => out.uv1 = [i16_at(o), i16_at(o + 2)].map(|x| x * TEXCOORD_UNIT),
                (TEXCOORD, 0, FLOAT2) => out.uv = [f32_at(o), f32_at(o + 4)],
                (TEXCOORD, 1, FLOAT2) => out.uv1 = [f32_at(o), f32_at(o + 4)],
                _ => {}
            }
        }
        Ok(out)
    }

    /// The bone slots and weights of vertex `index` of `buffer`, one of [`skin_vdata`](Self::skin_vdata).
    /// Vertices of buffers without bones have weights of 0.
    pub fn skin_weights(&self, buffer: &VData, index: usize) -> Result<SkinWeights> {
        let v = vertex_bytes(buffer, index)?;
        let u16_at = |o: usize| u16::from_le_bytes([v[o], v[o + 1]]);
        let f32_at = |o: usize| f32::from_le_bytes([v[o], v[o + 1], v[o + 2], v[o + 3]]);
        let mut out = SkinWeights::default();
        for c in buffer.channels.iter().filter(|c| c.usage == TEXCOORD) {
            let o = c.offset;
            match (c.index, c.ty) {
                // x, y, z, w of an E3DCOLOR are bytes 2, 1, 0, 3
                (FIRST_EXTRA_TEXCOORD, E3DCOLOR) => out.joints = [v[o + 2], v[o + 1], v[o], v[o + 3]].map(u16::from),
                (FIRST_EXTRA_TEXCOORD, SHORT4) => out.joints = [0, 1, 2, 3].map(|k| u16_at(o + 2 * k)),
                (WEIGHTS_TEXCOORD, E3DCOLOR) => out.weights = [v[o + 2], v[o + 1], v[o], v[o + 3]].map(|b| f32::from(b) / 255.0),
                (WEIGHTS_TEXCOORD, FLOAT4) => out.weights = [f32_at(o), f32_at(o + 4), f32_at(o + 8), f32_at(o + 12)],
                _ => {}
            }
        }
        Ok(out)
    }
}

fn vertex_bytes(buffer: &VData, index: usize) -> Result<&[u8]> {
    index
        .checked_mul(buffer.stride)
        .and_then(|start| buffer.vertices.get(start..start.checked_add(buffer.stride)?))
        .ok_or_else(|| invalid(format!("vertex {index} does not exist")))
}

// ---------------------------------------------------------------------------------------------
// Node names and skins

/// The names of the skeleton nodes a model refers to, indexed by node id. They are stored as a
/// `RoNameMapEx`: the names sorted, with the node id of each.
fn read_node_names(r: &mut Reader) -> Result<Vec<String>> {
    let size = r.size()?;
    let dump = r.take(size)?;
    let mut map = Reader::new(dump);
    let (names_at, count) = (map.size()?, map.size()?);
    map.skip(8)?;
    let (ids_at, id_count) = (map.size()?, map.size()?);
    if id_count != count {
        return Err(invalid("the node name map and its id table differ in length"));
    }
    check_table(dump, names_at, count, 8)?;
    check_table(dump, ids_at, id_count, 2)?;
    let budget = NameBudget::new(dump);
    let mut names = vec![String::new(); count];
    for i in 0..count {
        let name_at = Reader::at(dump, names_at + i * 8)?.wide()?;
        let id = usize::from(Reader::at(dump, ids_at + i * 2)?.u16()?);
        *names.get_mut(id).ok_or_else(|| invalid("a node id is past the end of the name map"))? =
            budget.take(Reader::at(dump, name_at)?.cstr()?)?;
    }
    Ok(names)
}

/// The skinned meshes, which come after the last LOD: a table of variants made for different shader
/// constant limits, vertex data like the rigid parts', then a zlib stream with, for each LOD, the node
/// ids of its skins, their sizes, and the skin dumps.
fn read_skins(r: &mut Reader, lods: &mut [Lod], counts: &[usize], node_name: &dyn Fn(usize) -> Result<String>) -> Result<Vec<VData>> {
    let section = r.pos();
    let (variants, total) = (r.u32()?, r.size()?);
    // Every skinned model of War Thunder 2.59 (3,632 of them) has one variant, made for 768 shader
    // constants, with nothing between its entry and the vertex data. How a table of several lays
    // out its variants is not in any file seen, so it is not guessed at
    if variants != 1 {
        return Err(Error::unsupported(format!("{variants} variants of the skinned meshes")));
    }
    let (constants, gap) = (r.u32()?, r.size()?);
    if constants > SKIN_CONSTANTS_LIMIT || gap != 0 {
        return Err(Error::unsupported(format!(
            "a skinned mesh variant for {constants} shader constants with {gap} bytes after its entry"
        )));
    }

    let (texture_count, _material_count, _buffer_count, vertex_word) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
    if texture_count != u32::MAX {
        return Err(Error::unsupported("skins with their own materials"));
    }
    let buffers = read_vertex_data(r, vertex_word)?;

    let end = section.saturating_add(8).saturating_add(total);
    let packed = r.take(end.checked_sub(r.pos()).ok_or_else(|| invalid("the skin section is longer than it says"))?)?;
    let stream = unpack::zlib(packed)?;
    let mut z = Reader::new(&stream);
    for (lod, &count) in lods.iter_mut().zip(counts) {
        let nodes = (0..count).map(|_| z.u32()).collect::<Result<Vec<_>>>()?;
        let sizes = (0..count).map(|_| z.wide()).collect::<Result<Vec<_>>>()?;
        for (node, size) in nodes.into_iter().zip(sizes) {
            if size & SKIN_CURRENT_FORMAT == 0 {
                return Err(Error::unsupported("a skinned mesh in the old format"));
            }
            let dump = z.take(size & !SKIN_CURRENT_FORMAT)?;
            let (bones, materials) = read_bones(dump, node_name)?;
            lod.skins.push(Skin { node: node_name(node as usize)?, elems: read_mesh(dump, 0x38, &buffers)?, bones, materials });
        }
    }
    Ok(buffers)
}

/// The bone table and the palettes at the start of a skinned mesh's dump.
fn read_bones(dump: &[u8], node_name: &dyn Fn(usize) -> Result<String>) -> Result<(Vec<Bone>, Vec<SkinMaterial>)> {
    let mut r = Reader::new(dump);
    let (tm_at, bone_count) = (r.size()?, r.size()?);
    r.skip(8)?;
    let (palettes_at, material_count) = (r.size()?, r.size()?);
    r.skip(8)?;
    let (nodes_at, materials_at) = (r.wide()?, r.wide()?);
    check_table(dump, tm_at, bone_count, 48)?;
    check_table(dump, nodes_at, bone_count, 2)?;
    check_table(dump, palettes_at, material_count, 16)?;
    check_table(dump, materials_at, material_count, 8)?;

    let mut bones = Vec::with_capacity(bone_count);
    for i in 0..bone_count {
        let node = usize::from(Reader::at(dump, nodes_at + i * 2)?.u16()?);
        let mut tm = Reader::at(dump, tm_at + i * 48)?;
        // Twelve floats: the x, y and z axes and the translation, three numbers each; the bottom row is 0 0 0 1
        let mut inverse_bind = [0f32; 16];
        inverse_bind[15] = 1.0;
        for column in 0..4 {
            for row in 0..3 {
                inverse_bind[column * 4 + row] = tm.f32()?;
            }
        }
        bones.push(Bone { node: node_name(node)?, inverse_bind });
    }

    let mut materials = Vec::with_capacity(material_count);
    for i in 0..material_count {
        let material = Reader::at(dump, materials_at + i * 8)?.size()?;
        let mut p = Reader::at(dump, palettes_at + i * 16)?;
        let (slots_at, slot_count) = (p.size()?, p.size()?);
        check_table(dump, slots_at, slot_count, 2)?;
        let mut slots = Reader::at(dump, slots_at)?;
        let mut palette = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            let bone = slots.u16()?;
            if usize::from(bone) >= bone_count {
                return Err(invalid("a palette names a bone that is not in the bone table"));
            }
            palette.push(bone);
        }
        materials.push(SkinMaterial { material, palette });
    }
    Ok((bones, materials))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn put32(out: &mut Vec<u8>, values: &[u32]) {
        for v in values {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }

    fn set32(buf: &mut [u8], at: usize, v: u32) {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Three vertices of 16 bytes: a SHORT4N position (8 bytes), an E3DCOLOR normal and a SHORT2 uv
    fn triangle() -> (Vec<u8>, Vec<u8>) {
        let mut vertices = Vec::new();
        for (pos, uv) in [([0i16, 0, 0], [0i16, 0]), ([32767, 0, 0], [4096, 0]), ([0, 32767, 0], [0, -4096])] {
            for p in pos {
                vertices.extend_from_slice(&p.to_le_bytes());
            }
            vertices.extend_from_slice(&0i16.to_le_bytes());
            vertices.extend_from_slice(&[0xFF, 0x80, 0x00, 0xFF]); // blue, green, red, alpha
            for c in uv {
                vertices.extend_from_slice(&c.to_le_bytes());
            }
        }
        let mut indices = Vec::new();
        for i in [0u16, 1, 2] {
            indices.extend_from_slice(&i.to_le_bytes());
        }
        (vertices, indices)
    }

    /// The header and the payload of a vertex data section holding the triangle's buffer, with the
    /// extra channels (type, usage, index) after its three, their bytes zero
    fn vertex_data(extra_channels: &[(u32, u32, u16)]) -> (Vec<u8>, Vec<u8>) {
        let (base, indices) = triangle();
        let decls: Vec<(u32, u32, u16)> = [(SHORT4N, POSITION, 0), (E3DCOLOR, NORMAL, 0), (SHORT2, TEXCOORD, 0)]
            .into_iter()
            .chain(extra_channels.iter().copied())
            .collect();
        let extra: usize = extra_channels.iter().map(|c| channel_bytes(c.0).unwrap()).sum();
        let stride = 16 + extra;
        let mut vertices = Vec::new();
        for v in base.chunks(16) {
            vertices.extend_from_slice(v);
            vertices.extend(std::iter::repeat_n(0u8, extra));
        }
        let mut header = vec![0u8; 16];
        put32(&mut header, &[0x20, 1, 0, 0]);
        // One buffer: 3 vertices, 6 bytes of indices, no flags, its declaration right after the table
        put32(&mut header, &[3, stride as u32, indices.len() as u32, 0, 0x40, decls.len() as u32, 0, 0]);
        for (ty, usage, index) in &decls {
            put32(&mut header, &[ty << 16, *usage]);
            header.extend_from_slice(&index.to_le_bytes());
            header.extend_from_slice(&[0, 0]);
        }
        let mut payload = vertices;
        payload.extend_from_slice(&indices);
        (header, payload)
    }

    /// A ShaderMesh dump at the start of `out` with `elems` as (material, vdata, first, triangles, base)
    fn mesh(elems: &[(u32, u32, u32, u32, u32)], base: usize) -> Vec<u8> {
        let mut m = Vec::new();
        put32(&mut m, &[(base + 0x28) as u32, elems.len() as u32, 0, 0, 0, 0, 0, 0, 0x8000_0000, 0]);
        for &(material, vdata, first, triangles, base_vertex) in elems {
            m.extend_from_slice(&[0; 8]);
            m.extend_from_slice(&u64::from(material).to_le_bytes());
            m.extend_from_slice(&u64::from(vdata).to_le_bytes());
            m.extend_from_slice(&[0; 12]);
            put32(&mut m, &[first, triangles, base_vertex]);
        }
        m
    }

    /// A model with one LOD (range 50) and one rigid part on node 0 ("hull") drawing the triangle
    pub(crate) fn model() -> Vec<u8> {
        let (header, payload) = vertex_data(&[]);
        let mut lods = vec![0u8; 0x58];
        set32(&mut lods, 0, 0x48);
        set32(&mut lods, 4, 1);
        for (at, v) in [(0x28, 10.0f32), (0x2C, 20.0), (0x30, 30.0), (0x38, 2.0), (0x3C, 3.0), (0x40, 4.0)] {
            lods[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        let mesh = mesh(&[(7, 0, 0, 1, 0)], 0);
        let mut scene = Vec::new();
        put32(&mut scene, &[24, 1, 0, 0, 0, 0]);
        scene.extend_from_slice(&(mesh.len() as u64).to_le_bytes());
        scene.extend_from_slice(&[0; 16]);
        put32(&mut scene, &[0, 0]);
        lods[0x48..0x50].copy_from_slice(&(scene.len() as u64).to_le_bytes());
        lods[0x50..0x54].copy_from_slice(&50f32.to_le_bytes());

        let mut res = Vec::new();
        put32(&mut res, &[lods.len() as u32, u32::MAX, u32::MAX, 1, header.len() as u32]);
        res.extend_from_slice(&header);
        res.extend_from_slice(&payload);
        res.extend_from_slice(&lods);
        let names = node_names(&["hull"]);
        put32(&mut res, &[names.len() as u32]);
        res.extend_from_slice(&names);
        res.extend_from_slice(&scene);
        res.extend_from_slice(&mesh);
        res
    }

    /// A RoNameMapEx with these names: the id of a name is its position
    fn node_names(names: &[&str]) -> Vec<u8> {
        let n = names.len();
        let mut d = vec![0u8; 0x20];
        set32(&mut d, 0, 0x20);
        set32(&mut d, 4, n as u32);
        set32(&mut d, 0x10, (0x20 + 8 * n) as u32);
        set32(&mut d, 0x14, n as u32);
        let strings_at = 0x20 + 8 * n + 2 * n;
        let mut strings = Vec::new();
        let mut pointers = Vec::new();
        for name in names {
            pointers.extend_from_slice(&((strings_at + strings.len()) as u64).to_le_bytes());
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
        }
        d.extend_from_slice(&pointers);
        for id in 0..n as u16 {
            d.extend_from_slice(&id.to_le_bytes());
        }
        d.extend_from_slice(&strings);
        d
    }

    #[test]
    fn a_small_model() {
        let m = DynModel::parse(&model()).unwrap();
        assert_eq!(m.pos_offset, [10.0, 20.0, 30.0]);
        assert_eq!(m.pos_scale, [2.0, 3.0, 4.0]);
        assert_eq!(m.vdata.len(), 1);
        assert!(m.skin_vdata.is_empty());
        let vd = &m.vdata[0];
        assert_eq!((vd.stride, vd.vertex_count(), vd.indices.clone()), (16, 3, vec![0, 1, 2]));
        assert_eq!(vd.channels[1], Channel { ty: E3DCOLOR, usage: NORMAL, index: 0, offset: 8 });
        assert_eq!(vd.channels[2].offset, 12);
        assert!(!vd.has_uv1());
        assert_eq!(vd.bone_groups(), 0);
        assert_eq!(m.lods.len(), 1);
        let lod = &m.lods[0];
        assert_eq!(lod.range, 50.0);
        assert!(lod.skins.is_empty());
        assert_eq!(lod.rigids.len(), 1);
        assert_eq!(lod.rigids[0].node, "hull");
        assert_eq!(lod.rigids[0].elems, [Elem { material: 7, vdata: 0, first_index: 0, triangles: 1, base_vertex: 0 }]);

        let v = m.vertex(vd, 1).unwrap();
        assert_eq!(v.pos, [32767.0 / 32767.0 * 2.0 + 10.0, 20.0, 30.0]);
        assert_eq!(v.uv, [1.0, 0.0]);
        assert_eq!(v.uv1, [0.0, 0.0]);
        // The normal bytes are blue 0xFF, green 0x80, red 0x00: x = red, y = green, z = blue
        assert_eq!(v.normal, [-1.0, 128.0 / 127.5 - 1.0, 1.0]);
        assert_eq!(m.vertex(vd, 2).unwrap().uv, [0.0, -1.0]);
        assert!(m.vertex(vd, 3).is_err());
        assert!(m.vertex(vd, usize::MAX).is_err());
        // No bone channels: nothing moves the vertex
        assert_eq!(m.skin_weights(vd, 0).unwrap(), SkinWeights::default());
    }

    #[test]
    fn wide_and_float_positions() {
        let float_pos = VData {
            stride: 12,
            vertices: [1.5f32, 2.5, 3.5].iter().flat_map(|f| f.to_le_bytes()).collect(),
            indices: vec![],
            channels: vec![Channel { ty: FLOAT3, usage: POSITION, index: 0, offset: 0 }],
        };
        let m = DynModel { vdata: vec![], skin_vdata: vec![], pos_offset: [100.0; 3], pos_scale: [100.0; 3], lods: vec![] };
        assert_eq!(m.vertex(&float_pos, 0).unwrap().pos, [1.5, 2.5, 3.5]);
    }

    #[test]
    fn bone_weights_of_a_vertex() {
        // Slots 1 2 3 4 as E3DCOLOR (stored z, y, x, w) and weights 255 0 0 0; then slots as SHORT4 and FLOAT4 weights
        let color = |x: u8, y: u8, z: u8, w: u8| [z, y, x, w];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&color(1, 2, 3, 4));
        bytes.extend_from_slice(&color(255, 0, 0, 0));
        let a = VData {
            stride: 8,
            vertices: bytes,
            indices: vec![],
            channels: vec![
                Channel { ty: E3DCOLOR, usage: TEXCOORD, index: 4, offset: 0 },
                Channel { ty: E3DCOLOR, usage: TEXCOORD, index: 5, offset: 4 },
            ],
        };
        let m = DynModel { vdata: vec![], skin_vdata: vec![], pos_offset: [0.0; 3], pos_scale: [1.0; 3], lods: vec![] };
        assert_eq!(m.skin_weights(&a, 0).unwrap(), SkinWeights { joints: [1, 2, 3, 4], weights: [1.0, 0.0, 0.0, 0.0] });
        assert_eq!(a.bone_groups(), 1);
        let mut wide = Vec::new();
        for j in [300u16, 301, 302, 303] {
            wide.extend_from_slice(&j.to_le_bytes());
        }
        for w in [0.25f32, 0.25, 0.5, 0.0] {
            wide.extend_from_slice(&w.to_le_bytes());
        }
        let b = VData {
            stride: 24,
            vertices: wide,
            indices: vec![],
            channels: vec![
                Channel { ty: SHORT4, usage: TEXCOORD, index: 4, offset: 0 },
                Channel { ty: FLOAT4, usage: TEXCOORD, index: 5, offset: 8 },
            ],
        };
        assert_eq!(m.skin_weights(&b, 0).unwrap(), SkinWeights { joints: [300, 301, 302, 303], weights: [0.25, 0.25, 0.5, 0.0] });
    }

    #[test]
    fn a_skin_picks_bones_through_its_palette() {
        let bone = |n: &str| Bone { node: n.into(), inverse_bind: [0.0; 16] };
        let skin = Skin {
            node: "track".into(),
            elems: vec![],
            bones: vec![bone("a"), bone("b"), bone("c")],
            materials: vec![SkinMaterial { material: 0, palette: vec![] }, SkinMaterial { material: 1, palette: vec![2, 0] }],
        };
        assert_eq!(skin.bone(0, 1).unwrap().node, "b");
        assert_eq!(skin.bone(1, 0).unwrap().node, "c");
        assert_eq!(skin.bone(1, 1).unwrap().node, "a");
        assert!(skin.bone(1, 2).is_none());
        assert!(skin.bone(0, 3).is_none());
        assert!(skin.bone(9, 0).is_none());
    }

    #[test]
    fn elements_are_checked_against_their_buffers() {
        let with = |elems: &[(u32, u32, u32, u32, u32)]| {
            let mut res = model();
            // The mesh is the last thing in the resource
            let old = mesh(&[(7, 0, 0, 1, 0)], 0);
            res.truncate(res.len() - old.len());
            res.extend_from_slice(&mesh(elems, 0));
            // The scene says how long the mesh is
            let scene_at = res.len() - mesh(elems, 0).len() - 24 - 32;
            res[scene_at + 24..scene_at + 32].copy_from_slice(&(mesh(elems, 0).len() as u64).to_le_bytes());
            DynModel::parse(&res)
        };
        assert!(with(&[(0, 0, 0, 1, 0)]).is_ok());
        assert!(with(&[]).is_ok());
        assert!(with(&[(0, 1, 0, 1, 0)]).is_err(), "a buffer that does not exist");
        assert!(with(&[(0, 0, 1, 1, 0)]).is_err(), "indices past the end");
        assert!(with(&[(0, 0, 0, 2, 0)]).is_err(), "more triangles than indices");
        assert!(with(&[(0, 0, 0, 1, 1)]).is_err(), "a base vertex that pushes an index past the vertices");
        assert!(with(&[(0, 0, u32::MAX, 1, 0)]).is_err());
        assert!(with(&[(0, 0, 0, u32::MAX, 0)]).is_err());
        assert!(with(&[(0, 0, 0, 1, u32::MAX)]).is_err());
        assert!(with(&[(0, 0, 0, 0x5555_5556, 0)]).is_err());
    }

    #[test]
    fn broken_models_are_errors() {
        let good = model();
        // Cut anywhere short of the end
        for cut in 0..good.len() - 1 {
            // Some cuts leave a model that is complete without its last padding, but none may panic
            let _ = DynModel::parse(&good[..cut]);
        }
        assert!(DynModel::parse(&good[..good.len() / 2]).is_err());
        assert!(DynModel::parse(&[]).is_err());
        // Models with their own materials are not read
        let mut own = good.clone();
        set32(&mut own, 4, 3);
        assert!(DynModel::parse(&own).is_err());
        // Every 32-bit word of the resource set to a huge value, one at a time, ends in an error or a
        // model, and never in a panic or a giant allocation
        for at in (0..good.len() - 3).step_by(4) {
            for v in [u32::MAX, 0x8000_0000, 0x7FFF_FFFF, 0x4000_0000] {
                let mut bad = good.clone();
                set32(&mut bad, at, v);
                let _ = DynModel::parse(&bad);
            }
        }
    }

    #[test]
    fn vertex_declarations_must_add_up() {
        let (mut header, payload) = vertex_data(&[]);
        // A stride that is not what the channels add up to
        set32(&mut header, 0x20 + 4, 20);
        let mut res = Vec::new();
        put32(&mut res, &[0, u32::MAX, u32::MAX, 1, header.len() as u32]);
        res.extend_from_slice(&header);
        res.extend_from_slice(&payload);
        assert!(DynModel::parse(&res).is_err());
        // A channel type nobody knows
        let (mut header, _) = vertex_data(&[]);
        let decl = 0x40;
        set32(&mut header, decl, 99 << 16);
        let mut r = Reader::new(&header);
        assert!(read_buffers(&header, header.len()).is_err());
        let _ = r.rest();
    }

    #[test]
    fn counts_are_checked_before_they_size_anything() {
        let mut vd = vec![0u8; 24];
        // 4 billion vertex buffers
        set32(&mut vd, 16, 24);
        set32(&mut vd, 20, u32::MAX);
        assert!(read_buffers(&vd, 24).is_err());
        // One buffer with 4 billion channels
        let mut vd = vec![0u8; 24 + 32];
        set32(&mut vd, 16, 24);
        set32(&mut vd, 20, 1);
        set32(&mut vd, 24 + 16, 56);
        set32(&mut vd, 24 + 20, u32::MAX);
        assert!(read_buffers(&vd, 56).is_err());
        // 4 billion vertices of 255 bytes
        let mut vd = vec![0u8; 24 + 32];
        set32(&mut vd, 16, 24);
        set32(&mut vd, 20, 1);
        set32(&mut vd, 24, u32::MAX);
        set32(&mut vd, 28, 255);
        assert!(read_buffers(&vd, 56).is_err());
    }

    #[test]
    fn a_stored_header_larger_than_the_resource_is_refused() {
        let mut res = Vec::new();
        put32(&mut res, &[0, u32::MAX, u32::MAX, 1, 0x3FFF_FFFF]);
        assert!(DynModel::parse(&res).is_err());
        let mut res = Vec::new();
        put32(&mut res, &[0, u32::MAX, u32::MAX, 1, 0xE000_0000]);
        assert!(DynModel::parse(&res).is_err());
    }

    #[test]
    fn packed_vertex_data_is_unpacked() {
        // The same triangle with header and buffers in one zstd block
        let (header, payload) = vertex_data(&[]);
        let mut all = header.clone();
        all.extend_from_slice(&payload);
        let packed = zstd::bulk::compress(&all, 0).unwrap();
        let mut res = Vec::new();
        put32(&mut res, &[0, u32::MAX, u32::MAX, 1, 0xC000_0000 | header.len() as u32, packed.len() as u32 | 1 << 30]);
        res.extend_from_slice(&packed);
        let mut r = Reader::at(&res, 20).unwrap();
        let buffers = read_vertex_data(&mut r, 0xC000_0000 | header.len() as u32).unwrap();
        assert_eq!(buffers.len(), 1);
        assert_eq!(buffers[0].indices, [0, 1, 2]);
        assert_eq!(r.remaining(), 0);
        // A block that unpacks to less than its header says it holds
        let short = zstd::bulk::compress(&all[..all.len() - 1], 0).unwrap();
        let mut res = Vec::new();
        put32(&mut res, &[short.len() as u32]);
        res.extend_from_slice(&short);
        let mut r = Reader::new(&res);
        assert!(read_vertex_data(&mut r, 0xC000_0000 | header.len() as u32).is_err());
    }

    #[test]
    fn packed_index_buffers_use_two_size_words() {
        // 5 bytes of header and tail around two one-byte indices, 7 bytes in all, and 4 bytes unpacked
        assert_eq!(index_bytes(7 << 8, 4, INDEX_PACKED), 7);
        assert_eq!(index_bytes(0xFFFF_FF00, 0xF000_0000, INDEX_PACKED), 0x0FFF_FFFF);
        assert_eq!(index_bytes(0xFFFF_FF00, 0xF123_4567, 0), 0x0123_4567);
    }

    #[test]
    fn node_names_map_ids_to_names() {
        let names = node_names(&["a", "bb", "ccc"]);
        let mut res = Vec::new();
        put32(&mut res, &[names.len() as u32]);
        res.extend_from_slice(&names);
        assert_eq!(read_node_names(&mut Reader::new(&res)).unwrap(), ["a", "bb", "ccc"]);
        // An id outside the map, and tables that disagree
        let mut bad = res.clone();
        let ids_at = 4 + 0x20 + 8 * 3;
        bad[ids_at..ids_at + 2].copy_from_slice(&9u16.to_le_bytes());
        assert!(read_node_names(&mut Reader::new(&bad)).is_err());
        let mut bad = res.clone();
        set32(&mut bad, 4 + 0x14, 2);
        assert!(read_node_names(&mut Reader::new(&bad)).is_err());
    }

    #[test]
    fn a_node_name_cannot_be_copied_without_limit() {
        // One long node name, and 40 rigid parts that name it
        let long = vec![b'a'; 1000];
        let build = |rigids: usize| {
            let (header, payload) = vertex_data(&[]);
            let mut lods = vec![0u8; 0x58];
            set32(&mut lods, 0, 0x48);
            set32(&mut lods, 4, 1);
            let mesh = mesh(&[], 0);
            let mut scene = Vec::new();
            put32(&mut scene, &[24, rigids as u32, 0, 0, 0, 0]);
            for _ in 0..rigids {
                scene.extend_from_slice(&(mesh.len() as u64).to_le_bytes());
                scene.extend_from_slice(&[0; 16]);
                put32(&mut scene, &[0, 0]);
            }
            lods[0x48..0x50].copy_from_slice(&(scene.len() as u64).to_le_bytes());
            let mut res = Vec::new();
            put32(&mut res, &[lods.len() as u32, u32::MAX, u32::MAX, 1, header.len() as u32]);
            res.extend_from_slice(&header);
            res.extend_from_slice(&payload);
            res.extend_from_slice(&lods);
            let name = std::str::from_utf8(&long).unwrap();
            let names = node_names(&[name]);
            put32(&mut res, &[names.len() as u32]);
            res.extend_from_slice(&names);
            res.extend_from_slice(&scene);
            for _ in 0..rigids {
                res.extend_from_slice(&mesh);
            }
            res
        };
        let ok = DynModel::parse(&build(1)).unwrap();
        assert_eq!(ok.lods[0].rigids[0].node.len(), 1000);
        assert!(DynModel::parse(&build(40)).is_err());
    }

    /// The skinned meshes section for one LOD with one skin on node 1 ("track"): bones on nodes 0 and 2,
    /// one skinned material (number 5) with a palette that swaps them, and an element over the triangle
    fn skins_section() -> Vec<u8> {
        let (header, payload) = vertex_data(&[(SHORT4, TEXCOORD, 4), (FLOAT4, TEXCOORD, 5)]);
        // The skin dump: pointers are offsets from its start
        let mut d = vec![0u8; 0x38];
        let mesh_at = 0x38;
        let elems_at = mesh_at + 0x28;
        let tm_at = elems_at + 48;
        let nodes_at = tm_at + 2 * 48;
        let materials_at = nodes_at + 4;
        let palettes_at = materials_at + 8;
        let slots_at = palettes_at + 16;
        set32(&mut d, 0, tm_at as u32);
        set32(&mut d, 4, 2);
        set32(&mut d, 0x10, palettes_at as u32);
        set32(&mut d, 0x14, 1);
        d[0x20..0x28].copy_from_slice(&(nodes_at as u64).to_le_bytes());
        d[0x28..0x30].copy_from_slice(&(materials_at as u64).to_le_bytes());
        d.extend_from_slice(&mesh(&[(5, 0, 0, 1, 0)], 0x38));
        // Bone 0: the identity, moved 1 along x; bone 1: moved 2 along y
        for t in [[1.0f32, 0.0, 0.0], [0.0, 2.0, 0.0]] {
            for f in [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0].iter().chain(&t) {
                d.extend_from_slice(&f.to_le_bytes());
            }
        }
        for node in [0u16, 2] {
            d.extend_from_slice(&node.to_le_bytes());
        }
        d.extend_from_slice(&[0; 0]);
        d.extend_from_slice(&5u64.to_le_bytes());
        put32(&mut d, &[slots_at as u32, 2, 0, 0]);
        for slot in [1u16, 0] {
            d.extend_from_slice(&slot.to_le_bytes());
        }
        // The stream of the zlib block: node id, size (bit 30 set), dump
        let mut stream = Vec::new();
        put32(&mut stream, &[1]);
        stream.extend_from_slice(&(d.len() as u64 | 0x4000_0000).to_le_bytes());
        stream.extend_from_slice(&d);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut z, &stream).unwrap();
        let zipped = z.finish().unwrap();

        let mut out = Vec::new();
        let total = 8 + 16 + header.len() + payload.len() + zipped.len();
        put32(&mut out, &[1, total as u32, 768, 0]);
        put32(&mut out, &[u32::MAX, u32::MAX, 1, header.len() as u32]);
        out.extend_from_slice(&header);
        out.extend_from_slice(&payload);
        out.extend_from_slice(&zipped);
        out
    }

    /// `model()` with a skinned part in its LOD, on the node named "track"
    pub(crate) fn skinned_model() -> Vec<u8> {
        let (header, payload) = vertex_data(&[]);
        let mut lods = vec![0u8; 0x58];
        set32(&mut lods, 0, 0x48);
        set32(&mut lods, 4, 1);
        let mut scene = Vec::new();
        put32(&mut scene, &[24, 0, 0, 0, 0, 1]);
        lods[0x48..0x50].copy_from_slice(&(scene.len() as u64).to_le_bytes());
        let mut res = Vec::new();
        put32(&mut res, &[lods.len() as u32, u32::MAX, u32::MAX, 1, header.len() as u32]);
        res.extend_from_slice(&header);
        res.extend_from_slice(&payload);
        res.extend_from_slice(&lods);
        let names = node_names(&["hull", "track", "wheel"]);
        put32(&mut res, &[names.len() as u32]);
        res.extend_from_slice(&names);
        res.extend_from_slice(&scene);
        res.extend_from_slice(&skins_section());
        res
    }

    #[test]
    fn a_model_with_a_skin() {
        let m = DynModel::parse(&skinned_model()).unwrap();
        assert_eq!(m.skin_vdata.len(), 1);
        assert_eq!(m.skin_vdata[0].bone_groups(), 1);
        let skin = &m.lods[0].skins[0];
        assert_eq!(skin.node, "track");
        assert_eq!(skin.elems, [Elem { material: 5, vdata: 0, first_index: 0, triangles: 1, base_vertex: 0 }]);
        assert_eq!(skin.bones.iter().map(|b| b.node.as_str()).collect::<Vec<_>>(), ["hull", "wheel"]);
        assert_eq!(skin.bones[0].inverse_bind[12..16], [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(skin.bones[1].inverse_bind[12..16], [0.0, 2.0, 0.0, 1.0]);
        assert_eq!(skin.bones[1].inverse_bind[0], 1.0);
        assert_eq!(skin.bones[1].inverse_bind[3], 0.0);
        assert_eq!(skin.materials, [SkinMaterial { material: 5, palette: vec![1, 0] }]);
        assert_eq!(skin.bone(5, 0).unwrap().node, "wheel");
    }

    #[test]
    fn a_skin_section_that_is_not_the_one_seen_is_refused() {
        let good = skinned_model();
        let section = good.len() - skins_section().len();
        // Two variants, a variant for too many constants, a gap after the entry
        for (at, v) in [(section, 2), (section + 8, 5000), (section + 12, 4)] {
            let mut bad = good.clone();
            set32(&mut bad, at, v);
            assert!(DynModel::parse(&bad).is_err(), "word at {at} = {v}");
        }
        // A section that says it is longer or shorter than it is
        for total in [0, 1, 5000, u32::MAX] {
            let mut bad = good.clone();
            set32(&mut bad, section + 4, total);
            assert!(DynModel::parse(&bad).is_err(), "total {total}");
        }
        // Skin sizes without the current-format bit, and a palette naming a bone that is not there
        let zipped_at = good.len() - 1;
        for cut in [zipped_at, zipped_at - 20] {
            assert!(DynModel::parse(&good[..cut]).is_err());
        }
    }
}
