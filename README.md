# wt-formats

Readers for the Dagor Engine resource formats of [War Thunder](https://warthunder.com), written in Rust:

- **GRP packs** (`*.grp`): the game's resources, found by name and class.
- **DynModels and skeletons** inside them: the models of vehicles (LODs, rigid parts, skinned tracks, vertex
  buffers with their packed index buffers) and the node trees that place their parts.
- **DXP packs** (`*.dxp.bin`) and **DDSx textures** inside them: stored, zstd or Oodle packed, in DDS mip order;
  with the `png` feature also the top mip as 8-bit RGBA or PNG.

It reads bytes you give it. It does not touch the game, ship any game data, or write anything. It does not panic
on malformed input, and what it allocates is bounded as described under [Limits](#limits).

```rust
use wt_formats::{dynmodel::DynModel, geom_node_tree::GeomNodeTree, grp::{class_id, GrpPack}};

let file = std::fs::read("usa_m4a1_76w_sherman.grp")?;
let pack = GrpPack::parse(&file)?;
let model = pack.find("m4a1_76w_sherman", class_id::DYN_MODEL).expect("a model");
let model = DynModel::parse(pack.data(model))?;
let skeleton = pack.find("m4a1_76w_sherman_skeleton", class_id::GEOM_NODE_TREE).expect("a skeleton");
let skeleton = GeomNodeTree::parse(pack.data(skeleton))?;
println!("{} LODs, {} nodes", model.lods.len(), skeleton.nodes.len());
```

The layouts are in [docs/FORMATS.md](docs/FORMATS.md), as facts (offsets, types, constants) with the bytes of the
game files that show each.

This is the reader that [wt-unpack](https://github.com/patodois/wt-unpack) uses. It is not a reader of vromfs
archives or BLK files; wt-unpack has its own.

## Credit and provenance

The formats are Dagor Engine's, and the layouts come from its sources (BSD-3-Clause,
[GaijinEntertainment/DagorEngine](https://github.com/GaijinEntertainment/DagorEngine)): `grpData.h`, `texPackMgr2.cpp`,
`ddsxTex.h`, `ddsxTexMipOrder.h`, `dynSceneRes.cpp`, `shSkinnedMesh.cpp`, `geomTree.cpp` and `matVdataLoad.cpp`.

The lineage of this code is: those sources, then this project's own readers in its pull requests #28 and #29 to
[wt_blk](https://github.com/Warthunder-Open-Source-Foundation/wt_blk) (not merged there; the fork is
`patodois/wt_blk`), then this crate, which was written after them and checked against the game's files. No line of
wt_blk's upstream author is used: upstream's `dxp_and_grp` holds 204 lines (a reader of the names in a DXP or GRP
file, an error type and a dispatcher), and the readers of this crate share at most 1.3% of their 8-token sequences
with them (3.0% of 6-token ones), which is imports and `Ok(...)`. The vromfs and BLK readers of wt_blk are not used.

**It is not a clean-room rewrite.** The author of this code had read the readers of the pull requests, and the result
is similar to them. Measured on the code without comments or tests, as the share of 12-token sequences of each file
that also occur in the pull requests' `src/dxp_and_grp/`: `ddsx.rs` 35%, `dynmodel.rs` 33%, `geom_node_tree.rs` 31%,
`unpack.rs` 24%, `grp.rs` 22%, `dxp.rs` 22%, `reader.rs` 21%, `block.rs` 13%, `meshopt.rs` 10%, `error.rs` 1%
(counting 8-token sequences, 20% to 47%). That is this project's own code, so nothing third-party is in those files; they
say so in a header comment. Test scenarios for hostile counts (a count that would size an allocation, a name copied many
times) follow the ones in those pull requests.

wt_blk states its license twice and the two differ: its `Cargo.toml` says MIT, its `LICENSE` file and README say
Apache-2.0 (copyright FlareFlo). Neither applies to code its author wrote, and both can be combined with this crate's
MIT OR Apache-2.0.

If you need a rewrite with no derivation at all, give `docs/FORMATS.md` and no other source to someone who has not
read those readers: the oracle will say whether the result agrees.

meshoptimizer's index codec (MIT, Arseny Kapoulkine) is the format of `src/meshopt.rs`.

## How it was checked

- **Unit tests** on synthetic inputs (`cargo test --features png`): every layout is built by hand, including the
  failure cases (61 tests with the `png` feature, 55 without).
- **Hostile input tests** (`src/hostile.rs`): well-formed files with bytes changed, cut and replaced by extreme
  values, 20,000 per format by default (`WT_FORMATS_FUZZ_ITERATIONS` for more; 1,000,000 each were run, and with
  `WT_GAME_DIR` set also mutations of real tank models and skeletons). Parsing must not panic and no single
  allocation may exceed `16 * input + 4 MiB`.
- **The oracle** (`oracle/`, not part of the crate): parses every `.grp` and `.dxp.bin` of an install with this
  crate and with the pull requests' readers in the `patodois/wt_blk` fork, and compares everything, down to the bits of
  every float. This is a regression check against the project's own earlier code (the same decoder crates are
  used underneath), not an independent validation. On War Thunder 2.59.0.80: 3,263 GRP
  files, 107,056 resources, 16,934 models, 18,770 skeletons and 1.87 billion vertices; 4,318 DXP packs and 147,960
  textures with their RGBA, 109 GB unpacked; **no difference**. Plus fuzzing against wt_blk and a check of wt-unpack's
  own vromfs and BLK readers. Details, and the few differences the fuzzing found with their causes, in
  [docs/ORACLE.md](docs/ORACLE.md).

```sh
cd oracle
cargo run --release --bin wt-formats-oracle -- grp "$GAME"        # packs, models, skeletons
cargo run --release --bin wt-formats-oracle -- dxp "$GAME"        # texture packs, textures, RGBA
cargo run --release --bin wt-formats-oracle -- fuzz "$GAME"       # mutated inputs: same verdict as wt_blk
```

## Limits

- A count or size read from a file is checked against the bytes behind it before it sizes anything.
- Packed data is unpacked as it comes. Oodle output grows only while the stream keeps producing it, zstd
  and zlib are read as they come, and a block never unpacks to more than `MAX_UNPACKED` (256 MiB; the largest
  texture in the game is 85 MiB, the largest model's vertex data 31 MiB) whatever its header claims. A skeleton's
  dump is bounded by its own size field (at most 1 MiB), so a packed skeleton can never unpack to more.
- A small zstd or Oodle stream can still describe a large, repetitive texture or vertex buffer: for those the bound is
  the cap, and buffers may reach twice it while they grow.
- A skinned mesh's palettes are copied per material, and the copies together may not take more bytes than the dump they
  come from. No model of War Thunder 2.59.0.80 has a palette slot at all (30,690 skinned materials, 0 slots).
- `Texture::to_rgba` and `to_png` refuse more than `MAX_RGBA_PIXELS` (8192 by 8192, the largest texture of the game) and
  take 4 bytes a pixel.

Dependencies are pure Rust: the Oodle-family decoder is
[`lztans`](https://github.com/patodois/lztans) (clean-room, MIT OR Apache-2.0), the block decoders [`bcn-decode`](https://github.com/patodois/bcn-decode) (written from the public specifications)
and zstd is [`ruzstd`](https://crates.io/crates/ruzstd) (only the first frame is read, and a frame whose window is larger
than `max(8 MiB, twice the data it may hold)` is refused before the decoder allocates it). Both say they do not panic on
malformed input (and the tests here check it), so nothing depends on `panic = "unwind"`.

## Features

- `png` (off by default): `Texture::to_rgba` and `Texture::to_png`, with BC1 to BC5 and BC7 block decoders from
  `bcn-decode` and the `png` encoder.

## License

MIT OR Apache-2.0, at your option; see `LICENSE-MIT` and `LICENSE-APACHE`.
