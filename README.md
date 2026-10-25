# wt-formats

Readers for the Dagor Engine resource formats of [War Thunder](https://warthunder.com), written in Rust:

- **GRP packs** (`*.grp`): the game's resources, found by name and class.
- **DynModels and skeletons** inside them: the models of vehicles (LODs, rigid parts, skinned tracks, vertex
  buffers with their packed index buffers) and the node trees that place their parts.
- **DXP packs** (`*.dxp.bin`) and **DDSx textures** inside them: stored, zstd or Oodle packed, in DDS mip order;
  with the `png` feature also the top mip as 8-bit RGBA or PNG.

It reads bytes you give it. It does not touch the game, ship any game data, or write anything. It does not panic
on malformed input, and what it allocates follows the input (see [Limits](#limits)).

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

The layouts in this crate were documented first by
[**wt_blk**](https://github.com/Warthunder-Open-Source-Foundation/wt_blk) (Warthunder-Open-Source-Foundation), which this
crate does not depend on. wt_blk's source was read to learn the formats, and its comments name the Dagor Engine files
they come from. The layouts were then checked against the game's files and written down in
[docs/FORMATS.md](docs/FORMATS.md), and the code was written from those notes.

**It is not a clean-room rewrite.** The author of this code had read wt_blk's readers, and the result is similar to
them. Measured on the code without comments or tests, as the share of 12-token sequences of each file that also occur
in wt_blk's `src/dxp_and_grp/`: `grp.rs` 38%, `ddsx.rs` 37%, `dynmodel.rs` 37%, `geom_node_tree.rs` 31%,
`reader.rs` 21%, `unpack.rs` 24%, `dxp.rs` 22%, `block.rs` 16%, `meshopt.rs` 10%, `error.rs` 1%. (Counting 8-token
sequences gives 20% to 50%.) Against the 204 lines of `dxp_and_grp` that were written by wt_blk's upstream author
rather than by this project, the overlap is under 1% of 8-token sequences in every file.

That is because the similar code is this project's own: wt_blk's `ddsx`, `dynmodel`, `geom_node_tree`, `grp_pack`,
`meshopt`, `unpack` and `cursor` modules and the `DxpPack` part of `dxp` were written by this project's author
(`patodois`; pull requests #28 and #29 of wt_blk, 22 commits in the `patodois/wt_blk` fork), not by wt_blk's
maintainers. The files that are substantially similar say so in a header comment. Hence no third-party code
is in them. A note on wt_blk's license, because the two places that state it disagree: its `Cargo.toml` says MIT, its
`LICENSE` file and README say Apache-2.0 (copyright FlareFlo); both can be combined with this crate's
MIT OR Apache-2.0, and neither applies to code its author wrote.

Test scenarios for hostile counts (a count that would size an allocation, a name copied many times) follow those in
wt_blk's tests. If you need a rewrite with no derivation at all, give `docs/FORMATS.md` and no other source to
someone who has not read wt_blk: the oracle will say whether the result agrees.

meshoptimizer's index codec (MIT, Arseny Kapoulkine) is the format of `src/meshopt.rs`.

## How it was checked

- **Unit tests** on synthetic inputs (`cargo test --features png`): every layout is built by hand, including the
  failure cases (55 tests).
- **Hostile input tests** (`src/hostile.rs`): well-formed files with bytes changed, cut and replaced by extreme
  values, 20,000 per format by default (`WT_FORMATS_FUZZ_ITERATIONS` for more; 1,000,000 each were run, and with
  `WT_GAME_DIR` set also mutations of real tank models and skeletons). Parsing must not panic and no single
  allocation may exceed `16 * input + 4 MiB`.
- **The oracle** (`oracle/`, not part of the crate): parses every `.grp` and `.dxp.bin` of an install with this
  crate and with wt_blk and compares everything, down to the bits of every float. On War Thunder 2.59.0.80: 3,263 GRP
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

A block never unpacks to more than `MAX_UNPACKED` (1 GiB; the largest in the game is 85 MiB) bytes, whatever its header claims. Inside that cap
memory follows the data: Oodle output grows only while the stream keeps producing it, zstd and zlib are read
as they come, and a count read from a file is checked against the bytes behind it before it sizes anything.
A small zstd or Oodle stream can still describe a large, repetitive texture; the cap is the bound.

The Oodle decoder (`oozextract`) panics on some malformed streams; the panic is caught and returned as an error.
That needs the default `panic = "unwind"`.

## Features

- `png` (off by default): `Texture::to_rgba` and `Texture::to_png`, with BC1 to BC5 and BC7 block decoders from
  `texture2ddecoder` and the `png` encoder.

## License

MIT OR Apache-2.0, at your option; see `LICENSE-MIT` and `LICENSE-APACHE`.
