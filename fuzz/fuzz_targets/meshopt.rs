#![no_main]
use libfuzzer_sys::fuzz_target;
use wt_formats::meshopt::decode_index_sequence;

// The first two bytes pick the index count and the width; the rest is the packed buffer.
fuzz_target!(|data: &[u8]| {
    let [a, b, rest @ ..] = data else { return };
    let count = usize::from(u16::from_le_bytes([*a, *b]) >> 1);
    let wide = *a & 1 != 0;
    if let Ok(out) = decode_index_sequence(rest, count, wide) {
        assert_eq!(out.len(), count);
    }
    // a count that no input can justify
    let _ = decode_index_sequence(rest, usize::MAX, wide);
});
