//! Compares wt-formats with wt_blk on a War Thunder install.
//!
//! wt_blk is the black box here: every file is parsed by both, and what comes out must be the same,
//! down to the bits of every float. Differences are reported with the file and the field.
//!
//! ```text
//! oracle grp   <game dir> [--threads N]    every .grp: pack tables, resource bytes, DynModels, skeletons
//! oracle dxp   <game dir> [--threads N]    every .dxp.bin: entries, headers, unpacked data, RGBA
//! oracle fuzz  [<game dir>] [--iterations N] [--seed N]   mutated and random inputs: same verdict, same output
//! ```

mod common;
mod dxp;
mod fuzz;
mod grp;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().cloned() else {
        eprintln!("usage: oracle grp|dxp <game dir> [--threads N] | oracle fuzz [<game dir>] [--iterations N]");
        std::process::exit(2);
    };
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let threads: usize = flag("--threads").and_then(|v| v.parse().ok()).unwrap_or(3);
    let limit: Option<usize> = flag("--limit").and_then(|v| v.parse().ok());
    let game = args.get(1).filter(|a| !a.starts_with("--")).map(PathBuf::from);
    let tally = match cmd.as_str() {
        "grp" => grp::run(&game.expect("game dir"), threads, limit),
        "dxp" => dxp::run(&game.expect("game dir"), threads, limit, flag("--match")),
        "fuzz" => fuzz::run(
            flag("--iterations").and_then(|v| v.parse().ok()).unwrap_or(200_000),
            flag("--seed").and_then(|v| v.parse().ok()).unwrap_or(1),
            game,
        ),
        other => {
            eprintln!("unknown command {other}");
            std::process::exit(2);
        }
    };
    tally.print();
    common::print_peak_rss();
    if tally.total_diffs > 0 {
        std::process::exit(1);
    }
}
