//! Checks wt-unpack's own vromfs and BLK readers against wt_blk on every `.vromfs.bin` of an install.
//!
//! wt-unpack does not use wt_blk for these (it has `src/vromfs.rs` and `src/blk.rs`); this is a check of
//! those readers, run here because the same oracle method applies. For every archive: the files and their
//! unpacked bytes must be identical, and every binary BLK must parse to the same names, types, values
//! and order.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use std::sync::{Arc, Mutex};
use wt_blk::blk::blk_structure::BlkField;
use wt_blk::blk::blk_type::BlkType;

use wt_blk::vromf::{ContinueMode, File, FileFilter, VromfUnpacker};
use wt_unpack::blk::{self as ours_blk, Block, Value};
use wt_unpack::vromfs::{shared_context, Vromfs};

#[derive(Default)]
struct Tally {
    counts: BTreeMap<String, u64>,
    diffs: Vec<String>,
    total_diffs: u64,
    samples: BTreeMap<String, Vec<String>>,
}

impl Tally {
    fn count(&mut self, k: impl Into<String>) {
        *self.counts.entry(k.into()).or_default() += 1;
    }
    fn add(&mut self, k: impl Into<String>, n: u64) {
        *self.counts.entry(k.into()).or_default() += n;
    }
    fn sample(&mut self, k: &str, d: String) {
        let v = self.samples.entry(k.to_owned()).or_default();
        if v.len() < 6 {
            v.push(d);
        }
    }
    fn diff(&mut self, kind: &str, detail: String) {
        self.total_diffs += 1;
        let n = self.counts.entry(format!("DIFF {kind}")).or_default();
        *n += 1;
        if *n <= 10 {
            self.diffs.push(format!("{kind}: {detail}"));
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ty) = e.file_type() else { continue };
        let p = e.path();
        if ty.is_dir() {
            walk(&p, out);
        } else if ty.is_file() && p.to_string_lossy().ends_with(".vromfs.bin") {
            out.push(p);
        }
    }
}

fn hash(b: &[u8]) -> u128 {
    xxhash_rust::xxh3::xxh3_128(b)
}

fn main() {
    let game = PathBuf::from(std::env::args().nth(1).expect("game dir"));
    let mut archives = Vec::new();
    walk(&game, &mut archives);
    archives.sort();
    if let Some(f) = std::env::args().nth(2) {
        archives.retain(|p| p.to_string_lossy().contains(&f));
    }
    let mut t = Tally::default();
    for path in &archives {
        let name = path.strip_prefix(&game).unwrap_or(path).display().to_string();
        eprintln!("{name}");
        one(&name, path, &mut t);
    }
    println!("---- counts");
    for (k, v) in &t.counts {
        println!("{k}: {v}");
    }
    println!(
        "colours with red and blue differing, where wt_blk's r holds our blue: {}",
        SWAPPED_COLORS.load(std::sync::atomic::Ordering::Relaxed)
    );
    println!("---- examples");
    for (k, v) in &t.samples {
        for d in v {
            println!("{k}: {d}");
        }
    }
    println!("---- differences: {}", t.total_diffs);
    for d in &t.diffs {
        println!("{d}");
    }
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    println!("peak RSS: {:.2} GB", ru.ru_maxrss as f64 / 1e9);
    if t.total_diffs > 0 {
        std::process::exit(1);
    }
}

/// A writer that keeps only the size and a hash of what it is given, so that wt_blk's unpacked files never
/// pile up in memory (the biggest archive is 630 MB)
struct Digest {
    name: String,
    len: usize,
    hasher: xxhash_rust::xxh3::Xxh3,
    sink: Arc<Mutex<Vec<(String, usize, u128)>>>,
}

