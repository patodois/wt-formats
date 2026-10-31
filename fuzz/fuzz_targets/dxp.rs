#![no_main]
use libfuzzer_sys::fuzz_target;
use wt_formats::dxp::DxpPack;

fuzz_target!(|data: &[u8]| {
    if let Ok(pack) = DxpPack::parse(data) {
        for i in 0..pack.entries.len().min(64) {
            if let Ok(texture) = pack.texture(i) {
                let _ = texture.to_rgba();
                let _ = texture.to_png();
            }
        }
        let _ = pack.texture(pack.entries.len());
    }
});
