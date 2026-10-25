// Provenance: this file is substantially similar to this project's own code in `src/dxp_and_grp/ddsx.rs` of
// the unmerged wt_blk pull requests #28 and #29 (github.com/patodois/wt_blk, commit 498c996), after which it was
// written. That code follows the Dagor Engine sources (BSD-3-Clause, github.com/GaijinEntertainment/DagorEngine).
// No line of wt_blk's upstream author is in it.
// See the README, "Credit and provenance".

//! DDSx: Dagor's texture container, the payload of each entry of a [`DXP`](crate::dxp) pack.
//!
//! A texture is a 32-byte header followed by its surfaces, stored or packed with zstd or Oodle. The
//! layout is in `docs/FORMATS.md`.

use crate::error::{Error, Result};
use crate::unpack;

const LABEL: &[u8; 4] = b"DDSx";

const FLAG_CUBEMAP: u32 = 0x0000_0800;
const FLAG_VOLUME: u32 = 0x0000_1000;
/// Either of these: only the top mip is stored and the game builds the rest
const FLAG_GENERATE_MIPS: u32 = 0x0000_2000 | 0x0000_4000;
const FLAG_PAIRED_BASE: u32 = 0x0002_0000;
const FLAG_SMALLEST_MIP_FIRST: u32 = 0x0004_0000;
const FLAG_ARRAY: u32 = 0x0020_0000;
const COMPRESSION_MASK: u32 = 0xE000_0000;

/// The most pixels [`Texture::to_rgba`] and [`Texture::to_png`] convert: 8192 by 8192, the largest texture
/// of War Thunder 2.59.0.80.
#[cfg(feature = "png")]
pub const MAX_RGBA_PIXELS: usize = 8192 * 8192;

#[cfg(feature = "png")]
const D3DFMT_A8R8G8B8: u32 = 21;
#[cfg(feature = "png")]
const D3DFMT_A8: u32 = 28;
#[cfg(feature = "png")]
const D3DFMT_A8B8G8R8: u32 = 32;
#[cfg(feature = "png")]
const D3DFMT_L8: u32 = 50;
#[cfg(feature = "png")]
const D3DFMT_R16F: u32 = 111;
#[cfg(feature = "png")]
const D3DFMT_A16B16G16R16F: u32 = 113;

/// How the surface data after a header is packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Stored as it is.
    None,
    /// A zstd frame.
    Zstd,
    /// An Oodle stream.
    Oodle,
    /// A zlib stream (no texture of War Thunder 2.59 uses it; not supported).
    Zlib,
    /// A 7-zip stream (not used by the game either; not supported).
    SevenZip,
    /// A value of the top three flag bits that no packer is known for.
    Unknown(u8),
}

/// The 32-byte header of a texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DdsxHeader {
    /// A `D3DFMT_*` number, or a FourCC such as `DXT5` read as a little-endian `u32`.
    pub d3d_format: u32,
    /// Flags: see [`DdsxHeader::compression`] and the `is_*` methods.
    pub flags: u32,
    /// Width of the top mip in pixels.
    pub width: u16,
    /// Height of the top mip in pixels.
    pub height: u16,
    /// Mip levels the full texture has.
    pub levels: u8,
    /// Levels that are in the separate high-quality part of a texture.
    pub hq_part_levels: u8,
    /// Depth of a volume texture.
    pub depth: u16,
    /// Bits per pixel of uncompressed formats.
    pub bits_per_pixel: u16,
    /// `log2` of the bytes in a 4x4 block, 0 for formats that are not block-compressed.
    pub dxt_shift: u8,
    /// Bytes of surface data once unpacked.
    pub mem_size: u32,
    /// Bytes of packed data after the header, 0 when the data is stored.
    pub packed_size: u32,
}

impl DdsxHeader {
    /// Bytes in a header.
    pub const SIZE: usize = 32;