impl std::io::Write for Digest {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.len += buf.len();
        self.hasher.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Digest {
    fn drop(&mut self) {
        self.sink.lock().unwrap().push((std::mem::take(&mut self.name), self.len, self.hasher.digest128()));
    }
}

fn one(name: &str, path: &Path, t: &mut Tally) {
    // One copy of the archive in memory: ours reads it, then wt_blk takes it over
    let bytes = std::fs::read(path).unwrap();
    t.count("archives");
    t.add("archive bytes", bytes.len() as u64);

    let mut ours_files: BTreeMap<String, (usize, u128)> = BTreeMap::new();
    let mut ours_blks: BTreeMap<String, Result<Block, String>> = BTreeMap::new();
    let ours_meta;
    {
        match Vromfs::parse(&bytes) {
            Ok(v) => {
                ours_meta = Some((v.target.clone(), v.version.clone(), v.md5_ok));
                let shared = shared_context(&v);
                for file in &v.files {
                    // wt_blk calls the shared name map `nm`; its name in the file is the bytes ff 3f 6e 6d
                    let key = if file.name == wt_unpack::vromfs::SHARED_NAMEMAP_NAME { "nm".to_owned() } else { file.name.clone() };
                    ours_files.insert(key, (file.data.len(), hash(&file.data)));
                    // BLK files are the ones named .blk whose first byte is a BLK format
                    if file.name.ends_with(".blk") && ours_blk::is_binary(&file.data) {
                        let r = match &shared {
                            Ok(ctx) => ours_blk::parse(&file.data, ctx.as_ref()).map_err(|e| format!("{e:#}")),
                            Err(e) => Err(format!("shared context: {e:#}")),
                        };
                        ours_blks.insert(file.name.clone(), r);
                    }
                }
                t.add("files (ours)", v.files.len() as u64);
            }
            Err(e) => {
                ours_meta = None;
                t.sample("ours failed", format!("{name}: {e:#}"));
            }
        }
    }
    let ours_ok = ours_meta.is_some();

    let theirs = VromfUnpacker::from_file(&File::from_raw(PathBuf::from(name), bytes), true, false);
    let theirs = match theirs {
        Ok(v) => v,
        Err(e) => {
            if ours_ok {
                t.diff("archive verdict", format!("{name}: ours parses, theirs fails: {e}"));
            } else {
                t.count("archive refused by both");
            }
            return;
        }
    };
    let Some((target, version, md5_ok)) = ours_meta else {
        t.diff("archive verdict", format!("{name}: theirs parses, ours fails"));
        return;
    };
    t.count(format!("archive: version {version:?}"));
    t.count(format!("archive: target {target:?}"));
    t.count(format!("archive: md5 ok (ours): {md5_ok}"));

    // Every BLK first, while wt_blk still holds the archive
    let mut theirs_blk_count = 0;
    let blk_names: Vec<String> = ours_blks.keys().cloned().collect();
    for fname in &blk_names {
        let file = match theirs.unpack_one(Path::new(fname), None, false) {
            Ok(f) => f,
            Err(e) => {
                t.diff("file missing in theirs", format!("{name}: {fname}: {e}"));
                continue;
            }
        };
        if !wt_blk::blk::util::maybe_blk(&file) {
            t.diff("blk verdict", format!("{name}: {fname}: ours treats it as a binary BLK, theirs does not"));
            continue;
        }
        theirs_blk_count += 1;
        let mut buf = file.split().1;
        let parsed = wt_blk::blk::unpack_blk(&mut buf, theirs.dict(), theirs.nm());
        match (ours_blks.remove(fname).unwrap(), parsed) {
            (Ok(a), Ok(b)) => {
                t.count("blk parsed by both");
                let mut d = Vec::new();
                compare(&a, &b, &mut d, fname, 0);
                t.add("blk values compared", count_values(&a));
                for x in d.iter().take(3) {
                    t.diff("blk structure", format!("{name}: {fname}: {x}"));
                }
            }
            (Err(a), Err(b)) => {
                t.count("blk refused by both");
                t.sample("blk refused by both", format!("{name}: {fname}: ours {a} | theirs {b}"));
            }
            (a, b) => t.diff("blk verdict", format!("{name}: {fname}: ours {:?}, theirs {:?}", a.err(), b.err().map(|e| e.to_string()))),
        }
    }
    t.add("binary blk files compared", theirs_blk_count);

    // Then the raw bytes of every file, hashed as wt_blk writes them
    let sink = Arc::new(Mutex::new(Vec::new()));
    let make = {
        let sink = sink.clone();
        move |file: &mut File| {
            Ok(Digest {
                name: file.path().to_string_lossy().into_owned(),
                len: 0,
                hasher: xxhash_rust::xxh3::Xxh3::new(),
                sink: sink.clone(),
            })
        }
    };
    if let Err(e) = theirs.unpack_all_with_writer(None, false, make, false, FileFilter::all(), ContinueMode::ExitOnFirstError) {
        t.diff("theirs unpack", format!("{name}: {e}"));
        return;
    }
    let theirs_files = std::mem::take(&mut *sink.lock().unwrap());
    if theirs_files.len() != ours_files.len() {
        t.diff("file count", format!("{name}: ours {}, theirs {}", ours_files.len(), theirs_files.len()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (fname, len, h) in &theirs_files {
        seen.insert(fname.clone());
        match ours_files.get(fname) {
            Some(&(olen, oh)) => {
                t.count("files with the same name in both");
                if olen != *len || oh != *h {
                    t.diff("file bytes", format!("{name}: {fname} ({olen} vs {len} bytes)"));
                } else {
                    t.add("identical unpacked bytes", olen as u64);
                    t.count("files byte-identical");
                }
            }
            None => t.diff("file missing in ours", format!("{name}: {fname}")),
        }
    }
    for n in ours_files.keys() {
        if !seen.contains(n) {
            t.diff("file missing in theirs", format!("{name}: {n}"));
        }
    }
    for n in ours_blks.keys() {
        t.diff("blk missing in theirs", format!("{name}: {n}"));
    }
}

fn count_values(b: &Block) -> u64 {
    b.params.len() as u64 + b.blocks.iter().map(count_values).sum::<u64>()
}

/// Does `a` (ours) hold what `b` (wt_blk's tree) holds? Params and blocks keep the order they have in the file.
fn compare(a: &Block, b: &BlkField, out: &mut Vec<String>, path: &str, depth: usize) {
    if out.len() > 3 {
        return;
    }
    let BlkField::Struct(_, fields) = b else {
        out.push(format!("{path}: theirs is not a struct"));
        return;
    };
    let mut params = Vec::new();
    let mut blocks = Vec::new();
    for f in fields {
        match f {
            BlkField::Value(n, v) => params.push((n.to_string(), v)),
            BlkField::Struct(n, _) => blocks.push((n.to_string(), f)),
            BlkField::Merged(..) => out.push(format!("{path}: merged field in an unmerged tree")),
        }
    }
    if a.params.len() != params.len() || a.blocks.len() != blocks.len() {
        out.push(format!("{path}: {} params / {} blocks vs {} / {}", a.params.len(), a.blocks.len(), params.len(), blocks.len()));
        return;
    }
    for (i, ((an, av), (bn, bv))) in a.params.iter().zip(&params).enumerate() {
        if an != bn || !same_value(av, bv) {
            out.push(format!("{path}: param {i} {an} {av:?} vs {bn} {bv:?}"));
            return;
        }
    }
    for ((ab, (bn, bb)), i) in a.blocks.iter().zip(&blocks).zip(0..) {
        if &ab.name != bn {
            out.push(format!("{path}: block {i} {} vs {bn}", ab.name));
            return;
        }
        compare(ab, bb, out, &format!("{path}/{bn}"), depth + 1);
    }
}

static SWAPPED_COLORS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn same_value(a: &Value, b: &BlkType) -> bool {
    let f = |x: f32, y: f32| x.to_bits() == y.to_bits();
    match (a, b) {
        (Value::Str(x), BlkType::Str(y)) => x.as_str() == y.as_str(),
        (Value::Int(x), BlkType::Int(y)) => x == y,
        (Value::Real(x), BlkType::Float(y)) => f(*x, *y),
        (Value::Point2(x), BlkType::Float2(y)) => x.iter().zip(y).all(|(p, q)| f(*p, *q)),
        (Value::Point3(x), BlkType::Float3(y)) => x.iter().zip(y).all(|(p, q)| f(*p, *q)),
        (Value::Point4(x), BlkType::Float4(y)) => x.iter().zip(y.iter()).all(|(p, q)| f(*p, *q)),
        (Value::IPoint2(x), BlkType::Int2(y)) => x == y,
        (Value::IPoint3(x), BlkType::Int3(y)) => x == y,
        (Value::IPoint4(x), BlkType::Int4(y)) => x == &**y,
        (Value::Bool(x), BlkType::Bool(y)) => x == y,
        // The file stores a colour as the bytes blue, green, red, alpha (a Direct3D colour). Ours returns
        // [red, green, blue, alpha]; wt_blk's fields r, g, b, a hold the bytes in file order, so its r is our
        // blue. Counted apart: this is a difference in meaning, not in data
        (Value::Color([r, g, bl, al]), BlkType::Color { r: r2, g: g2, b: b2, a: a2 }) => {
            if (r, g, bl, al) == (b2, g2, r2, a2) {
                if r != bl {
                    SWAPPED_COLORS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                true
            } else {
                false
            }
        }
        (Value::Matrix(x), BlkType::Float12(y)) => x.iter().flatten().zip(y.iter()).all(|(p, q)| f(*p, *q)),
        (Value::Int64(x), BlkType::Long(y)) => x == y,
        _ => false,
    }
}
