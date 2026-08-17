#![no_main]

//! What hunkpick renders, it must read back as the same thing.
//!
//! The byte-for-byte promise only covers git-canonical input (git omits the `,1` of a
//! single-line range, so `-1,1` comes back as `-1`), and arbitrary bytes are not canonical.
//! The property that does hold for every input is a fixed point one step later: rendering a
//! parsed diff and parsing the result gives the same model back. A parser that quietly drops a
//! line, or a renderer that moves one, breaks it.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(patch) = hunkpick::parser::parse(data) else {
        return;
    };
    let rendered = hunkpick::emit::emit(&patch);
    let reparsed = hunkpick::parser::parse(&rendered).expect("hunkpick must read its own output");
    assert_eq!(reparsed, patch, "parse . emit is not a fixed point");
    assert_eq!(
        hunkpick::emit::emit(&reparsed),
        rendered,
        "rendering twice differs"
    );
});
