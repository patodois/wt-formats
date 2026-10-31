#![no_main]
use libfuzzer_sys::fuzz_target;
use wt_formats::geom_node_tree::GeomNodeTree;

fuzz_target!(|data: &[u8]| {
    if let Ok(tree) = GeomNodeTree::parse(data) {
        for node in &tree.nodes {
            let _ = tree.find(&node.name);
        }
    }
});
