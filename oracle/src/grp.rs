//! `oracle grp`: GRP packs, and the DynModels and skeletons in them.

use std::path::{Path, PathBuf};

use crate::common::{kind, map, parallel, walk, Tally};

use wt_blk::dxp_and_grp::dynmodel as theirs_dm;
use wt_blk::dxp_and_grp::geom_node_tree::GeomNodeTree as TheirTree;
use wt_blk::dxp_and_grp::grp_pack::GrpPack as TheirPack;
use wt_formats::dynmodel as ours_dm;
use wt_formats::geom_node_tree::GeomNodeTree as OurTree;
use wt_formats::grp::{class_id, GrpPack as OurPack};

pub fn run(game: &Path, threads: usize, limit: Option<usize>) -> Tally {
    let mut files: Vec<PathBuf> = Vec::new();
    walk(game, ".grp", &mut files);
    files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)));
    if let Some(n) = limit {
        files.truncate(n);
    }
    eprintln!("{} .grp files", files.len());
    let mut tally = parallel(&files, threads, |_, path, t| one(path, t));
    tally.add("grp files", files.len() as u64);
    tally
}

fn one(path: &Path, t: &mut Tally) {
    let name = path.strip_prefix("/").unwrap_or(path).display().to_string();
    match map(path) {
        Ok(data) => compare_pack(&name, &data, t),
        Err(e) => t.count(format!("unreadable: {e}")),
    }
}

/// Parses `data` as a GRP pack with both libraries and compares everything.
pub fn compare_pack(name: &str, data: &[u8], t: &mut Tally) {
    t.add("grp bytes", data.len() as u64);
    let (ours, theirs) = (OurPack::parse(data), TheirPack::parse(data));
    let (ours, theirs) = match (ours, theirs) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(a), Err(b)) => {
            t.count(format!("pack rejected by both: ours {} / theirs {}", kind(&a), kind(&b)));
            return;
        }
        (Err(wt_formats::Error::OutOfBounds { at, .. }), Ok(_)) if at >= 1 << 32 => {
            t.count("intended difference: ours reads all 64 bits of a pointer field, wt_blk only the low 32");
            return;
        }
        (a, b) => {
            t.diff("pack verdict", format!("{name}: ours {:?}, theirs {:?}", a.err(), b.err()));
            return;
        }
    };
    t.count("packs parsed by both");
    if ours.resources.len() != theirs.resources.len() {
        t.diff("resource count", format!("{name}: ours {}, theirs {}", ours.resources.len(), theirs.resources.len()));
        return;
    }
    for (a, b) in ours.resources.iter().zip(&theirs.resources) {
        t.count("resources");
        t.count(format!("class {:08X}", a.class_id));
        if a.name != b.name || a.class_id != b.class_id {
            t.diff("resource entry", format!("{name}: ours {} {:08X}, theirs {} {:08X}", a.name, a.class_id, b.name, b.class_id));
            continue;
        }
        let (da, db) = (ours.data(a), theirs.data(b));
        if da != db {
            t.diff("resource bytes", format!("{name}: {} ({} vs {} bytes)", a.name, da.len(), db.len()));
            continue;
        }
        t.add("resource bytes", da.len() as u64);
        match a.class_id {
            class_id::DYN_MODEL => dyn_model(name, &a.name, da, t),
            class_id::GEOM_NODE_TREE => skeleton(name, &a.name, da, t),
            _ => {}
        }
    }
}

pub fn skeleton(file: &str, res: &str, data: &[u8], t: &mut Tally) {
    let (a, b) = (OurTree::parse(data), TheirTree::parse(data));
    let (a, b) = match (a, b) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(a), Err(b)) => {
            let k = format!("skeleton rejected by both: ours {} / theirs {}", kind(&a), kind(&b));
            t.sample(&k, format!("{file} {res}: {a} | {b:?}"));
            t.count(k);
            return;
        }
        (a, b) => {
            t.diff("skeleton verdict", format!("{file} {res}: ours {:?}, theirs {:?}", a.err(), b.err()));
            return;
        }
    };
    t.count("skeletons parsed by both");
    if a.nodes.len() != b.nodes.len() {
        t.diff("skeleton nodes", format!("{file} {res}: {} vs {}", a.nodes.len(), b.nodes.len()));
        return;
    }
    t.add("skeleton nodes", a.nodes.len() as u64);
    for (i, (x, y)) in a.nodes.iter().zip(&b.nodes).enumerate() {
        if x.name != y.name || x.parent != y.parent || !same_f32(&x.tm, &y.tm) || !same_f32(&x.wtm, &y.wtm) {
            t.diff("skeleton node", format!("{file} {res}: node {i} {} / {}", x.name, y.name));
        }
    }
}

