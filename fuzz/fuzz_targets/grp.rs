#![no_main]
use libfuzzer_sys::fuzz_target;
use wt_formats::grp::GrpPack;

fuzz_target!(|data: &[u8]| {
    if let Ok(pack) = GrpPack::parse(data) {
        for r in &pack.resources {
            let _ = pack.data(r);
            let _ = pack.find(&r.name, r.class_id);
        }
    }
});
