# Cloud report: coverage-guided fuzzing of `wt-formats`

Branch: `claude/fuzz-wt-formats-ypi5ta` (the task text names `cloud/fuzz-wt-formats`, but the session was pinned to the
branch above and told not to push anywhere else, so that is the branch used and the only one pushed).

## What was done

- Added `fuzz/` (cargo-fuzz / libFuzzer, nightly only). It has its own `[workspace]`, is listed in the root `exclude`
  lists (workspace and package), and is therefore outside the stable build, CI and the published crate.
  `cargo package --list` shows no `fuzz/` file.
- One target per public entry point that takes untrusted bytes (all built with the `png` feature so `to_rgba`/`to_png` are covered):

  | target | exercises |
  |---|---|
  | `grp` | `GrpPack::parse`, `data`, `find` |
  | `dxp` | `DxpPack::parse`, `texture(i)` for every entry (+ an out-of-range index), `to_rgba`, `to_png` |
  | `ddsx` | `DdsxHeader::parse` and its accessors, `Texture::from_ddsx` on the bytes after the header, `to_rgba`, `to_png` |
  | `dynmodel` | `DynModel::parse`, `vertex`, `skin_weights`, `Skin::bone`, `VData` accessors |
  | `geom_node_tree` | `GeomNodeTree::parse`, `find` |
  | `meshopt` | `decode_index_sequence` (first two input bytes choose count and width; also called with `usize::MAX`) |

  The internal unpackers (`unpack::{oodle,zstd,zlib}`, `block`) are reached through `ddsx`, `dxp`, `dynmodel`, `geom_node_tree`.
  They are not public, so they have no target of their own.
- Seed corpora in `fuzz/corpus/<target>/`: small synthetic inputs written by a new `#[ignore]`d test,
  `hostile::write_fuzz_seeds`, using the existing test builders (`WT_FORMATS_SEED_DIR=<dir> cargo test --release --all-features write_fuzz_seeds -- --ignored`).
  No game file or game data was used. The committed corpus was then replaced by the libFuzzer-merged, coverage-minimal set
  of everything the runs found (about 1.3 MB of files).
- Flags for all runs: `-timeout=10 -rss_limit_mb=2048 -malloc_limit_mb=600` (a legitimate block may unpack to
  `MAX_UNPACKED` = 256 MiB, so 600 MB flags only a clearly unbounded allocation).

## Bugs found

**None.** No panic, arithmetic overflow, out-of-memory, excessive allocation (over the 600 MB single-malloc limit) or hang
(over 10 s) was found by any target. Because of that, no library code was changed and no regression test was added
(there was nothing for one to fail on). The crate's existing `src/hostile.rs` tests, bounded readers (`Reader`, `check_table`,
`NameBudget`), size caps in `unpack.rs` and the checked arithmetic in `ddsx.rs`/`dynmodel.rs` held up. Overflow checks were
active: cargo-fuzz builds with `debug-assertions` and overflow checks on by default.

## Fuzzing time per target (wall clock, one process each, 4-core machine)

| target | time | executions | final coverage (edges / features) |
|---|---|---|---|
| grp | 33.4 min | 184.8 M | 223 / 683 |
| dxp | 33.4 min | 13.8 M | 1904 / 7372 |
| ddsx | 33.4 min + 33.4 min (`-max_len=16384`) | 5.95 M + 4.75 M | 3069 / 10262 |
| dynmodel | 33.4 min + 33.4 min (`-max_len=16384`) | 9.65 M + 11.5 M | 4336 / 11935 |
| geom_node_tree | 33.4 min | 40.1 M | 2865 / 7787 |
| meshopt | 33.4 min | 279.3 M | tiny target, saturated |

(Default `-max_len` was 4096 for the first runs.) Coverage was still creeping up in `dynmodel` and `ddsx` at the end,
so deeper bugs in those two are not excluded; I stopped at the planned budget because nothing had been found.

## Left out, and why

- No structure-aware (grammar / `arbitrary`) mutators for the DynModel or DDSx layouts. Plain byte mutation plus synthetic
  seeds was used. A structured harness might reach deeper in `dynmodel` (skins, packed vertex data).
- Oodle decoding is done by the `lztans` dependency; ZSTD by `ruzstd`; zlib by `flate2`. They are fuzzed only as far as this crate's
  entry points reach them. Their own bugs, if any, are outside this repository.
- No real game data was run (rule), so behaviour on real files was not checked beyond what the repository's own tests do.
- No CI job was added for fuzzing (task said to keep it out of CI).

## Uncertain / notes

- "No crash found" is evidence, not proof, particularly for `dynmodel` and `ddsx` (see above).
- Only the 64-bit target was fuzzed; the 32-bit `usize` paths (casts such as `header.mem_size as usize`) were reviewed by reading only.
- Fuzzing ran with `rss_limit_mb=2048`; reported RSS peaked at about 560 MB (mostly sanitizer and corpus overhead), below the limit.
