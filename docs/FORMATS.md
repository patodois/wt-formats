# Formats

The layouts this crate reads, written down as facts: offsets, types, constants, and the evidence for each. The
evidence is a file of a War Thunder 2.59.0.80 install (macOS) and the bytes in it. "All N files" means a
survey of every `.grp` or `.dxp.bin` in the install (3,263 GRP files, 13.0 GB; 4,318 DXP files, 55.9 GB),
made by a script that reads the bytes directly (no library); the scripts are in
[`oracle/survey/`](../oracle/survey).

## Where this comes from

These formats were documented first by
[wt_blk](https://github.com/Warthunder-Open-Source-Foundation/wt_blk) (MIT), whose source was read to learn
the layouts, and by the comments in it that name the Dagor Engine source files the layouts come from
(`grpData.h`, `texPackMgr2.cpp`, `ddsxTex.h`, `dynSceneRes.cpp`, `shSkinnedMesh.cpp`, `geomTree.cpp`,
`matVdataLoad.cpp`; the engine is open source at
[GaijinEntertainment/DagorEngine](https://github.com/GaijinEntertainment/DagorEngine), and those files were
not read for this crate). Everything below was then checked against the game's files, and the code of this
crate was written from these notes, not from wt_blk's code. The crate is compared with wt_blk on the whole
install; see [the README](../README.md#how-it-was-checked).

What the data does not show, this document does not claim. Where a branch of the code is not exercised by any
file of the install, it says so in "Not seen in the data" at the end.

## Conventions

- All integers are little-endian, floats are IEEE 754 binary32.
- A **table** is 16 bytes: `u32 offset`, `u32 count`, 8 bytes that are 0. The entries are `count` records at
  `offset`, counted from the start of the dump the table is in. (This crate does not look at the offset of an
  empty table.)
- A **dump** is a block of memory the game's tools wrote and the game loads in one piece, with the pointers
  turned into offsets from its start. Such an offset, or a size, is stored in a 64-bit field in some places and in
  a 32-bit one in others; each layout says which.
- A **block** (in the model streams) is a `u32` followed by that many bytes. The low 30 bits of the `u32` are
  the length; the top two bits are a tag. In all 16,663 packed vertex blocks of the install the tag is 2 and the
  contents are `u32 unpacked size` followed by an Oodle stream (its first byte is `0x8C`). A zstd frame
  inside a block is recognised by its magic (`28 B5 2F FD`); none occurs in the install.

## GRP packs (`*.grp`)

A GRP pack is a list of named resources of different classes (models, skeletons, collision, placed objects).

| Offset | Type | Field |
|---|---|---|
| 0x00 | 4 bytes | `GRP2` (all 3,263 files; `GRP3` is accepted but occurs in none) |
| 0x04 | u32 | not read |
| 0x08 | u32 | not read |
| 0x0C | u32 | file size minus 16 (true in all 3,263 files; not read) |
| 0x10 | table | names: `count` records of `i32` offsets from the start of the file to NUL-terminated names; a negative offset means "no name" |
| 0x20 | table | resources: 12-byte records `u32 class id`, `u32 file offset`, `u16 index into the names`, `u16` (0 in every record) |
| 0x30 | table | dependencies, one per resource (the count equals the resource count in all 3,263 files; not read) |
| 0x40 | | the names, NUL-terminated, one after the other |

The data of a resource runs from its offset to the offset of the next resource in the table, and the last one
to the end of the file. In all 3,263 files the offsets ascend and are multiples of 16.

Evidence, `content/base/res/avn_north_sea.grp` (1,923 bytes):

```
00000000: 47 52 50 32  e0 02 00 00  08 03 00 00  73 07 00 00   GRP2 .. .. 0x773 = 1923 - 16
00000010: 50 02 00 00  16 00 00 00  00 00 00 00  00 00 00 00   names: offset 0x250, 22 of them
00000020: a8 02 00 00  02 00 00 00  00 00 00 00  00 00 00 00   resources: offset 0x2a8, 2 of them
00000030: c0 02 00 00  02 00 00 00  00 00 00 00  00 00 00 00   dependencies: offset 0x2c0, 2
00000040: 61 76 6e 5f 6e 6f 72 74 68 5f ...                    "avn_north_sea_detailed", ...
000002a8: c4 59 fb 03  20 03 00 00  00 00 00 00                class 0x03FB59C4, offset 0x320, name 0
000002b4: c4 59 fb 03  30 05 00 00  01 00 00 00                class 0x03FB59C4, offset 0x530, name 1
```

Resource 0 is bytes 0x320 to 0x530, resource 1 is 0x530 to the end of the file.

Classes in the install (107,056 resources): `B4B7D9C4` DynModel 17,098; `56F81B6D` GeomNodeTree 18,770; `77F8232F`
placed-object model (RendInst) 20,582; `ACE50000` collision 22,994; `A6F87A9B` 5,105; `D543E771` 4,517;
`8F2A701A` 3,138; `88B7A117` 3,127; `855A1BE6` 2,935; `73A10E01` 2,786; `40C586F9` 3,021; `05DFF175` 1,192;
`03FB59C4` land class 1,157; `39B5B09E` 497; `CD5B9736` 75; `A7D3ED6A` 60; `2AD457F0` 2. This crate reads the first
two; the rest are for the callers (wt-unpack reads RendInst, collision and land classes itself).

## GeomNodeTree (class `56F81B6D`): skeletons

| Offset | Type | Field |
|---|---|---|
| 0x00 | u32 | size of the node dump; the low 20 bits |
| 0x04 | u32 | node count in the low 31 bits; bit 31 says the dump is in a packed block (never set in the install: all 18,770 are stored) |
| 0x08 | | the dump: for a packed tree a block (see Conventions) holding it |

Each node of the dump is 160 bytes, in this order:

| Offset | Type | Field |
|---|---|---|
| 0 | 16 f32 | `tm`, the transform relative to the parent, a 4x4 matrix in column-major order |
| 64 | 16 f32 | `wtm`, the transform in model space, column-major |
| 128 | table-like, 16 bytes | the children (not read) |
| 144 | u64 | the parent: the offset of the parent's node in the dump (node index times 160); node 0 holds `0xFFFFFFFF` |
| 152 | u64 | the offset of the node's name in the dump |

The names follow the nodes. The nodes are ordered parents first: in all 18,770 skeletons every node's parent
offset is a multiple of 160 and names an earlier node (this crate refuses a tree where it does not, so that
walking up to the root ends).

Evidence, `content/base/res/tanks/usa_m4a1_76w_sherman.grp`, resource `m4a1_76w_sherman_skeleton` (file offsets
0x1c72a0 to 0x1ce800):

```
001c72a0: 4c 75 00 00  aa 00 00 00  | 00 00 80 3f 00 00 00 00 ...    dump size 0x754c, 170 nodes, not packed
node 0 at +0x08: parent 0x00000000FFFFFFFF, name at 27200 (= 170 * 160), which reads as an empty string
node 1 at +0xa8: parent 0, name at 27201 -> "root"
```

## DynModel (class `B4B7D9C4`): vehicle models

A model is a stream of consecutive parts:

```
5 x u32      lods dump size; texture count; material count; vertex buffer count; vertex word
             texture and material counts are 0xFFFFFFFF in all 17,091 models this crate reads: the materials
             are not in the model but in the vehicle's dynModelDesc.bin
vertex data  see below; its length follows from the vertex word
lods dump    "lods dump size" bytes
names        u32 size, then the node name map dump
for each LOD (as many as the lods dump lists):
  scene dump (its size is in the lods dump)
  for each rigid part of the scene: its mesh dump
skinned meshes   only if some LOD says it has skins
```

Models whose texture count is not `0xFFFFFFFF` (7 in the install, with 0 textures and 0 materials: `sepr_844_rocket`,
`dummy_airfield_1900x400m_a`, `dummy_for_missions_b_500x500m`, `dummy_for_missions_b_500x500m_a` in `aces.grp`,
`dynBase_s75_mesh`, `wave_a` and `uk_navy_officer_beret_01b_model`) carry their own materials and are not read.

### Vertex data

The vertex word's top three bits say how it is stored:

- `110` packed (16,663 models): header and buffers together are one block. The low 29 bits of the word are
  the size of the header inside it. Evidence, `m4a1_76w_sherman_xray` (file offset 0x367680): the five words are
  `0x58, 0xFFFFFFFF, 0xFFFFFFFF, 1, 0xC0000058`; the block head at +20 is `0x80016c8d` (tag 2, 93,325 bytes);
  the block starts `4b f9 06 00 8c 0c ...`: unpacked size 0x6f94b = 457,035, then the Oodle stream.
- `000` stored (428 models): the low 30 bits of the word are the size of the header, which follows the five
  words; the buffers follow it and their total size is worked out from the header.
- Anything else (zlib or lzma) is not used by any model.

The unpacked vertex data is a header of the size the vertex word gave, then each buffer's vertices and indices:

| Header offset | Content |
|---|---|
| 0x00 | 16 bytes, a material table, empty in every model |
| 0x10 | table of buffers: 32-byte records |
| (any) | declarations: 12-byte records |

A buffer record:

| Offset | Type | Field |
|---|---|---|
| 0x00 | u32 | vertex count |
| 0x04 | u32 | `w1`: the low 8 bits are the stride in bytes; for packed indices the upper 24 bits are the low 24 bits of the packed index size |
| 0x08 | u32 | `w2`: the low 28 bits are the size of the indices (unpacked, for packed ones); for packed indices the top 4 bits are bits 24 to 27 of the packed size |
| 0x0C | u32 | flags: `0x20` 16-bit indices, `0x40` 32-bit indices, `0x200` the indices are packed (see below) |
| 0x10 | u32, u32 | offset and count of the buffer's declaration |
| 0x18 | 8 bytes | not read |

Indices are 32-bit if flag `0x40` is set or if the buffer has more than 65,536 vertices and `0x20` is not set;
otherwise they are 16-bit. A packed buffer stores its indices in the packed size and not in the "size of the
indices". The payload of buffer 0 starts right after the header, and each buffer's payload is its vertices
(vertex count times stride bytes) and then its stored indices (the packed size, or the size of the indices).
All bytes of the data are used: no buffer leaves bytes over.

A declaration record is 12 bytes: `u32` whose upper 16 bits are the **type** (the lower 16 bits are 0 in every
record), `u32` **usage**, `u16` **index**, 2 bytes padding. Channels are stored in the declaration's order with no
gaps, so a channel's offset in the vertex is the sum of the sizes of the channels before it, and the sizes add up
to the stride in every buffer:

| Type | Name | Bytes |
|---|---|---|
| 1 | FLOAT2 | 8 |
| 2 | FLOAT3 | 12 |
| 3 | FLOAT4 | 16 |
| 4 | E3DCOLOR (stored blue, green, red, alpha) | 4 |
| 5 | UBYTE4 | 4 |
| 6 | SHORT2 | 4 |
| 7 | SHORT4 | 8 |
| 9 | SHORT2N | 4 |
| 10 | SHORT4N | 8 |
| 15 | HALF2 | 4 |
| 16 | HALF4 | 8 |

Usages in the install: 0 position (`SHORT4N` in 63,222 buffers, `FLOAT3` in 5,512), 1 normal (`E3DCOLOR`, 68,734),
2 color (`E3DCOLOR`, 945), 3 texture coordinates (`SHORT2` sets 0, 1, 2 and 6; `FLOAT2` sets 0 to 2; `E3DCOLOR` sets 2 to 5).

Evidence, the same `m4a1_76w_sherman_xray`, unpacked header: the table is at 0x38 with 1 record
`44 71 00 00 | 0c c3 a9 01 | 08 4f 03 00 | 20 02 00 00 | 20 00 00 00 | 02 00 00 00 | 0 0`: 28,996 vertices;
`w1 = 0x01a9c30c` is stride 12 and packed index size `0x01a9c3` = 108,995; `w2 = 0x00034f08` is 216,840 bytes
of indices before packing (`0x34f08 / 2` = 108,420 indices of 16 bits); flags `0x220` is "packed, 16-bit"; two
declarations at 0x20: `type 10 (SHORT4N), usage 0` and `type 4 (E3DCOLOR), usage 1`: a position and a normal,
12 bytes. 88 (header) + 28,996 * 12 + 108,995 = 457,035, the unpacked size.

The vertex of a model is decoded as follows (the rest of the channels are not decoded by this crate):

- Position `SHORT4N`: `x = i16 / 32767 * pos_scale + pos_offset` per axis; `FLOAT3`/`FLOAT4`: the floats.
- Normal `E3DCOLOR`: bytes are blue, green, red, alpha, and the normal is `(red, green, blue) / 127.5 - 1`.
- Texture coordinates 0 and 1 `SHORT2`: `i16 / 4096`; `FLOAT2`: the floats.
- Bones of skinned vertices: texture coordinate sets 4 (the four bone slots, `E3DCOLOR` or `SHORT4` when a model
  has more than 255 bones) and 5 (their four weights, `E3DCOLOR` as `byte / 255` or `FLOAT4`). wt_blk's notes say
  sets 6 and 7 hold a second group of four; no model of the install has one (10,650 skinned buffers have sets 4 and 5
  as `E3DCOLOR`).

### The lods dump

| Offset | Content |
|---|---|
| 0x00 | table of LODs: 16-byte records `u64 scene dump size`, `f32 range`, 4 bytes |
| 0x28 | 3 x f32 `pos_offset` |
| 0x38 | 3 x f32 `pos_scale` |

Evidence, `m4a1_76w_sherman` (file offset 0x1cea30): lods dump 136 bytes, table at 0x48 with 4 LODs, offset
`[0.0278, 1.2769, 4.2e-7]`, scale `[3.0291, 2.5604, 1.3112]`, LOD ranges 40, 180 and two more.

Some resources of class `B4B7D9C4` have a lods dump of 56 bytes, too short to hold a position scale (157 of
them in the install, such as `dynBase_chp_mesh` of `fortifications.grp` and `dynaf_africa_kahili_mesh` of
`airfields.grp`); they are not vehicle models and this crate refuses them.

### Node names

`u32 size`, then a dump of a sorted name map (`RoNameMapEx`): a table of 8-byte name pointers at 0x00, and a
table of `u16` node ids at 0x10 with the same count; sorted slot `i` has the name at pointer `i` and the id `i`'s
`u16`. `names[id]` is the name of node `id`. The names are node names of the model's skeleton (`@root`, `hull`,
`turret_01`...). Evidence, `m4a1_76w_sherman`: size 2,896, names table at 0x6c8 with 116 entries, ids at 0xa68
with 116, first name `@root`.

### Scenes and meshes

A scene dump:

| Offset | Content |
|---|---|
| 0x00 | table of rigid parts: 32-byte records |
| 0x14 | u32 number of skinned meshes of this LOD |

A rigid part record: `u64 mesh dump size`, 16 bytes (a bounding sphere, not read), `u32 node id`, 4 bytes. The
mesh dumps follow the scene dump in the stream, one for each rigid part, in order.

A mesh dump (`ShaderMesh`; offsets inside it are from its own start):

| Offset | Content |
|---|---|
| 0x00 | table of elements |
| 0x10 | table of a second set of elements (the see-through parts: glass, sights) |
| 0x20 | u32; bit 31 set means the second table is not used, and then only the first is read |

An element is 48 bytes: 8 bytes (a pointer the game fills in), `u64 material`, `u64 vertex buffer`, 12 bytes (a
vertex range, not read), `u32 first index`, `u32 triangle count`, `u32 base vertex`. The element draws the indices
`first_index .. first_index + 3 * triangles` of the buffer, each with `base_vertex` added. The material is a number
that indexes the vehicle's material list (`matR` for rigid parts, `matS` for skins) in `dynModelDesc.bin`.

Evidence, `m4a1_76w_sherman_xray`, LOD 0: scene of 2,784 bytes at +0x176e9 with its table at 0x20 and 86 parts;
part 0 is `58 00 00 00 00 00 00 00 | 76 9d 28 bf 33 d7 e9 3d 1b cb 43 3d 9d 5f 31 3f | 01 00 00 00 | 0`: a mesh of
88 bytes on node 1. Its mesh dump is `28 00 00 00 01 00 00 00 ... | 58 00 00 00 ... | 01 00 00 00` (table at 0x28 with
1 element, second table at 0x58, flags word 1), and the element at 0x28 is
`00*8 | 00*8 (material 0) | 00*8 (buffer 0) | 54 00 00 00 99 6d 00 00 1a 02 00 00 | b5 9d 01 00 (first index 105,909) | 16 02 00 00 (534 triangles) | 00 00 00 00`.

Every element of all 16,934 models names an existing buffer and indices that land inside it
(this crate refuses a model where one does not, so that callers need not check).

### Skinned meshes

If any LOD's scene has skinned meshes, this section follows the last mesh dump:

| Content |
|---|
| `u32 variants`, `u32 total`: `total` is the number of bytes after these 8 up to the end of the section |
| `variants` entries `u32 constants`, `u32 gap` |
| four words and vertex data, as at the start of the model (the buffers of the skins) |
| a zlib stream up to the end of the section |

All 3,632 skinned models have `variants = 1` with the entry `(768, 0)`. The reader refuses anything else (see "Not
seen in the data"). The zlib stream (`78 da`) holds, for each LOD, in order: `u32` node ids, one per skin; `u64`
sizes, one per skin, with bit 30 set (`0x40000000`); and the skins' dumps, each of the size given.

A skin dump (offsets from its start):

| Offset | Content |
|---|---|
| 0x00 | table of bones: 48-byte records, a 3x4 matrix as 12 f32 (three columns of axes, then the translation) |
| 0x10 | table of skinned materials: 16-byte records, each the table of its palette (`u16` bone indices) |
| 0x20 | u64 offset of `u16` skeleton node ids, one per bone |
| 0x28 | u64 offset of u64 material numbers, one per skinned material |
| 0x30 | u64, filled in by the game |
| 0x38 | a mesh dump, as above |

The matrix of a bone is the transform from model space to the bone's space in the bind pose (`inverse_bind`). A
vertex names palette slots; a material with an empty palette uses the bone table's own indices. Evidence,
`m4a1_76w_sherman`, file offset 0x11fc30: `01 00 00 00 | 17 49 00 00` (1 variant, total 18,711), `00 03 00 00 | 00 00 00 00`
(768 constants, gap 0), then `ff ff ff ff ff ff ff ff 04 00 00 00 e8 00 00 c0` (textures and materials
unset, 4 buffers, vertex word `0xc00000e8`: packed, header of 232 bytes), and, after the block, the stream
`78 da ed 96 4d 68 13 41`, which inflates to 5,592 bytes beginning `63 00 00 00 72 00 00 00 73 00 00 00 98 03 00 40`:
the skins of LOD 0 are on nodes 99, 114 and 115, and the first one is `0x398` = 920 bytes with bit 30 set.

## Packed index buffers (meshoptimizer's index sequence)

The format of `meshopt_encodeIndexSequence` of [meshoptimizer](https://github.com/zeux/meshoptimizer) (MIT), which
flag `0x200` of a buffer selects: a header byte `0xD0 | version` (version 0 or 1), one variable-length integer per
index, and a 4-byte tail.

- A variable-length integer takes 1 to 5 bytes, 7 bits each, lowest group first; a byte below `0x80` is the last.
  The fifth byte's top bit is not a continuation bit.
- Bit 0 of the integer selects one of two bases; the rest `z` is a zigzag-coded delta, `(z >> 1) ^ -(z & 1)`; the
  index is `base + delta` (wrapping) and becomes that base.
- Both bases start at 0. A buffer that stores 16-bit indices keeps the low 16 bits.
- The stream ends exactly before the tail (the size in the buffer record is the size of header, integers and tail).

Evidence: every packed buffer of the 16,934 models decodes, with all its indices below the vertex count of its
buffer, and the element ranges are inside them.

## DXP texture packs (`*.dxp.bin`)

| Offset | Type | Field |
|---|---|---|
| 0x00 | 4 bytes | `DxP2` (all 4,318 files) |
| 0x04 | u32 | version, 2 (all files) |
| 0x08 | u32 | texture count (equals the count of each table below in all 4,318 files; not read) |
| 0x0C | u32 | size of the dump that follows |
| 0x10 | | the dump; offsets in it are from 0x10 |

The dump starts with three tables, 16 bytes apart (8 bytes of padding that is 0 in all files), each with one
record per texture:

| Dump offset | Table | Record |
|---|---|---|
| 0x00 | names | u64 offset of a NUL-terminated name |
| 0x10 | headers | the 32-byte DDSx header (below) |
| 0x20 | records | 24 bytes: `u32 0`, `u32 0`, `u32 0xFFFFFFFF`, `u32 file offset` of the texture's data, `u32 size`, `u32 0`; the offset is from the start of the file |

All 176,930 records of the install have 0, 0, `0xFFFFFFFF` and 0 in the fields shown as constants. The size is
the packed size for a packed texture (the same as `packed_size` of the header) and the unpacked size
(`mem_size`) for a stored one (all 23,774 stored textures; 32 more have 0 in both the header and the record).
The data of a texture is `size` bytes at the offset; there are 28,595 entries that are only a label
(a header that is the bytes `DDSx` followed by zeros, with size 0 in the record, 32 of them with 0 in the packed-size field too; the offset in the record is a position
in the file, its very end in 8,822 of them, and there is no data there). Entries do not overlap in any of the 4,318 files.

Evidence, `content/base/res/abandoned.dxp.bin` (5,183,272 bytes): `44 78 50 32 | 02 00 00 00 | 23 00 00 00 | c8 0c 00 00`
(35 textures, a dump of 0xcc8 bytes); tables at dump offsets 0x400 (names), 0x520 (headers), 0x980 (records);
entry 0 is named `af_cc_departments_low_tex_d*` and its record is
`00*8 | ff ff ff ff | 00 00 01 00 | 28 04 02 00 | 00 00 00 00`: data at file offset 65,536, 132,136 bytes.
`content/base/res/tile_tex_d-hq.dxp.bin` (224 bytes) is a pack of one label-only entry
(`detail_tropic_deadgrass_tex_n$hq*`) whose record points at offset 224, the end of the file.

## DDSx textures

The 32-byte header of each texture:

| Offset | Type | Field |
|---|---|---|
| 0x00 | 4 bytes | `DDSx` |
| 0x04 | u32 | format: a FourCC (`DXT1`, `DXT5`, `ATI2`, `BC7 ` ...) as little-endian text, or a number (a `D3DFMT_*`) |
| 0x08 | u32 | flags (below) |
| 0x0C | u16, u16 | width, height of the top mip |
| 0x10 | u8 | mip levels the full texture has |
| 0x11 | u8 | levels held in the separate high-quality part (not used here) |
| 0x12 | u16 | depth of a volume texture |
| 0x14 | u16 | bits per pixel (uncompressed formats) |
| 0x16 | u8 | not read |
| 0x17 | u8 | the low nibble is `log2` of the bytes in a 4x4 block (3 for DXT1 and ATI1, 4 for the rest), 0 for uncompressed formats; the high nibble is not read |
| 0x18 | u32 | `mem_size`: bytes of surface data once unpacked |
| 0x1C | u32 | `packed_size`: bytes of packed data, 0 when stored |

Flags: `0x0800` cubemap, `0x1000` volume, `0x2000` or `0x4000` only the top mip is stored (the game makes the others),
`0x20000` stored as a difference to a paired base texture (not read), `0x40000` mips are stored smallest first,
`0x200000` array texture (not read). The top three bits (`flags >> 29`) say how the data is packed: 0 stored,
1 zstd, 2 7-zip, 3 Oodle, 4 zlib; 5 to 7 are not defined.

### Unpacking and the order of the data

- Stored (`flags >> 29 == 0`): the `mem_size` bytes are at the data offset of the pack's record.
- Oodle (3): the data is an Oodle stream of `packed_size` bytes that unpacks to `mem_size` bytes. Oodle blocks start
  every 256 KiB of output, so a stream decoded into a buffer that ends on a block boundary gives the same bytes as
  the start of the whole; the crate relies on that to decode in growing prefixes instead of trusting `mem_size`.
- zstd (1): a zstd frame of `packed_size` bytes. No texture of the install uses it.
- The unpacked data may be longer than the surfaces the header describes, and the extra bytes are dropped. In the
  install this happens in 39,683 textures, all with a generate-mips flag, and always by exactly 12 bytes after the
  top mip; in all 108,309 other textures `mem_size` equals the described size exactly
  (`oracle/survey/ddsx_sizes.py`).

The surfaces are: for each stored mip level (all of `levels`, or only the first when a generate-mips flag is
set), `slices` surfaces, where a surface of `w x h` pixels (each side halved per level, never below 1) takes
`ceil(w/4) * ceil(h/4) << dxt_shift` bytes for block formats and `w * h * bits_per_pixel / 8` otherwise, and a volume
has `depth >> level` slices (at least 1); a cubemap has six faces of the whole chain. The unpacked data of every
texture of the install is at least this long.

With flag `0x40000` the mips are stored smallest first. A flat texture then holds its levels from the smallest to
the largest, each as a whole; a cubemap holds, for each mip from the smallest, its six faces one after the other.
DDS wants the largest level first, and for a cubemap each face with its whole chain; this crate puts the data in
that order (72 cubemaps and 62 volumes are in the install; 89,686 textures have the flag).

### What the install holds

Of the 176,930 entries of the 4,318 packs (War Thunder 2.59.0.80):

| | Count |
|---|---|
| Label only (format 0, size 0, no data) | 28,595 |
| Stored | 23,774 |
| Oodle | 124,561 |
| zstd, zlib, 7-zip, flag bits 5 to 7 | 0 |
| Flag `0x20000` (stored as a difference to a paired base texture) | 375 (not decoded) |
| Flag `0x200000` (array texture) | 0 |
| Cubemaps / volumes | 72 / 62 |
| Generate-mips flag | 40,058 |
| Smallest mip first | 89,686 |
| Unpacked and converted to RGBA | 147,960 (109.0 GB unpacked) |

Formats: `DXT5` 75,356; `DXT1` 49,759; `BC7 ` 18,052; `ATI2` 2,406; `ATI1` 737; `D3DFMT_113`
(A16B16G16R16F) 892; `D3DFMT_32` (A8B8G8R8) 565; `D3DFMT_21` (A8R8G8B8) 447; `D3DFMT_50` (L8) 104; `D3DFMT_28` (A8) 13;
`D3DFMT_111` (R16F) 4. The largest texture is 8192 x 8192 with 13 levels; the largest `mem_size` is 89,128,960
bytes and the largest `packed_size` 30,373,936. The flags without the compression bits take 51 different values (all listed by `oracle dxp`); the commonest are
`0x48011` (38,496 entries), `0x40011` (36,472), `0x4011` (14,576), `0xc011` (14,081) and `0x88011` (7,990).

Evidence, entry 0 of `content/base/res/abandoned.dxp.bin`:

```
44 44 53 78  44 58 54 35  11 00 04 60  00 02 00 02  0a 02 00 00  00 00 12 74  70 55 05 00  28 04 02 00
DDSx         DXT5         flags 0x60040011  512 x 512  10 levels, 2 in the HQ part, depth 0, bpp 0,
                          = Oodle (3), smallest first (0x40000), 0x11  byte 0x16 = 0x12, byte 0x17 = 0x74 (shift 4)
                                                         mem_size 0x55570 = 349,552   packed_size 0x20428 = 132,136
```

(`DXT5` is 16 bytes a block, shift 4. The ten levels take 262,144 + 65,536 + 16,384 + 4,096 + 1,024 + 256 + 64 + 16
+ 16 + 16 = 349,552 bytes, which is `mem_size`; the 1x1 and 2x2 levels each still take a whole block.)

## Not seen in the data

These are read by the code, from the layouts above, but nothing in the install exercises them; they are tested
on synthetic data only, and the oracle (wt_blk) agrees with them only there.

- `GRP3` packs (every pack is `GRP2`).
- Packed skeletons (all 18,770 are stored), and a vertex block packed with zstd instead of Oodle.
- Skin vertex buffers with their bone slots as `SHORT4` or their weights as `FLOAT4`, and a second group of four
  bone influences (all 10,650 skinned buffers have one group, as `E3DCOLOR`).
- Textures packed with zstd, zlib or 7-zip (none in the install).
- Skinned meshes with more than one variant, or a variant with a gap: this crate refuses them (wt_blk reads
  them by skipping the gap and trying the entries in turn, which no file confirms).
- Flag bits 5 to 7 of the compression: refused here; wt_blk reads the data as stored.
- Texture names that are not UTF-8: replaced here; wt_blk refuses the whole pack.
