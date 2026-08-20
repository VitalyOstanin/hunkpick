//! Every byte hunkpick can print has to be ASCII.
//!
//! The release pipeline ships an `x86_64-pc-windows-msvc` binary, and Rust writes stdout and
//! stderr as UTF-8 whatever the console is set to. A Windows console on cp866 or cp1251 — the
//! defaults in several locales — renders anything above ASCII as mojibake in the middle of a
//! sentence. `every_help_text_is_ascii` in `src/cli.rs` guards the help; these guard the rest,
//! because the character that guard was written for came back the same week in an error message.

/// Where a non-ASCII character sits in a source file, for the failure message.
struct Offender {
    file: String,
    line: usize,
    text: String,
    ch: char,
}

/// Every string literal of `src`, outside the `#[cfg(test)]` module at the end of each file.
///
/// Comments are excluded on purpose: they are read in an editor, not in a console, and the
/// repository writes them with typographic punctuation throughout. Test modules are excluded
/// because their fixtures stand for real input, which is not ASCII-only — a path such as
/// `é.txt` is exactly what the parser has to carry through.
fn non_ascii_string_literals() -> Vec<Offender> {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir(&src)
        .expect("src/ is part of the published crate")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.sort();
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // A literal left open at the end of a line continues on the next one: the messages this
        // test exists for are written that way, split across lines with a trailing backslash.
        let mut in_string = false;
        for (no, line) in text.lines().enumerate() {
            // The test module runs to the end of the file in every source here, so the first
            // marker ends the scan of this file.
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            for ch in string_literal_chars(line, &mut in_string) {
                if !ch.is_ascii() {
                    found.push(Offender {
                        file: name.clone(),
                        line: no + 1,
                        text: line.trim().to_string(),
                        ch,
                    });
                }
            }
        }
    }
    found
}

/// The characters inside string literals of one line of Rust.
///
/// A line scanner rather than a parser: it needs to tell a `"` that opens a literal from one
/// written in a comment or in a `b'"'` character literal, and that is all. `in_string` carries
/// an open literal from one line into the next. The sources hold no raw strings and no block
/// comments; a scan that met one would report its contents as literal text, which errs towards a
/// failing test rather than a silent pass.
fn string_literal_chars(line: &str, in_string: &mut bool) -> Vec<char> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if *in_string {
            match c {
                '\\' => i += 1, // the escaped character cannot end the literal
                '"' => *in_string = false,
                _ => out.push(c),
            }
            i += 1;
            continue;
        }
        match c {
            '/' if chars.get(i + 1) == Some(&'/') => break,
            '"' => *in_string = true,
            // A character literal: `'x'`, `'\n'`, `b'"'`. Skipped whole, so the quote inside one
            // does not read as the start of a string. A lifetime (`'a`) has no closing quote
            // within the next few characters and falls through to the plain advance below.
            '\'' => {
                if let Some(end) = char_literal_end(&chars, i) {
                    i = end;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The index of the closing quote of the character literal starting at `open`, if this is one.
fn char_literal_end(chars: &[char], open: usize) -> Option<usize> {
    let end = if chars.get(open + 1) == Some(&'\\') {
        // `'\n'`, `'\\'`, `'\u{2014}'`: scan to the quote that closes it.
        (open + 2..chars.len().min(open + 12)).find(|&i| chars[i] == '\'')?
    } else {
        open + 2
    };
    (chars.get(end) == Some(&'\'')).then_some(end)
}

/// No string literal outside the test modules holds a character above ASCII. This is the guard
/// the help-text test could not give: a message assembled in `main.rs` never goes through clap.
#[test]
fn no_source_string_literal_leaves_ascii() {
    let offenders = non_ascii_string_literals();
    let report: Vec<String> = offenders
        .iter()
        .map(|o| format!("src/{}:{} {:?} in: {}", o.file, o.line, o.ch, o.text))
        .collect();
    assert!(
        offenders.is_empty(),
        "non-ASCII in {} string literal(s); use ASCII punctuation (`--`, `\"`, `...`):\n{}",
        offenders.len(),
        report.join("\n")
    );
}
