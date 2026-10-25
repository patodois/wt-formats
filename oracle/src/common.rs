//! Shared helpers: file discovery, tallies, hashing.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub fn walk(dir: &Path, suffix: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ty) = e.file_type() else { continue };
        let p = e.path();
        if ty.is_dir() {
            walk(&p, suffix, out);
        } else if ty.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(suffix)) {
            out.push(p);
        }
    }
}

pub fn map(path: &Path) -> std::io::Result<memmap2::Mmap> {
    let f = File::open(path)?;
    // The oracle only reads; a game update while it runs would be the user's doing
    unsafe { memmap2::Mmap::map(&f) }
}

pub fn hash(bytes: &[u8]) -> u128 {
    xxhash_rust::xxh3::xxh3_128(bytes)
}

/// What a run found: counts by name, and the differences between the two libraries.
#[derive(Default)]
pub struct Tally {
    pub counts: BTreeMap<String, u64>,
    pub diffs: Vec<String>,
    pub total_diffs: u64,
    pub samples: BTreeMap<String, Vec<String>>,
}

impl Tally {
    pub fn count(&mut self, key: impl Into<String>) {
        self.add(key, 1);
    }
    pub fn add(&mut self, key: impl Into<String>, n: u64) {
        *self.counts.entry(key.into()).or_default() += n;
    }
    pub fn max(&mut self, key: impl Into<String>, v: u64) {
        let e = self.counts.entry(key.into()).or_default();
        *e = (*e).max(v);
    }
    /// Keeps the first few examples of something that is only counted otherwise.
    pub fn sample(&mut self, kind: &str, detail: String) {
        let v = self.samples.entry(kind.to_owned()).or_default();
        if v.len() < 6 {
            v.push(detail);
        }
    }
    /// Records a difference. Only the first few of each kind are kept in full.
    pub fn diff(&mut self, kind: &str, detail: String) {
        self.total_diffs += 1;
        let n = self.counts.entry(format!("DIFF {kind}")).or_default();
        *n += 1;
        if *n <= 8 {
            self.diffs.push(format!("{kind}: {detail}"));
        }
    }
    pub fn merge(&mut self, other: Tally) {
        for (k, v) in other.counts {
            if k.starts_with("max ") {
                let e = self.counts.entry(k).or_default();
                *e = (*e).max(v);
            } else {
                *self.counts.entry(k).or_default() += v;
            }
        }
        self.total_diffs += other.total_diffs;
        for (k, v) in other.samples {
            let e = self.samples.entry(k).or_default();
            for d in v {
                if e.len() < 6 {
                    e.push(d);
                }
            }
        }
        for d in other.diffs {
            if self.diffs.len() < 400 {
                self.diffs.push(d);
            }
        }
    }
    pub fn print(&self) {
        println!("---- counts");
        for (k, v) in &self.counts {
            println!("{k}: {v}");
        }
        println!("---- examples");
        for (k, v) in &self.samples {
            for d in v {
                println!("{k}: {d}");
            }
        }
        println!("---- differences: {}", self.total_diffs);
        for d in &self.diffs {
            println!("{d}");
        }
    }
}

/// Runs `work` on every item with `threads` workers, largest items first as given.
pub fn parallel<T: Sync>(items: &[T], threads: usize, work: impl Fn(usize, &T, &mut Tally) + Sync) -> Tally {
    let next = AtomicUsize::new(0);
    let total = Mutex::new(Tally::default());
    let done = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| {
                let mut local = Tally::default();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    work(i, item, &mut local);
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if d % 250 == 0 {
                        eprintln!("  {d}/{} files", items.len());
                    }
                    if local.total_diffs > 0 || d % 50 == 0 {
                        total.lock().unwrap().merge(std::mem::take(&mut local));
                    }
                }
                total.lock().unwrap().merge(local);
            });
        }
    });
    total.into_inner().unwrap()
}

/// The variant name of an error: `Debug` text up to its first `(`, `{` or space.
pub fn kind(e: &impl std::fmt::Debug) -> String {
    let s = format!("{e:?}");
    s.split(|c| c == '(' || c == '{' || c == ' ').next().unwrap_or("").to_owned()
}

pub fn print_peak_rss() {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    // macOS reports bytes, Linux kilobytes
    let bytes = if cfg!(target_os = "macos") { ru.ru_maxrss as u64 } else { ru.ru_maxrss as u64 * 1024 };
    println!("peak RSS: {:.2} GB", bytes as f64 / 1e9);
}
