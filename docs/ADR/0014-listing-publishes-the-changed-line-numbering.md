# ADR 0014 — The listing publishes the changed-line numbering, and narrows itself

Date: 2026-09-13

## Status

Accepted

## Context

[ADR 0009](0009-changed-line-addressing-supersedes-range.md) made `[path:]INDEX@L<set>` the sole
per-line cutter: the set numbers a sub-hunk's changed (`+`/`-`) lines `1..N` in body order. The
numbering was published in one place only — `changed_lines` in `list --json`. The human listing
showed the index, the content id, the `@@` header, the `+N -M` counts and a preview of the first
changed line: enough to pick a whole sub-hunk, not enough to cut one.

So the moment a cut was actually needed, the workflow left the tool:

```sh
git diff -- path | hunkpick list --json \
  | jq -r '.[0].hunks[7].changed_lines[] | "\(.i) \(.kind) \(.text)"'
```

That is three things to get right — the file's position in the array, the sub-hunk's position,
the field names — for a question the tool already has the answer to, plus a dependency on `jq`
that no other step of the pipeline has. The case that forces it is narrow but real: a sub-hunk
that merges two adjacent but unrelated changes (a constant replaced, another added right below
it, the two belonging to different commits). Auto-split cannot separate them — they are one
contiguous change run — and that is precisely what `@L` exists for.

Two shapes were weighed: a flag on `list`, and a separate `show` command printing the same
detail for addressed sub-hunks only. A separate command would keep the listing untouched at the
cost of a second listing command to document, to teach, and to keep in step with `list`'s
escaping and colour rules.

## Decision

The human listing publishes the numbering, and the listing can be narrowed:

- **`list --lines`** prints each sub-hunk's changed lines under its header line: the 1-based
  index within the sub-hunk, the kind (`+` / `-`), and the text. The numbering comes from
  `Hunk::changed_lines`, the same source `changed_lines` and the `@L` cut read, so what is
  printed is what a selector takes — with no translation step in between. The detail is escaped
  exactly as the rest of the human listing is (escape sequences, control bytes, bidirectional
  overrides), and stays lossy for non-UTF-8 content, which is addressed by content id. It is not
  accepted with `--json`, which already carries this.
- **`list --only <selector>`** lists only the sub-hunks that selector addresses, reading the
  grammar `select` reads, with one exception: `INDEX@L<set>` addresses changed lines within a
  sub-hunk rather than a sub-hunk to list, and is a usage error here. The flag takes one value
  and is repeated to name several, rather than consuming every following argument: a greedy flag
  reads a mistyped argument as another selector. A selector that matches nothing is a usage
  error, not an empty listing — including an index on a binary entry, which has no sub-hunks;
  `select` keeps taking such an entry whole, because the binary change is what it emits. It
  applies to both output forms. `select::resolve_subhunk_filter` resolves the selectors, so the
  listing and `select` cannot drift apart on what a selector means, and a property test compares
  what a selector lists with what the same selector stages.
- **A narrowed listing renumbers nothing.** Sub-hunks left out are skipped, not renumbered;
  `index` keeps numbering the file's sub-hunks and `id_count` keeps counting the whole patch. A
  selector read off a narrowed listing therefore addresses the same sub-hunk as one read off the
  full listing.
- **A sub-hunk is printed whole**, with no elision for a long one: a long sub-hunk is exactly
  where an `@L` cut is needed, and hiding its middle would hide the numbers the caller came for.
- **The detail is not coloured.** The kind stands in a column of its own, and the listing's
  colour marks the index and the preview, where nothing else distinguishes them.

`--json` is unchanged: `changed_lines` already carried this data, and the schema stays a stable
contract for machine consumers.

The library signatures follow: `list::list_human` and `list::list_json` both take a
`list::ListOptions` (colour, the per-line detail, the filter) instead of a bare colour flag. A
later listing option changes the options struct, not either signature; the JSON listing reads
only the filter.

## Consequences

- A cut no longer leaves the tool: `list --lines --only N` answers "which line is which number"
  in the pipeline's own vocabulary, and `jq` stays optional.
- One listing command, with the escaping, colour and lossiness rules stated once.
- The human listing grows a mode whose output is longer than one line per sub-hunk. Callers
  parsing the default listing by line are unaffected — the detail appears only under `--lines`.
- `--only` must keep reading exactly what `select` reads. It does so by construction (one
  resolver, one grammar); the `@L` exception is refused explicitly rather than silently widened
  to the whole sub-hunk, which would list something the caller did not address.
- Breaking for library callers of `list_human` / `list_json`, released as a minor version while
  the crate is pre-1.0.