fn same_f32(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

fn bad(t: &mut Tally, tag: &str, what: &str, detail: String) {
    t.diff(what, format!("{tag}: {detail}"));
}

pub fn dyn_model(file: &str, res: &str, data: &[u8], t: &mut Tally) {
    let (a, b) = (ours_dm::DynModel::parse(data), theirs_dm::DynModel::parse(data));
    let (a, b) = match (a, b) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(a), Err(b)) => {
            let k = format!("model rejected by both: ours {} / theirs {}", kind(&a), kind(&b));
            t.sample(&k, format!("{file} {res}: {a} | {b:?}"));
            t.count(k);
            return;
        }
        (Err(wt_formats::Error::Decompress(why)), Ok(_)) if why.starts_with("Oodle: Corrupt") => {
            t.count("difference: lztans reports a corrupt Oodle stream that oozextract decodes to something (mutated data)");
            return;
        }
        (a, b) => {
            t.diff("model verdict", format!("{file} {res}: ours {:?}, theirs {:?}", a.err(), b.err()));
            return;
        }
    };
    t.count("models parsed by both");
    let tag = format!("{file} {res}");
    if !same_f32(&a.pos_offset, &b.pos_offset) || !same_f32(&a.pos_scale, &b.pos_scale) {
        bad(t, &tag, "model position scale", format!("{:?}/{:?} vs {:?}/{:?}", a.pos_offset, a.pos_scale, b.pos_offset, b.pos_scale));
    }
    let mut vertices = 0u64;
    for v in a.vdata.iter().chain(&a.skin_vdata) {
        for c in &v.channels {
            t.count(format!("channel type {:2} usage {} index {}", c.ty, c.usage, c.index));
        }
        t.count(format!("buffer stride {}", v.stride));
    }
    for v in a.skin_vdata.iter() {
        t.count(format!("skin buffer with {} groups of bone influences", v.bone_groups()));
    }
    for (label, xs, ys, skin) in [("vdata", &a.vdata, &b.vdata, false), ("skin vdata", &a.skin_vdata, &b.skin_vdata, true)] {
        if xs.len() != ys.len() {
            bad(t, &tag, "vertex buffer count", format!("{label}: {} vs {}", xs.len(), ys.len()));
            continue;
        }
        for (i, (x, y)) in xs.iter().zip(ys).enumerate() {
            let chans_a: Vec<_> = x.channels.iter().map(|c| (c.ty, c.usage, c.index, c.offset)).collect();
            let chans_b: Vec<_> = y.channels.iter().map(|c| (c.ty, c.usage, c.index, c.offset)).collect();
            if x.stride != y.stride || chans_a != chans_b || x.indices != y.indices || x.vertices != y.vertices {
                bad(
                    t,
                    &tag,
                    "vertex buffer",
                    format!(
                        "{label} {i}: stride {}/{}, {} vs {} channels, {} vs {} indices, {} vs {} bytes",
                        x.stride,
                        y.stride,
                        chans_a.len(),
                        chans_b.len(),
                        x.indices.len(),
                        y.indices.len(),
                        x.vertices.len(),
                        y.vertices.len()
                    ),
                );
                continue;
            }
            if x.bone_groups() != y.bone_groups() || x.has_uv1() != y.has_uv1() || x.vertex_count() != y.vertex_count() {
                bad(t, &tag, "vertex buffer helpers", format!("{label} {i}"));
            }
            t.add("index buffer entries", x.indices.len() as u64);
            // Every vertex, decoded by both
            for v in 0..x.vertex_count() {
                match (a.vertex(x, v), b.vertex(y, v)) {
                    (Ok(p), Ok(q)) => {
                        if !same_f32(&p.pos, &q.pos)
                            || !same_f32(&p.normal, &q.normal)
                            || !same_f32(&p.uv, &q.uv)
                            || !same_f32(&p.uv1, &q.uv1)
                        {
                            bad(t, &tag, "decoded vertex", format!("{label} {i} vertex {v}: {p:?} vs {q:?}"));
                            break;
                        }
                    }
                    (p, q) => {
                        bad(t, &tag, "decoded vertex verdict", format!("{label} {i} vertex {v}: {:?} vs {:?}", p.err(), q.err()));
                        break;
                    }
                }
                if skin {
                    match (a.skin_weights(x, v), b.skin_weights(y, v)) {
                        (Ok(p), Ok(q)) => {
                            if p.joints != q.joints || !same_f32(&p.weights, &q.weights) {
                                bad(t, &tag, "skin weights", format!("{label} {i} vertex {v}: {p:?} vs {q:?}"));
                                break;
                            }
                        }
                        (p, q) => {
                            bad(t, &tag, "skin weights verdict", format!("{label} {i} vertex {v}: {:?} vs {:?}", p.err(), q.err()));
                            break;
                        }
                    }
                }
            }
            vertices += x.vertex_count() as u64;
            if x.vertex_count() > 0 && x.vertices.len() % x.stride.max(1) != 0 {
                t.count("vertex bytes not a whole number of vertices");
            }
        }
    }
    t.add("vertices decoded by both", vertices);
    if a.lods.len() != b.lods.len() {
        bad(t, &tag, "lod count", format!("{} vs {}", a.lods.len(), b.lods.len()));
        return;
    }
    let elems =
        |es: &[ours_dm::Elem]| es.iter().map(|e| (e.material, e.vdata, e.first_index, e.triangles, e.base_vertex)).collect::<Vec<_>>();
    let elems_b =
        |es: &[theirs_dm::Elem]| es.iter().map(|e| (e.material, e.vdata, e.first_index, e.triangles, e.base_vertex)).collect::<Vec<_>>();
    t.count(format!("models with {} lods", a.lods.len()));
    for (l, (x, y)) in a.lods.iter().zip(&b.lods).enumerate() {
        t.count("lods");
        if x.range.to_bits() != y.range.to_bits() || x.rigids.len() != y.rigids.len() || x.skins.len() != y.skins.len() {
            bad(
                t,
                &tag,
                "lod",
                format!(
                    "{l}: range {}/{}, rigids {}/{}, skins {}/{}",
                    x.range,
                    y.range,
                    x.rigids.len(),
                    y.rigids.len(),
                    x.skins.len(),
                    y.skins.len()
                ),
            );
            continue;
        }
        for (i, (p, q)) in x.rigids.iter().zip(&y.rigids).enumerate() {
            t.count("rigid parts");
            t.add("rigid elements", p.elems.len() as u64);
            if p.node != q.node || elems(&p.elems) != elems_b(&q.elems) {
                bad(t, &tag, "rigid", format!("lod {l} part {i}: {} / {}", p.node, q.node));
            }
        }
        for (i, (p, q)) in x.skins.iter().zip(&y.skins).enumerate() {
            t.count("skins");
            t.add("skin elements", p.elems.len() as u64);
            let bones_a: Vec<_> = p.bones.iter().map(|b| (b.node.clone(), b.inverse_bind.map(f32::to_bits))).collect();
            let bones_b: Vec<_> = q.bones.iter().map(|b| (b.node.clone(), b.inverse_bind.map(f32::to_bits))).collect();
            let mats_a: Vec<_> = p.materials.iter().map(|m| (m.material, m.palette.clone())).collect();
            let mats_b: Vec<_> = q.materials.iter().map(|m| (m.material, m.palette.clone())).collect();
            t.add("skin bones", p.bones.len() as u64);
            if p.node != q.node || elems(&p.elems) != elems_b(&q.elems) || bones_a != bones_b || mats_a != mats_b {
                bad(t, &tag, "skin", format!("lod {l} skin {i}: {} / {}", p.node, q.node));
            }
            // The helper that maps a slot to a bone
            for m in &p.materials {
                for slot in 0..8 {
                    let ba = p.bone(m.material, slot).map(|b| b.node.clone());
                    let bb = q.bone(m.material, slot).map(|b| b.node.clone());
                    if ba != bb {
                        bad(t, &tag, "skin bone lookup", format!("lod {l} skin {i} material {} slot {slot}: {ba:?} vs {bb:?}", m.material));
                    }
                }
            }
        }
    }
}