    /// Reads the header at the start of `bytes`.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let b = bytes.get(..Self::SIZE).ok_or(Error::OutOfBounds { at: Self::SIZE, len: bytes.len() })?;
        if &b[..4] != LABEL {
            return Err(Error::BadLabel { found: String::from_utf8_lossy(&b[..4]).into_owned() });
        }
        let u16_at = |at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        Ok(DdsxHeader {
            d3d_format: u32_at(0x04),
            flags: u32_at(0x08),
            width: u16_at(0x0C),
            height: u16_at(0x0E),
            levels: b[0x10],
            hq_part_levels: b[0x11],
            depth: u16_at(0x12),
            bits_per_pixel: u16_at(0x14),
            dxt_shift: b[0x17] & 0x0F,
            mem_size: u32_at(0x18),
            packed_size: u32_at(0x1C),
        })
    }

    /// Entries that are only a label: a texture the pack names but does not hold.
    pub fn is_empty(&self) -> bool {
        self.d3d_format == 0 && self.width == 0 && self.height == 0 && self.mem_size == 0
    }

    /// How the data after the header is packed.
    pub fn compression(&self) -> Compression {
        match self.flags & COMPRESSION_MASK {
            0 => Compression::None,
            0x2000_0000 => Compression::Zstd,
            0x4000_0000 => Compression::SevenZip,
            0x6000_0000 => Compression::Oodle,
            0x8000_0000 => Compression::Zlib,
            other => Compression::Unknown((other >> 29) as u8),
        }
    }

    /// Six faces, each with its mips.
    pub fn is_cubemap(&self) -> bool {
        self.flags & FLAG_CUBEMAP != 0
    }

    /// A 3D texture: every mip has `depth >> level` slices.
    pub fn is_volume(&self) -> bool {
        self.flags & FLAG_VOLUME != 0
    }

    /// Only the top mip is stored; the game builds the others when it loads the texture.
    pub fn generates_mips(&self) -> bool {
        self.flags & FLAG_GENERATE_MIPS != 0
    }

    /// The format as text: `DXT5` for a FourCC, `D3DFMT_21` for a number.
    pub fn format_name(&self) -> String {
        let bytes = self.d3d_format.to_le_bytes();
        if bytes.iter().all(|b| b.is_ascii_graphic() || *b == b' ') {
            String::from_utf8_lossy(&bytes).trim_end().to_owned()
        } else {
            format!("D3DFMT_{}", self.d3d_format)
        }
    }

    /// One side of the texture at a mip level. A header may claim up to 255 levels, past the 1x1 one.
    fn mip_side(side: u16, level: u32) -> u64 {
        u64::from(side).checked_shr(level).unwrap_or(0).max(1)
    }

    /// Bytes of one slice at a mip level. Sizes are `u64` so that nothing a header can say overflows,
    /// including where `usize` has 32 bits.
    fn slice_bytes(&self, level: u32) -> u64 {
        let (w, h) = (Self::mip_side(self.width, level), Self::mip_side(self.height, level));
        if self.dxt_shift == 0 {
            w * h * u64::from(self.bits_per_pixel) / 8
        } else {
            // Edges that are not a multiple of 4 pixels still take a whole block
            (w.div_ceil(4) * h.div_ceil(4)) << self.dxt_shift
        }
    }

    fn level_bytes(&self, level: u32) -> u64 {
        let slices = if self.is_volume() { Self::mip_side(self.depth, level) } else { 1 };
        self.slice_bytes(level) * slices
    }

    /// How many mips the unpacked data holds.
    fn stored_levels(&self) -> u32 {
        if self.generates_mips() {
            1
        } else {
            u32::from(self.levels.max(1))
        }
    }

    fn faces(&self) -> u64 {
        if self.is_cubemap() {
            6
        } else {
            1
        }
    }

    /// The bytes all stored surfaces take.
    fn surface_bytes(&self) -> u64 {
        (0..self.stored_levels()).map(|l| self.level_bytes(l)).sum::<u64>() * self.faces()
    }
}

