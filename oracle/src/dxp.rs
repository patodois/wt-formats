//! `oracle dxp`: DXP texture packs, their headers, unpacked surfaces and RGBA conversions.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::common::{hash, kind, map, parallel, walk, Tally};

use wt_blk::dxp_and_grp::dxp::DxpPack as TheirPack;
use wt_formats::dxp::DxpPack as OurPack;

/// Textures with more pixels than this are decoded one at a time, so that three workers cannot
/// hold three huge ones at once
const BIG_PIXELS: u64 = 4096 * 4096;
static BIG: Mutex<()> = Mutex::new(());

pub fn run(game: &Path, threads: usize, limit: Option<usize>, filter: Option<String>) -> Tally {
    let mut files: Vec<PathBuf> = Vec::new();
    walk(game, ".dxp.bin", &mut files);
    if let Some(f) = &filter {
        files.retain(|p| p.to_string_lossy().contains(f.as_str()));
    }
    files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)));
    if let Some(n) = limit {
        files.truncate(n);
    }
    eprintln!("{} .dxp.bin files", files.len());
    let mut tally = parallel(&files, threads, |_, path, t| one(path, t));
    tally.add("dxp files", files.len() as u64);
    tally
}

/// What one library makes of one texture
#[derive(PartialEq, Debug)]
struct Outcome {
    /// Header fields, or the error kind
    texture: Result<(Vec<u64>, u128, usize), String>,
    rgba: Result<(u128, usize), String>,
}

fn one(path: &Path, t: &mut Tally) {
    let name = path.strip_prefix("/").unwrap_or(path).display().to_string();
    match map(path) {
        Ok(data) => compare_pack(&name, &data, t),
        Err(e) => t.count(format!("unreadable: {e}")),
    }
}

