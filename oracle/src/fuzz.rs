//! `oracle fuzz`: inputs that were never in a game, given to both libraries. They must agree on the
//! verdict (value or error) and, when both give a value, on the value; neither may panic.
//!
//! With a game directory the seeds are real resources and textures, mutated; without it only
//! the synthetic inputs run.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use crate::common::{hash, kind, walk, Tally};
use crate::{dxp, grp};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

const EXTREMES: [u32; 12] =
    [0, 1, 2, 3, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 0xFFFF_FFFE, 0x4000_0000, 0xC000_0000, 0x0100_0000, 0x0000_FFFF];

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut b = seed.to_vec();
    if b.is_empty() {
        return b;
    }
    for _ in 0..1 + rng.below(4) {
        match rng.below(7) {
            0 => {
                let at = rng.below(b.len());
                b[at] = rng.next() as u8;
            }
            1 | 2 => {
                if b.len() >= 4 {
                    let at = rng.below(b.len() - 3) & !3;
                    let v = rng.pick(&EXTREMES);
                    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            3 => {
                let at = rng.below(b.len());
                b.truncate(at);
                if b.is_empty() {
                    return b;
                }
            }
            4 => {
                if b.len() >= 8 {
                    let (from, to) = (rng.below(b.len() - 3), rng.below(b.len() - 3));
                    let w: [u8; 4] = b[from..from + 4].try_into().unwrap();
                    b[to..to + 4].copy_from_slice(&w);
                }
            }
            5 => {
                // A small change to a number: the commonest way for a count to go wrong
                if b.len() >= 4 {
                    let at = rng.below(b.len() - 3) & !3;
                    let v = u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
                    let d = rng.pick(&[1u32, 2, 3, 0xFFFF_FFFF, 0xFFFF_FFFE, 16, 0xFFFF_FFF0]);
                    b[at..at + 4].copy_from_slice(&v.wrapping_add(d).to_le_bytes());
                }
            }
            _ => {
                let at = rng.below(b.len());
                b[at] ^= 1 << rng.below(8);
            }
        }
    }
    b
}

/// Runs a comparison, counting a panic of either library instead of dying with it.
fn guarded(t: &mut Tally, what: &str, f: impl FnOnce(&mut Tally)) {
    let before = t.total_diffs;
    let r = catch_unwind(AssertUnwindSafe(|| f(t)));
    if r.is_err() {
        t.diff("panic", format!("{what}: one of the libraries panicked"));
    }
    let _ = before;
}

pub fn run(iterations: u64, seed: u64, game: Option<PathBuf>) -> Tally {
    // The default hook prints every caught Oodle panic
    std::panic::set_hook(Box::new(|_| {}));
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xBF58_476D_1CE4_E5B9));
    let mut t = Tally::default();
    meshopt(&mut rng, iterations, &mut t);
    ddsx_synthetic(&mut rng, iterations / 2, &mut t);
    if let Some(game) = game {
        real_seeds(&game, &mut rng, iterations, &mut t);
    }
    t
}

// ---------------------------------------------------------------------------------------------

fn meshopt(rng: &mut Rng, iterations: u64, t: &mut Tally) {
    use wt_blk::dxp_and_grp::meshopt::decode_index_sequence as theirs;
    use wt_formats::meshopt::decode_index_sequence as ours;
    for i in 0..iterations {
        // Half the inputs are encodings of random index lists with a change or two, half are noise
        let (data, count, wide) = if rng.below(2) == 0 {
            let n = rng.below(40);
            let mut out = vec![0xD0 | rng.below(3) as u8];
            for _ in 0..n {
                let mut v = rng.next() as u32 >> rng.below(32);
                loop {
                    let low = (v & 0x7F) as u8;
                    v >>= 7;
                    if v == 0 {
                        out.push(low);
                        break;
                    }
                    out.push(low | 0x80);
                }
            }
            out.extend_from_slice(&[0; 4]);
            let out = if rng.below(3) == 0 { mutate(&out, rng) } else { out };
            let count = if rng.below(4) == 0 { rng.below(60) } else { n };
            (out, count, rng.below(2) == 0)
        } else {
            let len = rng.below(48);
            ((0..len).map(|_| rng.next() as u8).collect(), rng.below(48), rng.below(2) == 0)
        };
        guarded(t, "meshopt", |t| match (ours(&data, count, wide), theirs(&data, count, wide)) {
            (Ok(a), Ok(b)) => {
                t.count("meshopt: both decode");
                if a != b {
                    t.diff("meshopt output", format!("#{i}: {data:02x?} count {count} wide {wide}"));
                }
            }
            (Err(_), Err(_)) => t.count("meshopt: both refuse"),
            (a, b) => {
                t.diff("meshopt verdict", format!("#{i}: {data:02x?} count {count} wide {wide}: ours {:?}, theirs {:?}", a.err(), b.err()))
            }
        });
    }
}

