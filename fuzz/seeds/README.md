# Fuzzing seeds

Starting points for the fuzzer. Unlike `fuzz/corpus/` (machine-local, gitignored, grown by the
fuzzer itself), these are committed: they are where a run begins, not what it found.

The split is by input shape, not by target. `diff/` holds plain unified diffs, which is what
both `parse` and `roundtrip` read; they used to be two byte-identical directories, and nothing
kept the copies in agreement. `selectors/` holds the shape only that target takes — a diff, a
NUL byte, then one selector per line. A target reads its own directory when it has one and
`diff/` otherwise, so a new target that takes a plain diff needs no new files.

Each file in `diff/` is a small diff covering one shape the parser has to handle — a plain
replacement, two change blocks in one hunk, CRLF endings, a missing final newline, a mail
preamble, a rename, a binary patch, several files, a file deletion, lines after the last
hunk. Three of them carry
bytes above ASCII, for the branches only those reach: a path in git's quoted form with octal
escapes, content that is not valid UTF-8, and a UTF-8 byte-order mark in the preamble.

The seeds in `selectors/` carry both halves, one per selector form: index, range, several
selectors, `path:N`, `*`, `path:*`, `@L`, and a content id. Two use content ids — one naming a
single sub-hunk, one naming an id two identical sub-hunks share, which is the collision path.
An id is sixteen hex digits, so a mutation will not arrive at a real one: those seeds are the
only way that code is reached, and `tests/fuzz_seeds.rs` checks that each of them still
resolves against its own diff rather than being skipped in silence.

They are passed on the command line alongside the corpus directory, which is what
[`scripts/fuzz-all.sh`](../../scripts/fuzz-all.sh) does:

```sh
scripts/fuzz-all.sh <target>
```

The full command needs a nightly toolchain override, a spelled-out target triple, a corpus
directory created beforehand and a hang timeout; the script carries all four, and
[`CONTRIBUTING.md`](../../CONTRIBUTING.md) explains why each is there.

New inputs are written to the first directory, so the seeds stay as committed. A crash worth
keeping still becomes a regression test in `tests/`, not a file here.