/// A texture with its surface data unpacked and in DDS order: largest mip first, and for a cubemap
/// each face with its whole mip chain.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Texture {
    /// The header the data was read with.
    pub header: DdsxHeader,
    /// The surface data.
    pub data: Vec<u8>,
}

impl Texture {
    /// Unpacks the bytes that follow a header (`raw`), checking them against what the header describes.
    pub fn from_ddsx(header: DdsxHeader, raw: &[u8]) -> Result<Self> {
        if header.is_empty() {
            return Err(Error::unsupported("an entry that is only a label"));
        }
        if header.flags & FLAG_PAIRED_BASE != 0 {
            return Err(Error::unsupported("a texture stored as the difference to a paired base texture"));
        }
        if header.flags & FLAG_ARRAY != 0 {
            return Err(Error::unsupported("array textures"));
        }
        if header.is_cubemap() && header.is_volume() {
            return Err(Error::unsupported("a texture that is both a cubemap and a volume"));
        }

        let mem_size = header.mem_size as usize;
        let packed = || {
            let size = header.packed_size as usize;
            raw.get(..size).ok_or(Error::OutOfBounds { at: size, len: raw.len() })
        };
        let mut data = match header.compression() {
            Compression::None => raw.get(..mem_size).ok_or(Error::OutOfBounds { at: mem_size, len: raw.len() })?.to_vec(),
            Compression::Zstd => unpack::zstd(packed()?, mem_size)?,
            Compression::Oodle => unpack::oodle(packed()?, mem_size)?,
            Compression::Zlib => return Err(Error::unsupported("zlib-packed textures")),
            Compression::SevenZip => return Err(Error::unsupported("7-zip-packed textures")),
            Compression::Unknown(v) => return Err(Error::unsupported(format!("compression {v} (top bits of the flags)"))),
        };

        let expected = header.surface_bytes();
        if (data.len() as u64) < expected {
            return Err(Error::Decompress(format!("{} bytes of surface data, but the header describes {expected}", data.len())));
        }
        // Textures whose mips are generated carry a few bytes after the top mip
        data.truncate(expected as usize);

        if header.flags & FLAG_SMALLEST_MIP_FIRST != 0 && !header.generates_mips() {
            data = smallest_first_to_dds(&header, &data);
        }
        Ok(Texture { header, data })
    }

    /// Width of the top mip.
    pub fn width(&self) -> u32 {
        u32::from(self.header.width)
    }

    /// Height of the top mip.
    pub fn height(&self) -> u32 {
        u32::from(self.header.height)
    }
}

/// Puts mips stored smallest first into DDS order. `data` is exactly the surface data of `header`.
fn smallest_first_to_dds(header: &DdsxHeader, data: &[u8]) -> Vec<u8> {
    let levels = header.stored_levels();
    let mut out = vec![0u8; data.len()];
    if header.is_cubemap() {
        // Stored: for each mip from the smallest, the six faces one after the other.
        // Wanted: for each face, its mips from the largest.
        let slice = |level: u32| header.slice_bytes(level) as usize;
        let face_bytes: usize = (0..levels).map(slice).sum();
        let mut mip_start = Vec::with_capacity(levels as usize);
        let mut at = 0;
        for level in 0..levels {
            mip_start.push(at);
            at += slice(level);
        }
        let mut src = 0;
        for level in (0..levels).rev() {
            let bytes = slice(level);
            for face in 0..6 {
                let dst = face * face_bytes + mip_start[level as usize];
                out[dst..dst + bytes].copy_from_slice(&data[src..src + bytes]);
                src += bytes;
            }
        }
    } else {
        // Whole levels, so the reordered data is the same chunks in the opposite order
        let mut src = 0;
        let mut dst = out.len();
        for level in (0..levels).rev() {
            let bytes = header.level_bytes(level) as usize;
            out[dst - bytes..dst].copy_from_slice(&data[src..src + bytes]);
            src += bytes;
            dst -= bytes;
        }
    }
    out
}

