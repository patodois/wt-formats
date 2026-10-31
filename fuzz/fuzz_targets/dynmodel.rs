#![no_main]
use libfuzzer_sys::fuzz_target;
use wt_formats::dynmodel::DynModel;

fuzz_target!(|data: &[u8]| {
    if let Ok(model) = DynModel::parse(data) {
        for buffer in model.vdata.iter().chain(&model.skin_vdata) {
            for i in 0..buffer.vertex_count().min(256) {
                let _ = model.vertex(buffer, i);
                let _ = model.skin_weights(buffer, i);
            }
            let _ = model.vertex(buffer, buffer.vertex_count());
            let _ = (buffer.bone_groups(), buffer.has_uv1());
        }
        for lod in &model.lods {
            for skin in &lod.skins {
                for material in &skin.materials {
                    let _ = skin.bone(material.material, 0);
                    let _ = skin.bone(material.material, usize::MAX);
                }
            }
        }
    }
});