// ---------------------------------------------------------------------------------------------

fn ddsx_synthetic(rng: &mut Rng, iterations: u64, t: &mut Tally) {
    use wt_blk::dxp_and_grp::ddsx as their;
    use wt_formats::ddsx as our;
    let formats: [u32; 14] = [
        u32::from_le_bytes(*b"DXT1"),
        u32::from_le_bytes(*b"DXT3"),
        u32::from_le_bytes(*b"DXT5"),
        u32::from_le_bytes(*b"ATI1"),
        u32::from_le_bytes(*b"ATI2"),
        u32::from_le_bytes(*b"BC7 "),
        21,
        28,
        32,
        50,
        111,
        113,
        23,
        0,
    ];
    for i in 0..iterations {
        let format = if rng.below(8) == 0 { rng.next() as u32 } else { rng.pick(&formats) };
        let mut flags = 0u32;
        for bit in [0x800u32, 0x1000, 0x2000, 0x4000, 0x20000, 0x40000, 0x200000, 0x11] {
            if rng.below(4) == 0 {
                flags |= bit;
            }
        }
        let comp = rng.pick(&[0u32, 0, 0, 1, 1, 3, 3, 2, 4, 5, 6, 7]);
        flags |= comp << 29;
        let side = |rng: &mut Rng| -> u16 { rng.pick(&[0u16, 1, 2, 3, 4, 5, 7, 8, 16, 33, 64, 100, 255, 256]) };
        let (w, h) = (side(rng), side(rng));
        let levels = rng.pick(&[0u8, 1, 1, 2, 3, 5, 9, 12, 40, 255]);
        let depth = rng.pick(&[0u16, 1, 2, 4, 9]);
        let bpp = rng.pick(&[0u16, 8, 16, 32, 64, 24]);
        let shift = rng.pick(&[0u8, 3, 4, 4, 5, 15]);
        // Surface data that is right for the header, or off by a little
        let guess = {
            let (mut total, mut ww, mut hh) = (0u64, u64::from(w).max(1), u64::from(h).max(1));
            for _ in 0..levels.max(1).min(20) {
                total += if shift == 0 { ww * hh * u64::from(bpp) / 8 } else { ww.div_ceil(4) * hh.div_ceil(4) << shift };
                ww = (ww / 2).max(1);
                hh = (hh / 2).max(1);
            }
            total.min(1 << 18)
        };
        let mem_size = match rng.below(6) {
            0 => rng.next() as u32,
            1 => (guess as u32).wrapping_add(rng.below(5) as u32).wrapping_sub(2),
            _ => guess as u32,
        };
        let raw_plain: Vec<u8> = (0..(mem_size as usize).min(1 << 18) + rng.below(8)).map(|k| (k * 7 + i as usize) as u8).collect();
        let (raw, packed_size) = match comp {
            1 => {
                let z = zstd::bulk::compress(&raw_plain, 1).unwrap();
                let n = z.len() as u32;
                (z, if rng.below(8) == 0 { rng.next() as u32 } else { n })
            }
            3 => {
                // Oodle's stored blocks: a two-byte header, then up to 256 KiB as they are
                let s: Vec<u8> = raw_plain.chunks(0x40000).flat_map(|c| [&[0x4C, 0x06][..], c].concat()).collect();
                let n = s.len() as u32;
                (s, n)
            }
            _ => (raw_plain, 0),
        };
        let mut header = Vec::with_capacity(32);
        header.extend_from_slice(b"DDSx");
        header.extend_from_slice(&format.to_le_bytes());
        header.extend_from_slice(&flags.to_le_bytes());
        header.extend_from_slice(&w.to_le_bytes());
        header.extend_from_slice(&h.to_le_bytes());
        header.extend_from_slice(&[levels, 0]);
        header.extend_from_slice(&depth.to_le_bytes());
        header.extend_from_slice(&bpp.to_le_bytes());
        header.extend_from_slice(&[0x12, shift | 0x70]);
        header.extend_from_slice(&mem_size.to_le_bytes());
        header.extend_from_slice(&packed_size.to_le_bytes());
        let header = if rng.below(10) == 0 { mutate(&header, rng) } else { header };
        guarded(t, "ddsx", |t| compare_texture(&header, &raw, &format!("#{i}"), t));
        let _ = (&our::DdsxHeader::SIZE, &their::DdsxHeader::SIZE);
    }
}