#[cfg(feature = "png")]
impl Texture {
    /// The bytes of the top mip, if this crate converts the format and the data holds them.
    fn top_mip(&self) -> Result<&[u8]> {
        let h = &self.header;
        let (w, ht) = (u64::from(h.width), u64::from(h.height));
        let blocks = w.div_ceil(4) * ht.div_ceil(4);
        let cannot = || Error::unsupported(format!("converting {} to RGBA", h.format_name()));
        let needed = match &h.d3d_format.to_le_bytes() {
            b"DXT1" | b"ATI1" => blocks * 8,
            b"DXT3" | b"DXT5" | b"ATI2" | b"BC7 " => blocks * 16,
            _ => match h.d3d_format {
                D3DFMT_A8R8G8B8 | D3DFMT_A8B8G8R8 => w * ht * 4,
                D3DFMT_L8 | D3DFMT_A8 => w * ht,
                D3DFMT_A16B16G16R16F => w * ht * 8,
                D3DFMT_R16F => w * ht * 2,
                _ => return Err(cannot()),
            },
        };
        // Size, block size and bits per pixel are separate fields: the top mip must hold what the
        // format needs before anything is allocated for its pixels
        let have = h.slice_bytes(0);
        usize::try_from(have)
            .ok()
            .filter(|_| have == needed)
            .and_then(|have| self.data.get(..have))
            .ok_or_else(|| Error::unsupported(format!("a {w}x{ht} {} top mip of {have} bytes", h.format_name())))
    }

    /// Decodes the top mip (the first face or slice) to 8-bit RGBA, rows from the top. Half-float
    /// formats are clamped to 0..1. A texture of more than [`MAX_RGBA_PIXELS`] pixels is refused, so
    /// that a small hostile file cannot ask for gigabytes of pixels; the conversion takes at most
    /// 8 bytes a pixel while it runs.
    pub fn to_rgba(&self) -> Result<Vec<u8>> {
        let (w, h) = (self.width() as usize, self.height() as usize);
        if w * h > MAX_RGBA_PIXELS {
            return Err(Error::unsupported(format!("converting {w}x{h} pixels to RGBA (the limit is {MAX_RGBA_PIXELS})")));
        }
        let top = self.top_mip()?;
        let four_cc = |s: &[u8; 4]| u32::from_le_bytes(*s);
        type BlockDecoder = fn(&[u8], usize, usize, &mut [u32]) -> std::result::Result<(), &'static str>;
        let block_decoder: Option<BlockDecoder> = match self.header.d3d_format {
            f if f == four_cc(b"DXT1") => Some(texture2ddecoder::decode_bc1),
            f if f == four_cc(b"DXT3") => Some(texture2ddecoder::decode_bc2),
            f if f == four_cc(b"DXT5") => Some(texture2ddecoder::decode_bc3),
            f if f == four_cc(b"ATI1") => Some(texture2ddecoder::decode_bc4),
            f if f == four_cc(b"ATI2") => Some(texture2ddecoder::decode_bc5),
            f if f == four_cc(b"BC7 ") => Some(texture2ddecoder::decode_bc7),
            _ => None,
        };

        let bytes = (w * h).checked_mul(4).ok_or_else(|| Error::unsupported(format!("{w}x{h} pixels as RGBA")))?;
        let mut rgba = Vec::with_capacity(bytes);
        if let Some(decode) = block_decoder {
            let mut pixels = vec![0u32; w * h];
            decode(top, w, h, &mut pixels).map_err(|e| Error::Decompress(e.to_owned()))?;
            // The block decoders pack a pixel as 0xAARRGGBB
            for p in pixels {
                let [b, g, r, a] = p.to_le_bytes();
                rgba.extend_from_slice(&[r, g, b, a]);
            }
            return Ok(rgba);
        }
        match self.header.d3d_format {
            // Bytes in memory are B, G, R, A
            D3DFMT_A8R8G8B8 => top.as_chunks::<4>().0.iter().for_each(|p| rgba.extend_from_slice(&[p[2], p[1], p[0], p[3]])),
            D3DFMT_A8B8G8R8 => rgba.extend_from_slice(top),
            D3DFMT_L8 => top.iter().for_each(|&l| rgba.extend_from_slice(&[l, l, l, 0xFF])),
            D3DFMT_A8 => top.iter().for_each(|&a| rgba.extend_from_slice(&[0xFF, 0xFF, 0xFF, a])),
            D3DFMT_A16B16G16R16F => top.as_chunks::<2>().0.iter().for_each(|c| rgba.push(unorm8(half_to_f32(u16::from_le_bytes(*c))))),
            D3DFMT_R16F => top.as_chunks::<2>().0.iter().for_each(|c| {
                let r = unorm8(half_to_f32(u16::from_le_bytes(*c)));
                rgba.extend_from_slice(&[r, r, r, 0xFF]);
            }),
            _ => unreachable!("top_mip rejected the other formats"),
        }
        Ok(rgba)
    }

