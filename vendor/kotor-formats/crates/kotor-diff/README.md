# kotor-diff

Compares BioWare Aurora/Odyssey data files and says what changed.

Two tools needed this and each had grown its own answer. A query tool compares
resources structurally and can patch and merge them. An instruction-file editor
needs the same comparison from the other end: given the file a mod started from
and the file it produced, work out the instructions that turn one into the
other. Same question, same formats, two copies waiting to drift apart.

## The two layers

| Module | What it does | Needs |
| --- | --- | --- |
| `changes` | Writes the TSLPatcher instructions that reproduce a difference | `kotor-formats` |
| `json` | Structural diff, patch and three-way merge over JSON values | serde, behind the `json` feature |

`changes` works on the types `kotor-formats` parses rather than on JSON, and it
has to: an `AddField` section names a field's exact on-disk type, and `Byte` and
`DWORD` are different keywords. A representation that folds every integer width
into one number cannot answer the question.

`json` knows nothing about game formats. It works on whatever a caller has
already turned into JSON, aligning arrays of objects by `_row`, `strref`, `name`
or `Tag` where they have one.

## Sections, not text

`ChangesIni` keeps its output as ordered sections and keys until `render` is
called. An editor with a file already open can therefore merge the generated
sections in through its own writer, instead of pasting a block of text over
whatever formatting the file had.

Token allocation is a separate, opt-in pass. `link_tokens` notices when a
literal in one file matches an index the comparison itself created and rewrites
it as a `2DAMEMORY` reference — which only works because the document is still
structured at that point.

## What it will not do

Some differences have no instruction. A deleted row, a deleted field, a shorter
dialog table: nothing in the format expresses them. Those come back through
`warnings` rather than being silently dropped, so a generated file can say what
it left undone.

## Usage

```rust
use kotor_diff::changes::ChangesIni;

let mut changes = ChangesIni::new();
changes.add_twoda("spells.2da", &before, &after);
changes.link_tokens();

for warning in changes.warnings() {
    eprintln!("cannot express: {warning}");
}
print!("{}", changes.render());
```

Take only the file comparison:

```toml
kotor-diff = { git = "...", tag = "..." }
```

Add the JSON layer:

```toml
kotor-diff = { git = "...", tag = "...", features = ["json"] }
```

## License

MIT. See the [repository root](../../LICENSE).