/// Parses `data` as a DXP pack with both libraries and compares everything, texture by texture.
pub fn compare_pack(name: &str, data: &[u8], t: &mut Tally) {
    t.add("dxp bytes", data.len() as u64);
    let (ours, theirs) = match (OurPack::parse(data), TheirPack::parse(data)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(a), Err(b)) => {
            t.count(format!("pack rejected by both: ours {} / theirs {}", kind(&a), kind(&b)));
            return;
        }
        (Ok(_), Err(wt_blk::dxp_and_grp::error::DxpGrpError::Utf8Error(_) | wt_blk::dxp_and_grp::error::DxpGrpError::CStringError(_))) => {
            t.count("intended difference: a texture name that is not UTF-8 is read lossily by ours and refuses the pack in wt_blk");
            return;
        }
        (Err(wt_formats::Error::OutOfBounds { at, .. }), Ok(_)) if at >= 1 << 32 => {
            t.count("intended difference: ours reads all 64 bits of a name pointer, wt_blk only the low 32");
            return;
        }
        (a, b) => {
            t.diff("pack verdict", format!("{name}: ours {:?}, theirs {:?}", a.err(), b.err()));
            return;
        }
    };
    t.count("packs parsed by both");
    if ours.entries.len() != theirs.entries.len() {
        t.diff("entry count", format!("{name}: ours {}, theirs {}", ours.entries.len(), theirs.entries.len()));
        return;
    }
    for (i, (a, b)) in ours.entries.iter().zip(&theirs.entries).enumerate() {
        t.count("textures");
        let ha = [
            a.header.d3d_format,
            a.header.flags,
            a.header.width.into(),
            a.header.height.into(),
            a.header.levels.into(),
            a.header.hq_part_levels.into(),
            a.header.depth.into(),
            a.header.bits_per_pixel.into(),
            a.header.dxt_shift.into(),
            a.header.mem_size,
            a.header.packed_size,
        ];
        let hb = [
            b.header.d3d_format,
            b.header.flags,
            b.header.width.into(),
            b.header.height.into(),
            b.header.levels.into(),
            b.header.hq_part_levels.into(),
            b.header.depth.into(),
            b.header.bits_per_pixel.into(),
            b.header.dxt_shift.into(),
            b.header.mem_size,
            b.header.packed_size,
        ];
        if a.name != b.name || ha != hb || a.header.is_empty() != b.header.is_empty() || a.header.format_name() != b.header.format_name() {
            t.diff("entry header", format!("{name}: #{i} {} / {}: {ha:?} vs {hb:?}", a.name, b.name));
            continue;
        }
        let h = &a.header;
        // Statistics of what the game holds
        t.count(format!("format {}", h.format_name()));
        t.count(format!("compression {:?}", h.compression()));
        if h.is_cubemap() {
            t.count("cubemaps");
        }
        if h.is_volume() {
            t.count("volumes");
        }
        if h.generates_mips() {
            t.count("textures with generated mips");
        }
        if h.flags & 0x4_0000 != 0 {
            t.count("textures with the smallest mip first");
        }
        t.count(format!("flags {:#010x}", h.flags & !0xE000_0000));
        t.max("max mem_size", u64::from(h.mem_size));
        t.max("max packed_size", u64::from(h.packed_size));
        t.max("max width", u64::from(h.width));
        t.max("max height", u64::from(h.height));
        t.max("max levels", u64::from(h.levels));
        if h.is_empty() {
            t.count("empty placeholders");
        }
        let pixels = u64::from(h.width) * u64::from(h.height);
        let _guard = (pixels >= BIG_PIXELS || h.mem_size >= 128 << 20).then(|| BIG.lock().unwrap_or_else(|e| e.into_inner()));
        let x = outcome_ours(&ours, i);
        let y = outcome_theirs(&theirs, i);
        match (&x.texture, &y.texture) {
            (Ok((_, hx, nx)), Ok((_, hy, ny))) => {
                t.count("textures unpacked by both");
                t.add("unpacked bytes", *nx as u64);
                if hx != hy || nx != ny {
                    t.diff("texture data", format!("{name}: #{i} {} ({} vs {} bytes)", a.name, nx, ny));
                    continue;
                }
                match (&x.rgba, &y.rgba) {
                    (Ok((rx, nx)), Ok((ry, ny))) => {
                        t.count("textures converted to RGBA by both");
                        if rx != ry || nx != ny {
                            t.diff("rgba", format!("{name}: #{i} {} {}", a.name, h.format_name()));
                            // How much: decode both again and count
                            if let (Ok(p), Ok(q)) = (ours.texture(i).and_then(|t| t.to_rgba()), theirs.texture(i).map_err(|e| format!("{e:?}")).and_then(|t| t.to_rgba().map_err(|e| format!("{e:?}")))) {
                                let f = h.format_name();
                                let mut differing = 0u64;
                                let mut worst = 0u8;
                                for (u, v) in p.chunks_exact(4).zip(q.chunks_exact(4)) {
                                    let d = u.iter().zip(v).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
                                    if d > 0 {
                                        differing += 1;
                                        worst = worst.max(d);
                                    }
                                }
                                t.count(format!("rgba differs [{f}]: textures"));
                                t.add(format!("rgba differs [{f}]: pixels differing"), differing);
                                t.add(format!("rgba differs [{f}]: pixels in those textures"), (p.len() / 4) as u64);
                                t.max(format!("max rgba differs [{f}]: largest channel difference"), u64::from(worst));
                                if worst > 3 {
                                    t.sample(&format!("rgba differs by more than 3 [{f}]"), format!("{name}: #{i} {} worst {worst}, {differing} pixels of {}", a.name, p.len() / 4));
                                }
                            }
                        }
                    }
                    (Err(ex), Err(ey)) => t.count(format!("rgba refused by both: ours {ex} / theirs {ey} [{}]", h.format_name())),
                    (ex, ey) => t.diff("rgba verdict", format!("{name}: #{i} {} {}: ours {ex:?}, theirs {ey:?}", a.name, h.format_name())),
                }
            }
            (Err(ex), Err(ey)) => t.count(format!("texture refused by both: ours {ex} / theirs {ey}")),
            (Err(ex), Ok(_)) if ex == "Unsupported:compression" => {
                t.count("intended difference: flag bits 5 to 7 of the compression are refused by ours, read as stored by wt_blk");
            }
            (ex, ey) => {
                t.diff("texture verdict", format!("{name}: #{i} {}: ours {:?}, theirs {:?}", a.name, ex.as_ref().err(), ey.as_ref().err()))
            }
        }
    }
}

fn outcome_ours(pack: &OurPack, i: usize) -> Outcome {
    match pack.texture(i) {
        Err(wt_formats::Error::Unsupported(why)) if why.starts_with("compression ") => {
            Outcome { texture: Err("Unsupported:compression".into()), rgba: Err("-".into()) }
        }
        Err(e) => Outcome { texture: Err(kind(&e)), rgba: Err("-".into()) },
        Ok(tex) => {
            let h = tex.header;
            let fields = vec![u64::from(h.mem_size)];
            let texture = Ok((fields, hash(&tex.data), tex.data.len()));
            let rgba = match tex.to_rgba() {
                Ok(p) => Ok((hash(&p), p.len())),
                Err(e) => Err(kind(&e)),
            };
            Outcome { texture, rgba }
        }
    }
}

fn outcome_theirs(pack: &TheirPack, i: usize) -> Outcome {
    match pack.texture(i) {
        Err(e) => Outcome { texture: Err(kind(&e)), rgba: Err("-".into()) },
        Ok(tex) => {
            let h = tex.header;
            let fields = vec![u64::from(h.mem_size)];
            let texture = Ok((fields, hash(&tex.data), tex.data.len()));
            let rgba = match tex.to_rgba() {
                Ok(p) => Ok((hash(&p), p.len())),
                Err(e) => Err(kind(&e)),
            };
            Outcome { texture, rgba }
        }
    }
}
