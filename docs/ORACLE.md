# Agreement with wt_blk

`oracle/` is a separate program (not part of the crate) that parses the same bytes with this crate and with the
fork `patodois/wt_blk` at `498c996`, which wt-unpack used before, and compares the results. The readers it
compares with are this project's own earlier code (its pull requests #28 and #29 to wt_blk), and the decoder crates
underneath are the same, so this is a regression check, not an independent validation of the layouts: that is what
the surveys of the game's bytes in [FORMATS.md](FORMATS.md) are for. The fork is a black box here: nothing it does
is assumed right. Run on War Thunder 2.59.0.80 (macOS), 3 worker threads, peak resident
memory 0.92 GB (GRP) and 2.65 GB (DXP, mostly memory-mapped file pages).

## GRP packs: `oracle grp`

3,263 files (13.0 GB), 107,056 resources.

| Compared | Result |
|---|---|
| Pack tables: resource count, name, class id | identical in 3,263 of 3,263 |
| Bytes of each resource | identical for all 107,056 (13.0 GB) |
| DynModels parsed by both | 16,934: identical |
| DynModels refused by both | 164 (157 with a lods dump too short for a position scale, 7 with their own materials) |
| Vertex buffers, indices, channels | identical (5.35 billion index entries) |
| Vertices decoded (position, normal, both UV sets; the bone slots and weights of skins), bit for bit | 1,868,138,620, identical |
| LODs / rigid parts / elements | 42,493 / 1,585,066 / 2,173,007, identical |
| Skins / bones / skin elements | 28,530 / 870,144 / 30,752, identical, with palettes and `Skin::bone` lookups |
| Skeletons parsed by both | 18,770 (2,530,853 nodes): names, parents and both matrices identical to the bit |

Differences: **0**.

## DXP packs: `oracle dxp`

4,318 files (55.9 GB), 176,930 entries.

| Compared | Result |
|---|---|
| Pack tables, names, every header field | identical in 4,318 of 4,318 |
| Textures unpacked by both | 147,960 (109.0 GB unpacked): the data of each, hashed (xxh3, 128 bits), identical |
| Same, converted to RGBA | 147,960 of 147,960: identical |
| Refused by both | 28,970: 28,595 label-only entries and 375 stored as differences to a paired base texture |

Differences: **0**. The statistics of the headers (formats, flags, sizes) are in the output of `oracle dxp` and
summarised in [FORMATS.md](FORMATS.md#what-the-install-holds). The RGBA conversion uses the same block decoder
crate in both libraries (`texture2ddecoder`), so that check covers the code around it (sizes, swizzles, the
formats that are not blocks), not the decoder.

## Mutated and random inputs: `oracle fuzz`

400,000 mutations of real resources (models and skeletons of two tanks, 60 small GRP packs, 60 small DXP packs;
bytes changed, words set to 0, 1, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF..., cut, copied over each other), 400,000
random or mutated packed index buffers, and 200,000 synthetic DDSx headers (random format, flags, sizes, stored,
zstd or Oodle data). Neither library panicked outside the Oodle decoder's own panics (below).

Results: 153,301 packs, 28,799 models, 61,505 skeletons and 74,125 index buffers parsed by both with identical
output; the rest refused by both. Unexplained differences: **0**. Explained ones, counted apart:

| Difference | Count | Cause, and who is right |
|---|---|---|
| DXP pack with a name that is not UTF-8 | 7,186 | wt_blk refuses the whole pack, this crate replaces the bad bytes (U+FFFD). Design choice; no file of the game has such a name. |
| Compression bits 5 to 7 of the DDSx flags | 10,841 | No packer is known for them. wt_blk reads the data as stored (and then, most of the time, fails the size checks); this crate refuses. |
| 64-bit name or parent pointer with bits above 32 set | 3,476 | wt_blk reads the low 32 bits; this crate reads 64 and finds the offset out of range. A pointer with those bits set is wrong in either reading. |
| Oodle stream of 3 to 5 bytes (a stored block of 1 to 3 bytes) | 13 | **This crate is right.** wt_blk refuses it because it assumes 6 bytes of stream per 256 KiB block; the stream is valid and the decoder unpacks it. No texture of the game is that small. |

In the model sweep one more difference of design is not exercised by fuzzing: this crate refuses a skinned-mesh
section with other than one variant or with a gap (see [FORMATS.md](FORMATS.md#not-seen-in-the-data)).

## Extra: wt-unpack's own vromfs and BLK readers: `oracle-vromfs`

wt-unpack does **not** use wt_blk's vromfs or BLK code: it has its own (`src/vromfs.rs`, `src/blk.rs`), written from
the Dagor sources. They are not in this crate. Here wt_blk's vromfs and BLK code is upstream's own, so this
comparison is between two independent readers. As the same method applies, `oracle-vromfs/` (a separate program that
depends on the wt-unpack checkout next to this repository) compares them with wt_blk on every vromfs of the install.

| Compared | Result |
|---|---|
| Archives | 29 files (8 of them byte copies in `cache/binary.2.59.0/`); 0.92 GB |
| Files in them | 101,291 byte-identical (1.14 GB unpacked), same names (wt_blk calls the shared name map `nm`; wt-unpack keeps its bytes `ff 3f 6e 6d`) |
| Binary BLKs parsed by both | 63,313 (42,305,696 values): same names, types, values and order |

Differences: **0** in data. One difference of meaning: for the `c` (colour) type, wt-unpack returns `[red, green,
blue, alpha]` from the file's bytes `blue, green, red, alpha`, while wt_blk's fields `r, g, b, a` hold the bytes in file
order, so its `r` is wt-unpack's blue. 24,512 colour values have different red and blue. wt-unpack's reading is the
Direct3D one (and `waterLineColor` of `damagemodelhits.blk` comes out teal, `jetLightColor` of `gameparams.blk` orange);
this was not checked against Dagor's source. Separately, wt-unpack's `md5_ok` is false for 6 of the 29 archives (not
investigated; wt_blk's own validation passed for all 29).

## Reproducing

```sh
GAME="/Applications/WarThunderLauncher.app/Contents/WarThunder.app/Contents/Resources/game"
cd oracle && cargo build --release
./target/release/wt-formats-oracle grp "$GAME" --threads 3
./target/release/wt-formats-oracle dxp "$GAME" --threads 3        # 20 minutes
./target/release/wt-formats-oracle fuzz "$GAME" --iterations 400000
cd ../oracle-vromfs && cargo build --release && ./target/release/wt-unpack-vromfs-oracle "$GAME"
```

`oracle/survey/*.py` read the install's bytes with the Python standard library only; `oracle/src/bin/evidence.rs`
prints the bytes quoted in [FORMATS.md](FORMATS.md) for a model.
