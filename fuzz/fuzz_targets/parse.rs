#![no_main]

//! Parsing arbitrary bytes must fail as a `ParseError`, never panic.
//!
//! hunkpick reads whatever a caller pipes into it — a truncated diff, a mail, a binary file
//! named by mistake. A panic there is exit code 101 and a backtrace where the contract promises
//! exit code 2 and a sentence.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = hunkpick::parser::parse(data);
});