    /// Encodes the top mip as an 8-bit RGBA PNG.
    pub fn to_png(&self) -> Result<Vec<u8>> {
        let rgba = self.to_rgba()?;
        let fail = |e: png::EncodingError| Error::Encode(e.to_string());
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width(), self.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(fail)?;
        writer.write_image_data(&rgba).map_err(fail)?;
        writer.finish().map_err(fail)?;
        Ok(out)
    }
}

/// 0..1 to a byte, clamping what is outside.
#[cfg(feature = "png")]
fn unorm8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// IEEE 754 half precision to single.
#[cfg(feature = "png")]
fn half_to_f32(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from((h >> 10) & 0x1F);
    let frac = u32::from(h & 0x3FF);
    let bits = match (exp, frac) {
        (0, 0) => sign,
        // Subnormal: shift the fraction up until its top bit is the implicit one
        (0, _) => {
            let shift = frac.leading_zeros() - 21;
            sign | ((113 - shift) << 23) | (((frac << shift) & 0x3FF) << 13)
        }
        (0x1F, _) => sign | 0x7F80_0000 | (frac << 13),
        _ => sign | ((exp + 112) << 23) | (frac << 13),
    };
    f32::from_bits(bits)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const DXT1: u32 = u32::from_le_bytes(*b"DXT1");

    /// The header of a square DXT1 texture
    pub(crate) fn dxt1(side: u16, levels: u8, flags: u32, mem_size: u32) -> DdsxHeader {
        DdsxHeader {
            d3d_format: DXT1,
            flags,
            width: side,
            height: side,
            levels,
            hq_part_levels: 0,
            depth: 0,
            bits_per_pixel: 0,
            dxt_shift: 3,
            mem_size,
            packed_size: 0,
        }
    }

    #[test]
    fn header_fields() {
        let mut raw = [0u8; 32];
        raw[..4].copy_from_slice(b"DDSx");
        raw[4..8].copy_from_slice(b"DXT5");
        raw[8..12].copy_from_slice(&0x6004_0011u32.to_le_bytes());
        raw[12..14].copy_from_slice(&512u16.to_le_bytes());
        raw[14..16].copy_from_slice(&256u16.to_le_bytes());
        raw[16] = 10;
        raw[17] = 2;
        raw[0x17] = 0x74;
        raw[0x18..0x1C].copy_from_slice(&174_776u32.to_le_bytes());
        raw[0x1C..0x20].copy_from_slice(&110_032u32.to_le_bytes());
        let h = DdsxHeader::parse(&raw).unwrap();
        assert_eq!((h.width, h.height, h.levels, h.hq_part_levels, h.dxt_shift), (512, 256, 10, 2, 4));
        assert_eq!((h.mem_size, h.packed_size), (174_776, 110_032));
        assert_eq!(h.compression(), Compression::Oodle);
        assert_eq!(h.format_name(), "DXT5");
        assert_eq!(DdsxHeader { d3d_format: 21, ..h }.format_name(), "D3DFMT_21");
        assert!(DdsxHeader::parse(&raw[..31]).is_err());
        raw[3] = b'y';
        assert!(DdsxHeader::parse(&raw).is_err());
    }

    #[test]
    fn compression_comes_from_the_top_three_bits() {
        let c = |flags| DdsxHeader { flags, ..dxt1(4, 1, 0, 8) }.compression();
        assert_eq!(c(0), Compression::None);
        assert_eq!(c(0x1FFF_FFFF), Compression::None);
        assert_eq!(c(0x2000_0000), Compression::Zstd);
        assert_eq!(c(0x4000_0000), Compression::SevenZip);
        assert_eq!(c(0x6000_0000), Compression::Oodle);
        assert_eq!(c(0x8000_0000), Compression::Zlib);
        assert_eq!(c(0xA000_0000), Compression::Unknown(5));
        assert_eq!(c(0xC000_0000), Compression::Unknown(6));
        assert_eq!(c(0xE000_0000), Compression::Unknown(7));
    }

    #[test]
    fn smallest_first_mips_are_reordered() {
        // 8x8 DXT1: mip 0 is 32 bytes and mip 1 (4x4) is 8, stored smallest first
        let mut stored = vec![1u8; 8];
        stored.extend_from_slice(&[0u8; 32]);
        let t = Texture::from_ddsx(dxt1(8, 2, FLAG_SMALLEST_MIP_FIRST, 40), &stored).unwrap();
        assert_eq!(&t.data[..32], &[0u8; 32]);
        assert_eq!(&t.data[32..], &[1u8; 8]);
    }

    #[test]
    fn block_sizes_round_up() {
        // 6x6 DXT1: mip 0 is 2x2 blocks (32 bytes), mip 1 (3x3) is one block
        let mut stored = vec![1u8; 8];
        stored.extend_from_slice(&[0u8; 32]);
        let t = Texture::from_ddsx(dxt1(6, 2, FLAG_SMALLEST_MIP_FIRST, 40), &stored).unwrap();
        assert_eq!(&t.data[..32], &[0u8; 32]);
        assert_eq!(&t.data[32..], &[1u8; 8]);
        #[cfg(feature = "png")]
        assert_eq!(t.to_rgba().unwrap().len(), 6 * 6 * 4);
    }

    #[test]
    fn cubemap_mips_become_face_major() {
        // 4x4 DXT1 cubemap with two 8-byte levels (4x4 and 2x2), stored as mip 1 of the six faces, then mip 0
        let stored: Vec<u8> = (0..12u8).flat_map(|i| [i; 8]).collect();
        let t = Texture::from_ddsx(dxt1(4, 2, FLAG_SMALLEST_MIP_FIRST | FLAG_CUBEMAP, 96), &stored).unwrap();
        for face in 0..6usize {
            let chain = &t.data[face * 16..face * 16 + 16];
            assert_eq!(&chain[..8], &[6 + face as u8; 8], "face {face} mip 0");
            assert_eq!(&chain[8..], &[face as u8; 8], "face {face} mip 1");
        }
    }

    #[test]
    fn volumes_have_a_slice_per_depth() {
        // 4x4x4 DXT1 volume, mips 4x4x4 (4 slices) and 2x2x2 (2 slices): 6 blocks of 8 bytes
        let header = DdsxHeader { depth: 4, ..dxt1(4, 2, FLAG_VOLUME, 48) };
        let t = Texture::from_ddsx(header, &[3u8; 48]).unwrap();
        assert_eq!(t.data.len(), 48);
        assert!(Texture::from_ddsx(header, &[3u8; 47]).is_err());
    }

    #[test]
    fn generated_mips_keep_the_top_level() {
        // 8x8 DXT1 plus the 12 bytes the game stores after it
        let t = Texture::from_ddsx(dxt1(8, 4, 0x4000, 44), &[7u8; 44]).unwrap();
        assert_eq!(t.data.len(), 32);
    }

    #[test]
    fn more_levels_than_the_texture_has_sizes() {
        // 40 levels: 32 bytes for the 8x8 mip, then 8 bytes for each level of one block or less
        let t = Texture::from_ddsx(dxt1(8, 40, FLAG_SMALLEST_MIP_FIRST, 32 + 39 * 8), &[0; 344]).unwrap();
        assert_eq!(t.data.len(), 344);
        // 255 levels of a volume: sizes are computed without overflow, and the data is missing
        let volume = DdsxHeader { depth: 2, ..dxt1(8, 255, FLAG_VOLUME, 0) };
        assert!(Texture::from_ddsx(volume, &[0; 16]).is_err());
    }

    #[test]
    fn packed_sizes_do_not_size_allocations() {
        // 4 GiB claimed for a handful of bytes
        let oodle = DdsxHeader { mem_size: u32::MAX, packed_size: 6, ..dxt1(8, 1, 0x6000_0000, 0) };
        assert!(Texture::from_ddsx(oodle, &[0x8C, 0x06, 0, 0, 0, 0]).is_err());
        let packed = zstd::bulk::compress(&[7; 32], 0).unwrap();
        let zstd = DdsxHeader { mem_size: u32::MAX, packed_size: packed.len() as u32, ..dxt1(8, 1, 0x2000_0000, 0) };
        assert_eq!(Texture::from_ddsx(zstd, &packed).unwrap().data, [7; 32]);
        // The header's size is also an upper bound
        assert!(Texture::from_ddsx(DdsxHeader { mem_size: 31, ..zstd }, &packed).is_err());
        // And packed data the file does not have is not read
        assert!(Texture::from_ddsx(DdsxHeader { packed_size: packed.len() as u32 + 1, ..zstd }, &packed).is_err());
    }

    #[test]
    fn unusable_entries_are_refused() {
        let empty = DdsxHeader { d3d_format: 0, width: 0, height: 0, mem_size: 0, ..dxt1(0, 0, 0, 0) };
        assert!(empty.is_empty());
        assert!(Texture::from_ddsx(empty, &[]).is_err());
        assert!(Texture::from_ddsx(dxt1(8, 1, FLAG_PAIRED_BASE, 32), &[0; 32]).is_err());
        assert!(Texture::from_ddsx(dxt1(8, 1, FLAG_ARRAY, 32), &[0; 32]).is_err());
        assert!(Texture::from_ddsx(dxt1(8, 2, 0, 40), &[0; 39]).is_err());
        assert!(Texture::from_ddsx(dxt1(8, 1, 0x4000_0000, 32), &[0; 32]).is_err());
        let both = DdsxHeader { depth: 2, ..dxt1(4, 2, FLAG_CUBEMAP | FLAG_VOLUME | FLAG_SMALLEST_MIP_FIRST, 144) };
        assert!(Texture::from_ddsx(both, &[1; 144]).is_err());
    }

    #[cfg(feature = "png")]
    mod rgba {
        use super::*;

        #[test]
        fn bgra_becomes_rgba() {
            let header = DdsxHeader { d3d_format: D3DFMT_A8R8G8B8, dxt_shift: 0, bits_per_pixel: 32, ..dxt1(1, 1, 0, 4) };
            let t = Texture::from_ddsx(header, &[1, 2, 3, 4]).unwrap();
            assert_eq!(t.to_rgba().unwrap(), [3, 2, 1, 4]);
            assert_eq!(&t.to_png().unwrap()[1..4], b"PNG");
        }

        #[test]
        fn single_channel_formats() {
            let l8 = DdsxHeader { d3d_format: D3DFMT_L8, dxt_shift: 0, bits_per_pixel: 8, ..dxt1(2, 1, 0, 4) };
            assert_eq!(Texture::from_ddsx(l8, &[0, 1, 2, 3]).unwrap().to_rgba().unwrap()[..8], [0, 0, 0, 255, 1, 1, 1, 255]);
            let a8 = DdsxHeader { d3d_format: D3DFMT_A8, ..l8 };
            assert_eq!(Texture::from_ddsx(a8, &[9, 1, 2, 3]).unwrap().to_rgba().unwrap()[..8], [255, 255, 255, 9, 255, 255, 255, 1]);
            let abgr = DdsxHeader { d3d_format: D3DFMT_A8B8G8R8, dxt_shift: 0, bits_per_pixel: 32, ..dxt1(1, 1, 0, 4) };
            assert_eq!(Texture::from_ddsx(abgr, &[1, 2, 3, 4]).unwrap().to_rgba().unwrap(), [1, 2, 3, 4]);
        }

        #[test]
        fn a_top_mip_must_match_the_format() {
            // 32-bit pixels declared with 8 bits per pixel: 4 bytes, not the 16 that 2x2 pixels need
            let header = DdsxHeader { d3d_format: D3DFMT_A8R8G8B8, dxt_shift: 0, bits_per_pixel: 8, ..dxt1(2, 1, 0, 4) };
            assert!(Texture::from_ddsx(header, &[0; 4]).unwrap().to_rgba().is_err());
            // DXT1 with no block size has no surface data, and must not allocate for 65535x65535 pixels
            let header = DdsxHeader { dxt_shift: 0, ..dxt1(65535, 1, 0, 1) };
            let t = Texture::from_ddsx(header, &[0]).unwrap();
            assert!(t.data.is_empty());
            assert!(t.to_rgba().is_err() && t.to_png().is_err());
            // A format the crate does not convert
            let header = DdsxHeader { d3d_format: 23, dxt_shift: 0, bits_per_pixel: 16, ..dxt1(1, 1, 0, 2) };
            assert!(Texture::from_ddsx(header, &[0; 2]).unwrap().to_rgba().is_err());
        }

        #[test]
        fn halves() {
            assert_eq!(half_to_f32(0x3C00), 1.0);
            assert_eq!(half_to_f32(0xC000), -2.0);
            assert_eq!(half_to_f32(0x3800), 0.5);
            assert_eq!(half_to_f32(0x0001), 2.0f32.powi(-24));
            assert_eq!(half_to_f32(0x0000), 0.0);
            assert!(half_to_f32(0x8000).is_sign_negative());
            assert!(half_to_f32(0x7C00).is_infinite());
            assert!(half_to_f32(0x7E00).is_nan());
            // 1.0, 0.5, -1.0, 2.0 clamp to 255, 128, 0, 255
            let header = DdsxHeader { d3d_format: D3DFMT_A16B16G16R16F, dxt_shift: 0, bits_per_pixel: 64, ..dxt1(1, 1, 0, 8) };
            let raw: Vec<u8> = [0x3C00u16, 0x3800, 0xBC00, 0x4000].iter().flat_map(|h| h.to_le_bytes()).collect();
            assert_eq!(Texture::from_ddsx(header, &raw).unwrap().to_rgba().unwrap(), [255, 128, 0, 255]);
            let r16 = DdsxHeader { d3d_format: D3DFMT_R16F, bits_per_pixel: 16, ..header };
            assert_eq!(
                Texture::from_ddsx(DdsxHeader { mem_size: 2, ..r16 }, &0x3800u16.to_le_bytes()).unwrap().to_rgba().unwrap(),
                [128, 128, 128, 255]
            );
        }

        #[test]
        fn blocks_are_decoded() {
            // A DXT1 block whose two colours are red and blue, every pixel using colour 0
            let mut block = Vec::new();
            block.extend_from_slice(&0xF800u16.to_le_bytes());
            block.extend_from_slice(&0x001Fu16.to_le_bytes());
            block.extend_from_slice(&[0; 4]);
            let t = Texture::from_ddsx(dxt1(4, 1, 0, 8), &block).unwrap();
            let rgba = t.to_rgba().unwrap();
            assert_eq!(rgba.len(), 64);
            assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
        }
    }
}
