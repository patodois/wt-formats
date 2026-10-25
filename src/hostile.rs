//! Hostile inputs: every reader is fed well-formed files with bytes changed, cut and replaced by
//! extreme values, and must answer with a value or an error, never panic, and never ask the allocator
//! for more than the input can account for.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::ddsx::{tests::dxt1, DdsxHeader, Texture};
use crate::dxp::{tests::pack as dxp_pack, DxpPack};
use crate::dynmodel::{tests::model, tests::skinned_model, DynModel};
use crate::geom_node_tree::{tests::tree, GeomNodeTree};
use crate::grp::{tests::pack as grp_pack, GrpPack};
use crate::meshopt::decode_index_sequence;

/// Remembers the largest single allocation of the current thread.
struct Probe;

thread_local! {
    static LARGEST: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Probe {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = LARGEST.try_with(|l| l.set(l.get().max(layout.size())));
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let _ = LARGEST.try_with(|l| l.set(l.get().max(layout.size())));
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = LARGEST.try_with(|l| l.set(l.get().max(new_size)));
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: Probe = Probe;

/// The largest allocation `f` made, and what it returned
fn probed<T>(f: impl FnOnce() -> T) -> (T, usize) {
    LARGEST.with(|l| l.set(0));
    let out = f();
    (out, LARGEST.with(|l| l.get()))
}

/// A small, fast generator; the tests must give the same inputs on every run
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
}

const EXTREMES: [u32; 10] = [0, 1, 2, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 0xFFFF_FFFE, 0x4000_0000, 0xC000_0000, 0x0100_0000];

/// A copy of `seed` with a few changes
fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut b = seed.to_vec();
    if b.is_empty() {
        return b;
    }
    for _ in 0..1 + rng.below(4) {
        match rng.below(6) {
            0 => {
                let at = rng.below(b.len());
                b[at] = rng.next() as u8;
            }
            1 | 2 => {
                if b.len() >= 4 {
                    let at = rng.below(b.len() - 3);
                    let v = EXTREMES[rng.below(EXTREMES.len())];
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
                // The same word written over another one
                if b.len() >= 8 {
                    let (from, to) = (rng.below(b.len() - 3), rng.below(b.len() - 3));
                    let w: [u8; 4] = b[from..from + 4].try_into().unwrap();
                    b[to..to + 4].copy_from_slice(&w);
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

fn iterations() -> u64 {
    std::env::var("WT_FORMATS_FUZZ_ITERATIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000)
}

/// Runs `target` on mutations of every seed. It must not panic, and must not allocate more than
/// `16 * input + 4 MiB` in one go.
fn hammer(name: &str, seeds: &[Vec<u8>], target: impl Fn(&[u8])) {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ name.len() as u64);
    for seed in seeds {
        target(seed);
    }
    for i in 0..iterations() {
        let seed = &seeds[rng.below(seeds.len())];
        let input = mutate(seed, &mut rng);
        let (outcome, largest) = probed(|| catch_unwind(AssertUnwindSafe(|| target(&input))));
        assert!(outcome.is_ok(), "{name}: panic on input #{i}: {:02x?}", &input[..input.len().min(256)]);
        assert!(
            largest <= 16 * input.len() + (4 << 20),
            "{name}: input #{i} of {} bytes made an allocation of {largest} bytes: {:02x?}",
            input.len(),
            &input[..input.len().min(256)]
        );
    }
}

#[test]
fn grp_packs() {
    let seeds = vec![
        grp_pack(&[("a", 1, b"xyz"), ("b", 2, b"12345678"), ("c", 3, b"")]),
        grp_pack(&[]),
        grp_pack(&[("only", 0xB4B7_D9C4, &[7; 100])]),
    ];
    hammer("grp", &seeds, |b| {
        if let Ok(p) = GrpPack::parse(b) {
            for r in &p.resources {
                let _ = p.data(r);
                let _ = p.find(&r.name, r.class_id);
            }
        }
    });
}

#[test]
fn dxp_packs() {
    let seeds = vec![dxp_pack(&[("one*", 1), ("two*?u03", 2)]), dxp_pack(&[("x", 9)]), dxp_pack(&[])];
    hammer("dxp", &seeds, |b| {
        if let Ok(p) = DxpPack::parse(b) {
            for i in 0..p.entries.len().min(64) {
                if let Ok(t) = p.texture(i) {
                    #[cfg(feature = "png")]
                    let _ = t.to_rgba();
                    let _ = t;
                }
            }
        }
    });
}

#[test]
fn ddsx_textures() {
    // A header and its data: plain, packed with zstd, and a cubemap with its mips reversed
    let make = |header: DdsxHeader, data: &[u8]| {
        let mut b = Vec::new();
        b.extend_from_slice(b"DDSx");
        b.extend_from_slice(&header.d3d_format.to_le_bytes());
        b.extend_from_slice(&header.flags.to_le_bytes());
        b.extend_from_slice(&header.width.to_le_bytes());
        b.extend_from_slice(&header.height.to_le_bytes());
        b.extend_from_slice(&[header.levels, header.hq_part_levels]);
        b.extend_from_slice(&header.depth.to_le_bytes());
        b.extend_from_slice(&header.bits_per_pixel.to_le_bytes());
        b.extend_from_slice(&[0, header.dxt_shift]);
        b.extend_from_slice(&header.mem_size.to_le_bytes());
        b.extend_from_slice(&header.packed_size.to_le_bytes());
        b.extend_from_slice(data);
        b
    };
    let packed = zstd::bulk::compress(&[3u8; 40], 0).unwrap();
    let seeds = vec![
        make(dxt1(8, 2, 0, 40), &[1; 40]),
        make(DdsxHeader { packed_size: packed.len() as u32, ..dxt1(8, 2, 0x2000_0000, 40) }, &packed),
        make(dxt1(4, 2, 0x4_0800, 96), &[2; 96]),
        make(DdsxHeader { depth: 4, ..dxt1(4, 3, 0x1000, 8 * 4 + 16 + 8) }, &[5; 56]),
        make(DdsxHeader { d3d_format: 21, dxt_shift: 0, bits_per_pixel: 32, ..dxt1(2, 1, 0, 16) }, &[9; 16]),
        make(DdsxHeader { packed_size: 6, ..dxt1(8, 1, 0x6000_0000, 32) }, &[0x8C, 0x06, 0, 0, 0, 0]),
    ];
    hammer("ddsx", &seeds, |b| {
        if let Ok(h) = DdsxHeader::parse(b) {
            let _ = (h.is_empty(), h.compression(), h.format_name(), h.is_cubemap(), h.is_volume(), h.generates_mips());
            if let Ok(t) = Texture::from_ddsx(h, &b[DdsxHeader::SIZE..]) {
                let _ = (t.width(), t.height());
                #[cfg(feature = "png")]
                {
                    let _ = t.to_rgba();
                    let _ = t.to_png();
                }
            }
        }
    });
}

#[test]
fn geom_node_trees() {
    let seeds = vec![tree(&[0, 0, 1, 1, 3]), tree(&[0]), tree(&[])];
    hammer("geom_node_tree", &seeds, |b| {
        if let Ok(t) = GeomNodeTree::parse(b) {
            for n in &t.nodes {
                let _ = t.find(&n.name);
            }
        }
    });
}

#[test]
fn dyn_models() {
    let seeds = vec![model(), skinned_model()];
    hammer("dynmodel", &seeds, |b| {
        if let Ok(m) = DynModel::parse(b) {
            for v in m.vdata.iter().chain(&m.skin_vdata) {
                for i in 0..v.vertex_count().min(256) {
                    let _ = m.vertex(v, i);
                    let _ = m.skin_weights(v, i);
                }
                let _ = (v.bone_groups(), v.has_uv1());
            }
            for lod in &m.lods {
                for s in &lod.skins {
                    for mat in &s.materials {
                        let _ = s.bone(mat.material, 0);
                    }
                }
                // Parsing promises that every index of an element names a vertex
                let rigid = lod.rigids.iter().flat_map(|r| &r.elems).map(|e| (e, &m.vdata));
                let skinned = lod.skins.iter().flat_map(|s| &s.elems).map(|e| (e, &m.skin_vdata));
                for (e, buffers) in rigid.chain(skinned) {
                    let buffer = &buffers[e.vdata];
                    for &i in &buffer.indices[e.first_index..e.first_index + e.triangles * 3] {
                        assert!(i as usize + e.base_vertex < buffer.vertex_count());
                    }
                }
            }
        }
    });
}

#[test]
fn index_sequences() {
    let seeds = vec![vec![0xD1, 0, 4, 4, 4, 0, 0, 0, 0], vec![0xD1, 0x80, 0x01, 0, 0, 0, 0], vec![0xD0, 0, 0, 0, 0]];
    hammer("meshopt", &seeds, |b| {
        for count in [0, 1, 3, 100, usize::MAX] {
            for wide in [false, true] {
                if let Ok(out) = decode_index_sequence(b, count, wide) {
                    assert_eq!(out.len(), count);
                }
            }
        }
    });
}

#[test]
fn random_bytes_are_refused_quickly() {
    let mut rng = Rng(7);
    for _ in 0..iterations() {
        let len = rng.below(300);
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let (outcome, largest) = probed(|| {
            catch_unwind(AssertUnwindSafe(|| {
                let _ = GrpPack::parse(&bytes);
                let _ = DxpPack::parse(&bytes);
                let _ = DynModel::parse(&bytes);
                let _ = GeomNodeTree::parse(&bytes);
                let _ = DdsxHeader::parse(&bytes);
            }))
        });
        assert!(outcome.is_ok(), "panic on {bytes:02x?}");
        assert!(largest <= 16 * len + (4 << 20), "{largest} bytes for {len}");
    }
}

/// The same checks on the real files, a few at a time, when the game is there
#[test]
fn real_resources_cut_and_changed() {
    let Some(game) = std::env::var_os("WT_GAME_DIR") else { return };
    let game = std::path::PathBuf::from(game);
    let Ok(file) = std::fs::read(game.join("content/base/res/tanks/usa_m4a1_76w_sherman.grp")) else { return };
    let pack = GrpPack::parse(&file).unwrap();
    let mut seeds = Vec::new();
    for r in &pack.resources {
        let data = pack.data(r);
        if data.len() < 400_000 && matches!(r.class_id, crate::grp::class_id::DYN_MODEL | crate::grp::class_id::GEOM_NODE_TREE) {
            seeds.push(data.to_vec());
        }
    }
    assert!(!seeds.is_empty());
    let mut rng = Rng(42);
    for i in 0..(iterations() / 10).max(200) {
        let input = mutate(&seeds[rng.below(seeds.len())], &mut rng);
        let (outcome, largest) = probed(|| {
            catch_unwind(AssertUnwindSafe(|| {
                let _ = DynModel::parse(&input);
                let _ = GeomNodeTree::parse(&input);
            }))
        });
        assert!(outcome.is_ok(), "panic on real-data mutation #{i}");
        assert!(largest <= 64 * input.len() + (64 << 20), "mutation #{i}: {largest} bytes for {}", input.len());
    }
}
