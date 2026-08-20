# Fuzzing seeds

Starting points for the fuzzer, one directory per target. Unlike `fuzz/corpus/` (machine-local,
gitignored, grown by the fuzzer itself), these are committed: they are where a run begins, not what
it found.

Each file is a small diff covering one shape the parser has to handle — a plain replacement, two
change blocks in one hunk, CRLF endings, a missing final newline, a mail preamble, a rename, a
binary patch, several files, a file deletion, lines after the last hunk. Three of them carry
bytes above ASCII, for the branches only those reach: a path in git's quoted form with octal
escapes, content that is not valid UTF-8, and a UTF-8 byte-order mark in the preamble. The
`selectors` target reads a diff, a NUL byte and then one selector per line, so its seeds carry
both halves; one addresses a quoted path by its decoded bytes.

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
