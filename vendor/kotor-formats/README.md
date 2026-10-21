# kotor-formats

Shared readers and writers for the file formats *Star Wars: Knights of the Old
Republic* I and II keep their data in.

Three separate tools were each carrying their own copy of this code: a mod
installer, a compiled-script compiler, and a query tool. The same byte layouts,
written three times, free to drift apart. These crates are the one copy they
all read from.

## Crates

| Crate | What it holds | Dependencies |
| --- | --- | --- |
| [`kotor-ncs-isa`](crates/kotor-ncs-isa) | The NCS bytecode instruction set, as data | none |
| [`kotor-ncs`](crates/kotor-ncs) | NCS reader/writer and DeNCS-algorithm decompiler | `kotor-ncs-isa` |
| [`kotor-formats`](crates/kotor-formats) | GFF, 2DA, TLK, SSF, ERF/RIM readers and writers | none |
| [`kotor-diff`](crates/kotor-diff) | Compares two files and says what changed | `kotor-formats` |

They are split so a consumer takes only what it needs. A script compiler can
take the instruction set alone, or `kotor-ncs` when it also needs to read
bytecode and decompile it. A patcher takes the structured formats and not the
bytecode. Neither side has to depend on the other.

`kotor-diff` answers one question from two ends. A query tool compares resources
structurally and can patch and merge them; an instruction-file editor needs the
same comparison to work out what a mod did, so it can write the instructions
that reproduce it. Two tools had each grown their own copy. Its JSON layer sits
behind a feature, so a consumer that only wants the file comparison does not
pull in serde.

## Taking more than one crate

Cargo resolves a git source once, so every dependency taken from one repository
has to name the same ref. A consumer cannot pin each crate separately.

So pin one tag and take every crate from it. A tag is a point in the whole
repository, so whichever you name carries all three crates as they stood at that
release. `vX.Y.Z` is the one to prefer; the per-crate tags name the same commits
and exist so each changelog has somewhere to point.

```toml
kotor-formats = { git = "https://github.com/kolto-labs/kotor-formats", tag = "v0.1.0" }
kotor-diff = { git = "https://github.com/kolto-labs/kotor-formats", tag = "v0.1.0" }
```

The crates keep their own version numbers, which do not have to match — what
matters is the ref, and a release publishes every crate at the same one.

Nothing here is on crates.io, so the crates do not name each other by version —
only by path. A version requirement would be one more thing to keep in step with
every release, and when it fell behind it broke the published tag rather than
anything a test would catch.

## Design

**Round trips are exact.** Loading a file and saving it back without edits
produces the same bytes. Field type tags, field-data widths, raw label bytes,
and the padding a file happened to carry are all preserved rather than
normalized away. A tool that rewrites a file it did not mean to change is a
tool that corrupts saves.

**Text is stored losslessly.** Bytes decode through a total, reversible
Latin-1 mapping, so `encode(decode(x)) == x` holds for all 256 values. Real
CP1252 cannot promise that — five of its byte values are undefined, and any
character outside its repertoire has nowhere to go. Presentation is a separate
concern: a display layer can remap the 0x80–0x9F range for output, as long as
that mapping never reaches a write path.

**Strictness is the default, leniency is opt-in.** `GffFile::parse` accepts
what the original Delphi TSLPatcher accepted, so a patcher built on this crate
rejects the same files it always did. `parse_with` and `GffParseOptions` let a
read-only tool accept more — V3.3 headers, `StrRef` fields — without changing
what anyone else sees.

**Errors carry a subsystem and a code**, rendering as `(GFF-1)` or `(2DA-8)`.
That is the form mod authors have been reading in install logs for twenty
years, and it is preserved deliberately.

## Status

Early. The API is not stable yet, and `kotor-formats` is not published to
crates.io — depend on it by git tag or path.

## License

MIT. See [LICENSE](LICENSE).
