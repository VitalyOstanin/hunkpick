#![no_main]

//! Selector parsing and selection must fail as usage errors, never panic — and a selection
//! that succeeds must pass the checks hunkpick applies to its own output.
//!
//! The input is split into a diff and a selector list at the first NUL byte, so the fuzzer can
//! vary both sides. Indexing into sub-hunks, cutting a sub-hunk down to a line set and the
//! anchor arithmetic all read numbers the caller supplied; this is where an out-of-range index
//! or an overflow would show up as a panic.

use libfuzzer_sys::fuzz_target;
use std::ffi::OsString;

/// How many selector lines are taken from the input. A selection is resolved per selector, so an
/// unbounded list turns a short input into a long run and starves the fuzzer of iterations; eight
/// is past the point where another selector exercises new code.
const MAX_SELECTORS: usize = 8;

fuzz_target!(|data: &[u8]| {
    let (diff, rest) = match data.iter().position(|&b| b == 0) {
        Some(i) => (&data[..i], &data[i + 1..]),
        None => (data, &[][..]),
    };
    let Ok(patch) = hunkpick::parser::parse(diff) else {
        return;
    };
    // The CLI rejects an inconsistent input diff as a usage error before it selects anything
    // (`load_and_parse` in main.rs): counts that disagree with the body are a defect of the
    // input, not of hunkpick. Selection carries that disagreement into its result, so the
    // output checks below only hold for input the CLI would have accepted.
    if hunkpick::validate::validate_input(&patch).is_err() {
        return;
    }
    let args: Vec<OsString> = rest
        .split(|&b| b == b'\n')
        .take(MAX_SELECTORS)
        .map(os_string)
        .collect();
    let Ok(selectors) = hunkpick::select::parse_selectors(&args) else {
        return;
    };
    let Ok(result) = hunkpick::select::select(&patch, &selectors) else {
        return;
    };
    // A successful selection is a diff hunkpick stands behind: it has to pass the check that
    // guards its own output, and be acceptable as input to the next invocation.
    hunkpick::validate::validate_internal(&result).expect("a selection must be self-consistent");
    hunkpick::validate::validate_input(&result).expect("a selection must be valid input");
});

#[cfg(unix)]
fn os_string(bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec())
}

#[cfg(not(unix))]
fn os_string(bytes: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(bytes).into_owned())
}
