# Fuzzing seeds

Starting points for the fuzzer, one directory per target. Unlike `fuzz/corpus/` (machine-local,
gitignored, grown by the fuzzer itself), these are committed: they are where a run begins, not what
it found.

Each file is a small diff covering one shape the parser has to handle — a plain replacement, two
change blocks in one hunk, CRLF endings, a missing final newline, a mail preamble, a rename, a
binary patch, several files, a file deletion, lines after the last hunk. The `selectors` target
reads a diff, a NUL byte and then one selector per line, so its seeds carry both halves.

Passed on the command line alongside the corpus directory:

```sh
cargo fuzz run <target> fuzz/corpus/<target> fuzz/seeds/<target> \
  -- -dict=fuzz/dictionaries/diff.dict
```

New inputs are written to the first directory, so the seeds stay as committed. A crash worth
keeping still becomes a regression test in `tests/`, not a file here.