/// One header and the bytes after it, given to both `Texture::from_ddsx`.
fn compare_texture(header: &[u8], raw: &[u8], what: &str, t: &mut Tally) {
    use wt_blk::dxp_and_grp::ddsx as their;
    use wt_formats::ddsx as our;
    let (a, b) = (our::DdsxHeader::parse(header), their::DdsxHeader::parse(header));
    let (a, b) = match (a, b) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(_), Err(_)) => {
            t.count("ddsx header: both refuse");
            return;
        }
        (a, b) => {
            t.diff("ddsx header verdict", format!("{what}: {header:02x?}: ours {:?}, theirs {:?}", a.err(), b.err()));
            return;
        }
    };
    let fields = |h: &our::DdsxHeader| {
        [
            h.d3d_format,
            h.flags,
            h.width.into(),
            h.height.into(),
            h.levels.into(),
            h.hq_part_levels.into(),
            h.depth.into(),
            h.bits_per_pixel.into(),
            h.dxt_shift.into(),
            h.mem_size,
            h.packed_size,
        ]
    };
    let fields_b = |h: &their::DdsxHeader| {
        [
            h.d3d_format,
            h.flags,
            h.width.into(),
            h.height.into(),
            h.levels.into(),
            h.hq_part_levels.into(),
            h.depth.into(),
            h.bits_per_pixel.into(),
            h.dxt_shift.into(),
            h.mem_size,
            h.packed_size,
        ]
    };
    if fields(&a) != fields_b(&b) || a.is_empty() != b.is_empty() || a.format_name() != b.format_name() {
        t.diff("ddsx header fields", format!("{what}: {:?} vs {:?}", fields(&a), fields_b(&b)));
        return;
    }
    let comp_a = format!("{:?}", a.compression());
    let comp_b = format!("{:?}", b.compression());
    if comp_a != comp_b && !(comp_a.starts_with("Unknown") && comp_b == "None") {
        t.diff("ddsx compression", format!("{what}: flags {:#x}: ours {comp_a}, theirs {comp_b}", a.flags));
    }
    if (a.is_cubemap(), a.is_volume(), a.generates_mips()) != (b.is_cubemap(), b.is_volume(), b.generates_mips()) {
        t.diff("ddsx flag helpers", what.to_string());
    }
    match (our::Texture::from_ddsx(a, raw), their::Texture::from_ddsx(b, raw)) {
        (Ok(x), Ok(y)) => {
            t.count("ddsx texture: both unpack");
            if x.data != y.data {
                t.diff("ddsx data", format!("{what}: {:?} ({} vs {} bytes)", fields(&a), x.data.len(), y.data.len()));
                return;
            }
            match (x.to_rgba(), y.to_rgba()) {
                (Ok(p), Ok(q)) => {
                    t.count("ddsx rgba: both convert");
                    if hash(&p) != hash(&q) {
                        t.diff("ddsx rgba", format!("{what}: {:?}", fields(&a)));
                    }
                }
                (Err(_), Err(_)) => t.count("ddsx rgba: both refuse"),
                (p, q) => t.diff("ddsx rgba verdict", format!("{what}: {:?}: ours {:?}, theirs {:?}", fields(&a), p.err(), q.err())),
            }
        }
        (Err(_), Err(_)) => t.count("ddsx texture: both refuse"),
        (Err(wt_formats::Error::Unsupported(why)), Ok(_)) if why.starts_with("compression ") && (a.flags >> 29) >= 5 => {
            // Top flag bits 5 to 7 name no packer; ours refuses them, wt_blk reads the data as stored
            t.count("intended difference: flag bits 5 to 7 of the compression are refused by ours, read as stored by wt_blk");
        }
        (Ok(x), Err(wt_blk::dxp_and_grp::error::DxpGrpError::Decompression(why)))
            if why.contains("Oodle data cannot unpack")
                && raw.len() < 6
                && raw.starts_with(&[0x4C, 0x06])
                && x.data == raw[2..2 + x.data.len()] =>
        {
            // A stored Oodle block of 1 to 3 bytes is a valid stream; wt_blk's bound of 6 bytes per block refuses it
            t.count("difference, ours right: a valid Oodle stream of under 6 bytes (a stored block of 1 to 3 bytes) is refused by wt_blk's size bound");
        }
        (x, y) => {
            let k = format!("ddsx texture verdict: ours {:?} / theirs {:?}", x.as_ref().err().map(kind), y.as_ref().err().map(kind));
            t.diff(&k, format!("{what}: {:?} (data {} bytes): ours {:?}, theirs {:?}", fields(&a), raw.len(), x.err(), y.err()));
        }
    }
}

