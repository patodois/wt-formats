//! Prints the bytes behind the claims of docs/FORMATS.md for one model of a GRP file, reading the file
//! with nothing but the layout notes (no wt-formats, no wt_blk): `evidence <file.grp> <resource name>`.

use std::io::Write;

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}
fn cstr(b: &[u8], at: usize) -> String {
    let end = b[at..].iter().position(|&c| c == 0).unwrap();
    String::from_utf8_lossy(&b[at..at + end]).into_owned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let file = std::fs::read(&args[1]).unwrap();
    let want = &args[2];
    let (names_at, nc) = (u32_at(&file, 0x10) as usize, u32_at(&file, 0x14) as usize);
    let (res_at, rc) = (u32_at(&file, 0x20) as usize, u32_at(&file, 0x24) as usize);
    let names: Vec<String> = (0..nc)
        .map(|i| {
            let o = u32_at(&file, names_at + 4 * i) as i32;
            if o < 0 {
                String::new()
            } else {
                cstr(&file, o as usize)
            }
        })
        .collect();
    let mut found = None;
    for i in 0..rc {
        let (class, off, nid) = (
            u32_at(&file, res_at + 12 * i),
            u32_at(&file, res_at + 12 * i + 4) as usize,
            u16::from_le_bytes([file[res_at + 12 * i + 8], file[res_at + 12 * i + 9]]) as usize,
        );
        if &names[nid] == want {
            let end = if i + 1 < rc { u32_at(&file, res_at + 12 * i + 12 + 4) as usize } else { file.len() };
            found = Some((class, off, end));
        }
    }
    let (class, off, end) = found.expect("no such resource");
    let r = &file[off..end];
    let out = std::io::stdout();
    let mut o = out.lock();
    writeln!(o, "resource {want}: class {class:08X}, file offsets {off:#x}..{end:#x} ({} bytes)", r.len()).unwrap();
    if class == 0x56F8_1B6D {
        let (ds, cw) = (u32_at(r, 0), u32_at(r, 4));
        writeln!(o, "  +0x00 dump size {ds:#x}; +0x04 count word {cw:#x} (count {}, packed bit {})", cw & 0x7FFF_FFFF, cw >> 31).unwrap();
        for n in [0usize, 1] {
            let at = 8 + n * 160;
            writeln!(
                o,
                "  node {n} at +{at:#x}: tm {} ... parent ptr {} name ptr {} -> {:?}",
                hex(&r[at..at + 16]),
                u64_at(r, at + 144),
                u64_at(r, at + 152),
                cstr(r, 8 + u64_at(r, at + 152) as usize)
            )
            .unwrap();
        }
        return;
    }
    let w: Vec<u32> = (0..5).map(|i| u32_at(r, 4 * i)).collect();
    writeln!(
        o,
        "  words: lods dump size {:#x}, textures {:#x}, materials {:#x}, buffers {}, vertex word {:#x}",
        w[0], w[1], w[2], w[3], w[4]
    )
    .unwrap();
    assert_eq!(w[4] >> 29, 6, "this tool shows packed vertex data");
    let head = u32_at(r, 20);
    let bl = (head & 0x3FFF_FFFF) as usize;
    writeln!(o, "  block head {head:#x}: tag {}, length {bl}; block starts {}", head >> 30, hex(&r[24..24 + 8])).unwrap();
    let unpacked = u32_at(r, 24) as usize;
    let mut vd = vec![0u8; unpacked];
    let n = oozextract::Extractor::new().read_from_slice(&r[28..24 + bl], &mut vd).unwrap();
    writeln!(o, "  Oodle gave {n} of {unpacked} bytes; header size (vertex word & 0x3FFFFFFF) = {:#x}", w[4] & 0x3FFF_FFFF).unwrap();
    let (tab, cnt) = (u32_at(&vd, 16) as usize, u32_at(&vd, 20) as usize);
    writeln!(o, "  header +0x10: table offset {tab:#x}, count {cnt}; first 16 bytes {}", hex(&vd[..16])).unwrap();
    for i in 0..cnt.min(3) {
        let e = tab + 32 * i;
        let (vn, w1, w2, fl, dofs, dn) =
            (u32_at(&vd, e), u32_at(&vd, e + 4), u32_at(&vd, e + 8), u32_at(&vd, e + 12), u32_at(&vd, e + 16), u32_at(&vd, e + 20));
        writeln!(
            o,
            "  buffer {i}: {} ({})",
            hex(&vd[e..e + 32]),
            format!("vertices {vn}, w1 {w1:#x} (stride {}), w2 {w2:#x}, flags {fl:#x}, declaration at {dofs:#x} x {dn}", w1 & 0xFF)
        )
        .unwrap();
        for k in 0..dn as usize {
            let d = dofs as usize + 12 * k;
            writeln!(
                o,
                "     channel {k}: {}  type {}, usage {}, index {}",
                hex(&vd[d..d + 12]),
                u32_at(&vd, d) >> 16,
                u32_at(&vd, d + 4),
                u16::from_le_bytes([vd[d + 8], vd[d + 9]])
            )
            .unwrap();
        }
        let packed = if fl & 0x200 != 0 { ((w1 >> 8) | ((w2 >> 28) << 24)) as usize } else { (w2 & 0x0FFF_FFFF) as usize };
        writeln!(o, "     vertex bytes {} + index bytes {}", vn as usize * (w1 & 0xFF) as usize, packed).unwrap();
    }
    // The rest of the resource, past the block
    let mut at = 24 + bl;
    let lods_size = w[0] as usize;
    let lods = &r[at..at + lods_size];
    writeln!(
        o,
        "  lods dump at +{at:#x} ({lods_size} bytes): table {:#x} x {}; offset vec3 at 0x28: {:?}; scale vec3 at 0x38: {:?}",
        u32_at(lods, 0),
        u32_at(lods, 4),
        [f32_at(lods, 0x28), f32_at(lods, 0x2C), f32_at(lods, 0x30)],
        [f32_at(lods, 0x38), f32_at(lods, 0x3C), f32_at(lods, 0x40)]
    )
    .unwrap();
    at += lods_size;
    let ns = u32_at(r, at) as usize;
    let nm = &r[at + 4..at + 4 + ns];
    writeln!(
        o,
        "  node name map at +{at:#x}: size {ns}; names table {:#x} x {}; ids table {:#x} x {}; first name {:?}",
        u32_at(nm, 0),
        u32_at(nm, 4),
        u32_at(nm, 0x10),
        u32_at(nm, 0x14),
        cstr(nm, u64_at(nm, u32_at(nm, 0) as usize) as usize)
    )
    .unwrap();
    at += 4 + ns;
    let (lt, lc) = (u32_at(lods, 0) as usize, u32_at(lods, 4) as usize);
    for l in 0..lc {
        let (ssz, range) = (u64_at(lods, lt + 16 * l) as usize, f32_at(lods, lt + 16 * l + 8));
        let scene = &r[at..at + ssz];
        let (ro, rcnt, skins) = (u32_at(scene, 0), u32_at(scene, 4), u32_at(scene, 20));
        let show = l < 2;
        if show {
            writeln!(o, "  LOD {l}: scene {ssz} bytes at +{at:#x}, range {range}; rigids at {ro:#x} x {rcnt}, skins {skins}").unwrap();
        }
        at += ssz;
        let e = ro as usize;
        let mut first_mesh = None;
        for k in 0..rcnt as usize {
            let ms = u64_at(scene, e + 32 * k) as usize;
            if k == 0 && show {
                writeln!(o, "     rigid 0: {} (mesh {ms} bytes, node id {})", hex(&scene[e..e + 32]), u32_at(scene, e + 24)).unwrap();
                first_mesh = Some(&r[at..at + ms]);
            }
            at += ms;
        }
        if let Some(m) = first_mesh.filter(|_| show) {
            writeln!(o, "     mesh 0 header: {}", hex(&m[..0x28.min(m.len())])).unwrap();
            let (eo, ec) = (u32_at(m, 0) as usize, u32_at(m, 4));
            if ec > 0 {
                writeln!(o, "     element 0: {}", hex(&m[eo..eo + 48])).unwrap();
            }
        }
    }
    writeln!(o, "  after the LODs: +{at:#x} of {}", r.len()).unwrap();
    let skin_total: u32 = (0..lc)
        .map(|l| {
            let ssz = u64_at(lods, lt + 16 * l) as usize;
            let _ = ssz;
            0u32
        })
        .sum();
    let _ = skin_total;
    if at + 8 < r.len() {
        let (variants, total) = (u32_at(r, at), u32_at(r, at + 4) as usize);
        writeln!(
            o,
            "  skin section at +{at:#x}: variants {variants}, total {total}; entry (constants, skip) = ({}, {}); then {}",
            u32_at(r, at + 8),
            u32_at(r, at + 12),
            hex(&r[at + 16..at + 32])
        )
        .unwrap();
        let w2: Vec<u32> = (0..4).map(|i| u32_at(r, at + 16 + 4 * i)).collect();
        writeln!(o, "    its vertex word {:#x}; the section ends at +{:#x}", w2[3], at + 8 + total).unwrap();
        let bl2 = (u32_at(r, at + 32) & 0x3FFF_FFFF) as usize;
        let after = at + 36 + bl2;
        writeln!(o, "    vertex block length {bl2}; then zlib stream at +{after:#x}: {}", hex(&r[after..after + 8])).unwrap();
        let mut z = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&r[after..at + 8 + total]), &mut z).unwrap();
        writeln!(o, "    unzipped {} bytes: first words {}", z.len(), hex(&z[..16.min(z.len())])).unwrap();
        // One skin per LOD 0 entry: node ids (u32 each), sizes (u64 each)
        let skins = {
            let ssz = u64_at(lods, lt) as usize;
            let _ = ssz;
            0
        };
        let _ = skins;
        // Assume the first LOD's skin count from the first words: count = (ids until the sizes): read as ids/sizes pairs
        let id0 = u32_at(&z, 0);
        writeln!(o, "    first u32 {id0} (node id of the first skin)").unwrap();
    }
    if lc > 0 && at + 8 <= r.len() {
        writeln!(o, "  next 24 bytes: {}", hex(&r[at..(at + 24).min(r.len())])).unwrap();
    }
}
