#![no_main]
use libfuzzer_sys::fuzz_target;
use wt_formats::ddsx::{DdsxHeader, Texture};

fuzz_target!(|data: &[u8]| {
    if let Ok(header) = DdsxHeader::parse(data) {
        let _ = (header.is_empty(), header.compression(), header.format_name());
        let _ = (header.is_cubemap(), header.is_volume(), header.generates_mips());
        if let Ok(texture) = Texture::from_ddsx(header, &data[DdsxHeader::SIZE..]) {
            let _ = (texture.width(), texture.height());
            let _ = texture.to_rgba();
            let _ = texture.to_png();
        }
    }
});