// ---------------------------------------------------------------------------------------------

fn real_seeds(game: &Path, rng: &mut Rng, iterations: u64, t: &mut Tally) {
    use wt_formats::grp::{class_id, GrpPack};
    // Models and skeletons of a few packs
    let mut models: Vec<Vec<u8>> = Vec::new();
    let mut trees: Vec<Vec<u8>> = Vec::new();
    for rel in ["content/base/res/tanks/usa_m4a1_76w_sherman.grp", "content/base/res/tanks/germ_pzkpfw_vi_ausf_b_tiger_ii_h.grp"] {
        let Ok(file) = std::fs::read(game.join(rel)) else { continue };
        let Ok(pack) = GrpPack::parse(&file) else { continue };
        for r in &pack.resources {
            let d = pack.data(r);
            if d.len() > 600_000 {
                continue;
            }
            match r.class_id {
                class_id::DYN_MODEL => models.push(d.to_vec()),
                class_id::GEOM_NODE_TREE => trees.push(d.to_vec()),
                _ => {}
            }
        }
    }
    eprintln!("fuzz seeds: {} models, {} skeletons", models.len(), trees.len());
    // Small packs: a whole GRP and a whole DXP, for the tables
    let small = |suffix: &str, limit: u64| -> Vec<Vec<u8>> {
        let mut files = Vec::new();
        walk(game, suffix, &mut files);
        files.retain(|p| std::fs::metadata(p).map(|m| m.len() <= limit).unwrap_or(false));
        files.sort();
        files.iter().step_by((files.len() / 60).max(1)).filter_map(|p| std::fs::read(p).ok()).collect()
    };
    let grps = small(".grp", 150_000);
    let dxps = small(".dxp.bin", 150_000);
    eprintln!("fuzz seeds: {} small GRP packs, {} small DXP packs", grps.len(), dxps.len());

    let mut saved = 0;
    for i in 0..iterations {
        let which = rng.below(10);
        let before = t.total_diffs;
        let mut last: Option<Vec<u8>> = None;
        if which < 3 && !models.is_empty() {
            let input = mutate(&models[rng.below(models.len())], rng);
            guarded(t, "model", |t| grp::dyn_model("mutated", &format!("#{i}"), &input, t));
            last = Some(input);
        } else if which < 4 && !trees.is_empty() {
            let input = mutate(&trees[rng.below(trees.len())], rng);
            guarded(t, "skeleton", |t| grp::skeleton("mutated", &format!("#{i}"), &input, t));
            last = Some(input);
        } else if which < 7 && !grps.is_empty() {
            let input = mutate(&grps[rng.below(grps.len())], rng);
            guarded(t, "grp", |t| grp::compare_pack(&format!("mutated #{i}"), &input, t));
            last = Some(input);
        } else if !dxps.is_empty() {
            let input = mutate(&dxps[rng.below(dxps.len())], rng);
            guarded(t, "dxp", |t| dxp::compare_pack(&format!("mutated #{i}"), &input, t));
            last = Some(input);
        }
        if t.total_diffs > before && saved < 40 {
            if let Some(input) = last {
                let _ = std::fs::create_dir_all("out/fuzz-cases");
                let _ = std::fs::write(format!("out/fuzz-cases/case-{i}.bin"), input);
                saved += 1;
            }
        }
        if i % 5000 == 0 {
            eprintln!("  fuzz {i}/{iterations}");
        }
    }
    let _ = hash;
}
