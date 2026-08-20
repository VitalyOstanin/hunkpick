#![no_main]

//! Selector parsing and selection must fail as usage errors, never panic — and a selection
//! that succeeds must pass the checks hunkpick applies to its own output.
//!
//! The input is split into a diff and a selector list at the first NUL byte, so the fuzzer can
//! vary both sides. Indexing into sub-hunks, cutting a sub-hunk down to a line set and the
//! anchor arithmetic all read numbers the caller supplied; this is where an out-of-range index
//! or an overflow would show up as a panic.

use hunkpick::model::{FileContent, LineKind, Patch};
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
    let Ok(mut patch) = hunkpick::parser::parse(diff) else {
        return;
    };
    // The CLI rejects an inconsistent input diff as a usage error before it selects anything
    // (`load_and_parse` in main.rs): counts that disagree with the body are a defect of the
    // input, not of hunkpick. Selection carries that disagreement into its result, so the
    // output checks below only hold for input the CLI would have accepted.
    //
    // Skipping such an input would cost most of the stream rather than a rare degenerate case:
    // the mutations libFuzzer favours — insert a fragment, delete one, duplicate a line — break
    // the header-to-body correspondence more often than not. So the counts are recomputed from
    // the body first, and only what that does not fix (overlapping hunks, a hunk starting at
    // line 0) is skipped. Every mutation of a body then stays a usable input, and the code under
    // test is the selection, not the accident of the counts surviving.
    recount_hunk_headers(&mut patch);
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

/// Recompute every hunk header's line counts from its body, leaving the start positions as the
/// input had them. The mirror of what `validate_input` checks, applied instead of asserted.
fn recount_hunk_headers(patch: &mut Patch) {
    for file in &mut patch.files {
        let FileContent::Text(hunks) = &mut file.content else {
            continue;
        };
        for hunk in hunks {
            hunk.old_lines = count(hunk.lines.iter().map(|l| l.kind), LineKind::Add);
            hunk.new_lines = count(hunk.lines.iter().map(|l| l.kind), LineKind::Del);
        }
    }
}

/// How many lines the side that does not carry `absent` covers: context plus its own changes.
fn count(kinds: impl Iterator<Item = LineKind>, absent: LineKind) -> u32 {
    kinds.filter(|k| *k != absent).count() as u32
}
