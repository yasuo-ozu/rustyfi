<p align="center">
  <img src="https://raw.githubusercontent.com/yasuo-ozu/rustyfi/main/manual/logo.png" width="160" alt="rustyfi logo: a gear with the word rustyfi engraved between braces">
</p>

# rustyfi [![CI]][ci-workflow] [![Release]][releases] [![SATySFi]][upstream] [![License]][license]

[CI]: https://github.com/yasuo-ozu/rustyfi/actions/workflows/ci.yml/badge.svg
[ci-workflow]: https://github.com/yasuo-ozu/rustyfi/actions/workflows/ci.yml
[Release]: https://img.shields.io/github/v/release/yasuo-ozu/rustyfi?label=release&color=blue
[releases]: https://github.com/yasuo-ozu/rustyfi/releases
[SATySFi]: https://img.shields.io/badge/SATySFi-0.0%20%C2%B7%200.1-orange
[upstream]: https://github.com/gfngfn/SATySFi
[License]: https://img.shields.io/badge/license-MIT-blue.svg
[license]: #license

**[SATySFi](https://github.com/gfngfn/SATySFi), reimplemented in Rust.** One
binary takes a `.saty` document and writes a PDF — same language, same packages,
same output, no OCaml toolchain to install.

It speaks both dialects: **0.0** (upstream v0.0.6) and **0.1**
(`dev-0-1-0`/`saphe-split`), and a document in one may use packages from the
other.

## Install

Take an archive for your platform from the [releases page][releases] and unpack
it into a prefix — `~/.local` for yourself, `/usr/local` for everyone:

```console
$ shasum -a 256 -c rustyfi-<tag>-x86_64-unknown-linux-gnu.tar.gz.sha256
$ tar -xzf rustyfi-<tag>-x86_64-unknown-linux-gnu.tar.gz --strip-components=1 -C ~/.local
$ rustyfi --version
```

### From source

```console
$ git clone https://github.com/yasuo-ozu/rustyfi && cd rustyfi
$ cargo build --release --bin rustyfi
$ sh scripts/download-fonts.sh      # IPAex, Junicode, Latin Modern — pinned, ~175 MB
```

## Compile a document

```satysfi
@require: stdja-mini

document (|
  title = {Milestone One};
  author = {yasuo};
|) '<
  +p { Hello, world! This is \emph{SATySFi-in-Rust}. }
>
```

```console
$ rustyfi doc.saty
  output written on doc.pdf (1 page(s), 2 line(s)).
```

Recompiling an unchanged document is near-instant: results are cached by content
hash (`--no-cache` opts out).

## Packages

`@require:` resolves against **lib roots** (`<root>/dist/packages/`). Name one
with `--lib-root` or `$RUSTYFI_LIB_ROOT` and it is used alone; name none and
they are discovered, nearest first:

1. `lib-rustyfi/` above your document — a checked-out source tree
2. `.rustyfi/` beside a `Satyristes` — a project-local install
3. `<exe>/../lib/rustyfi` — the install this binary belongs to
4. `~/.local/lib/rustyfi`, then `/usr/local/lib/rustyfi` and `/usr/lib/rustyfi`

All of them are searched in that order, so a package a project installed for
itself layers over the system one rather than hiding it — and a clone needs no
configuration at all.

Roughly 30 upstream packages ship with it, including `stdja`, `stdjabook`,
`stdjareport`, `itemize`, `code`, `math`, `tabular`, `annot` and `proof`, plus
the 0.1 tree (`std-ja`, `inline`, `block`, `map`, `set`, …) under `dist-v01/`.

To install someone else's package, the same binary is a Satyrographos analog:

```console
$ rustyfi search font theano       # keywords narrow: every one must match
$ rustyfi install ./satysfi-xpath  # a local path, a .tar.gz, or a registry name
$ rustyfi install xpath easytable  # install/uninstall take several at once
$ rustyfi install https://example.org/pkg.tar.gz#sha256=…   # or a URL
$ rustyfi list
```

The default repository can live in your own config, so `search` and `install`
work outside any project:

```toml
# ~/.config/rustyfi/config.toml
[[registry]]
url = "https://github.com/na4zagin3/satyrographos-repo"

[[registry]]
url = "https://example.org/another-index"
```

`search` covers every repository listed and labels each hit; `install NAME`
tries them in order and takes the first that has the package.

Archives ship their own `share/rustyfi/config.toml`, which the binary finds
relative to itself (`<exe>/../share/rustyfi`) — as it finds its packages at
`<exe>/../lib/rustyfi` — so an unpacked archive is self-contained wherever it
sits. Precedence, lowest last: `--registry`, `$RUSTYFI_REGISTRY`, the project's
own `(registry (url …))` in `Satyristes`, your config, the shipped one.

It reads upstream `Satyristes` manifests, keeps a project lockfile, and verifies
registry downloads by sha256. A manifest's `(libraryDoc …)` targets — documents
built *from* a library — are built by running their own declared commands:

```console
$ rustyfi build                    # or --doc NAME, when several are declared
```

## Language versions

SATySFi comes in two incompatible generations and this handles both. `0.0`
(0.0.6) is the default; `--lang 0.1` selects the newer one, and a 0.1-style
`use` header selects it on its own:

```console
$ rustyfi doc.saty              # 0.0.6
$ rustyfi --lang 0.1 doc.saty   # 0.1
```

Packages carry a generation too. A `Satyristes` says which one each library is
written for, and one manifest may declare the same name for both; `--lang` picks
among what it declares, and is only needed to disambiguate.

```lisp
(library (name "greet") (version "1.0") (lang 0.1)
  (sources ((packageDir "src"))))
```

```console
$ rustyfi install ./greet --lang 0.1
installed greet 1.0 (1 path(s)):
  dist-v01/packages/greet
```

0.1 packages live in `<root>/dist-v01/packages/`, 0.0 ones in
`<root>/dist/packages/`, and both can be installed side by side under one name.
`@require:` prefers your document's own generation and falls back to the other —
which matters, because names like `itemize`, `list` and `code` exist in both
corpora with genuinely different APIs, and you get the one written for the
language you are compiling.

That fallback is also what lets a **0.1 document `@require:` a 0.0.6 package**,
which works end to end. The limit is types whose runtime representation forks
between generations: `page`, `font`, `math-text` and `math-boxes` are refused
with an error naming the type rather than quietly mis-rendered, `math` is
relabelled to `math-text` for you, and `deco`/`deco-set`/`paren` cross through
generated wrappers. The reverse direction is partial — see
[Known gaps](#known-gaps).

## Staging

A program runs in two stages. **Stage 1** is the document stage: it produces the
PDF, and it is where a `.saty` file's own code lives. **Stage 0** runs before any
of that, and its job is to build stage-1 code rather than to typeset anything.

Two prefixes move between them, and they are inverses. `&e` **quotes**: it does
not run `e`, it yields a value standing for it. `~e` **splices**: it runs `e` one
stage earlier and inserts the code value that comes back into the program at
that point. `~(&e)` is `e`. A quote is legal only at stage 0 and a splice only at
stage 1, which is why the macro below needs a stage of its own to live at.

```satysfi
% macros.satyh
@stage: 0

let twice c = &( ~c ^ ~c )
```

```satysfi
% doc.saty
@require: stdja-mini
@import: macros

let s = ~(twice &(`ab`)) in
document (|
  title = {Staging};
  author = {yasuo};
|) '<
  +p(embed-string s);
>
```

```console
$ rustyfi doc.saty
  output written on doc.pdf (1 page(s), 2 line(s)).
```

The page reads `abab`. `twice` ran before the document did and leaves no trace in
it; what the document evaluates is `` `ab` ^ `ab` ``.

A file says which stage it is written at differently in each generation:

- **0.0.6** — one header for the whole file: `@stage: 0`, `@stage: 1` (the
  default, so documents need no header) or `@stage: persistent`. `persistent` is
  its own stage, nameable from both of the others; upstream's `list.satyg` and
  `option.satyg` are written at it, which is what lets a document call `List.map`
  at all.
- **0.1** — no header, one qualifier per binding: `val ~x = e` is stage 0, `val
  persistent ~x = e` is persistent, a plain `val x = e` is stage 1. It goes in
  front of every binding shape — `val ~rec`, `val ~mutable`, `val ~inline`, `val
  ~block`, `val ~math`.

```satysfi
% macros.satyh — the same macro, 0.1
module Macro :> sig
  val ~twice : code string -> code string
end = struct
  val ~twice c = &( ~c ^ ~c )
end
```

The document splices it the same way, writing ``~(Macro.twice &(`ab`))``:

```console
$ rustyfi --lang 0.1 doc.saty
  output written on doc.pdf (1 page(s), 2 line(s)).
```

`code τ` is the type `&` produces, and it is a **0.1 spelling only** — 0.0.6 has
none, deliberately: upstream's 0.0.6 type decoder knows `list` and `ref` and
nothing else, so `int code` there is an undefined type name and stays one here. A
0.1 signature declares its member's stage as well as its type, and the `struct`
has to provide both.

Staging crosses the generation boundary, too: a `@stage: 0` 0.0.6 library's
macro is usable from a 0.1 document, and the other way round, with each side
keeping its own stage rules. A quote keeps the generation it was **written**
in, whichever generation forces it — a `&` inside a 0.0.6 package still means
0.0.6's primitives when a 0.1 document splices it, so a macro cannot change
meaning by being imported. The one thing refused is a 0.0.6 package that writes
`code` in a `type` declaration: since 0.0.6 has no such spelling, that text
would quietly become 0.1's real `code` type on the way in, so it errors instead
of changing meaning.

The rule that catches people out: **a stage-0 binding cannot be named from stage
1.** Only `persistent` crosses. So the reference has to sit inside a splice —
`~(twice …)`, never `twice …`:

```console
$ rustyfi doc.saty
Error: doc.saty: line 4, characters 8-13: invalid occurrence of variable 'twice' as to stage: it is bound at stage 0, but this is stage 1
```

The same rule applies *inside* a quote, which is the surprising half: a quote's
body is one stage later, so a stage-0 parameter is out of scope there. `let twice
c = &( c ^ c )` is refused for naming `c`; `&( ~c ^ ~c )` is what you meant,
because `~c` reads `c` back at stage 0.

One deviation from upstream. Upstream resolves every splice in a preprocessing
pass, before any stage-1 code runs, so all splices happen first and in file
order; here a splice runs where it stands. For the pure code-building staging is
for, the value is identical — the two differ only for side effects interleaved
between a splice and the stage-1 code around it.

## HTML output

```console
$ rustyfi doc.saty --format html         # every glyph where the PDF puts it
$ rustyfi doc.saty --format html-reflow  # real flowing paragraphs, CSS layout
```

`html` is the same laid-out page the PDF writer renders, serialized with
absolute positions and the real fonts embedded — a preview and visual-diff aid.
`html-reflow` is the opposite trade: semantic, reflowable, not layout-faithful.

## Useful options

| flag | what it does |
|---|---|
| `-o <path>` | output path (default: the input with a `.pdf` extension) |
| `--format <fmt>` | `pdf` (default), `html`, `html-reflow` |
| `--lib-root <dir>` | where `@require:` looks for packages |
| `--lang <v>` | `0.0` (default) or `0.1`; a `use` header auto-selects `0.1` |
| `--font <file>` | use a TrueType/OpenType file as the regular face |
| `--font-dir <dir>` | font root holding `dist/hash/fonts.satysfi-hash` |
| `--no-cache` | bypass the compile cache |
| `--no-aux` | do not read or write the `.satysfi-aux` cross-reference file |
| `--timing` | per-phase timing to stderr (load / typecheck / eval / render) |

The `.satysfi-aux` file is upstream's format, so the two engines can share one.

## How close is it?

Every document in the vendored corpus is rebuilt and compared against the PDF
the original SATySFi produced, word box by word box
(`scripts/layout_fidelity.py`):

| doc | pages (port / SATySFi) | words in the same place | exercises |
|---|---|---|---|
| latexcmds | 12 / 12 | 89.5 % | math, framed and coloured boxes |
| xpath | 11 / 11 | 96.7 % | paths, béziers, diagrams |
| enumitem | 27 / 27 | 88.9 % | deeply nested, customized lists |
| easytable | 19 / 19 | 86.8 % | tables, rules, spans |
| figbox | 20 / **21** | 88.1 % | figures, floats, captions |
| slydifi | 30 / 30 | 85.1 % | slides, overlays, themes |

Glyph metrics agree to within 0.75 pt at the 95th percentile, so what is left is
the line breaker's own judgement — and one missing page in `figbox`.

It is also faster. Minimum CPU time over three interleaved runs against SATySFi
0.0.11 (`--bytecomp` is upstream's bytecode compiler, the fair comparison for
the evaluator; `scripts/benchmark.py` reproduces it):

| doc | SATySFi | SATySFi `--bytecomp` | rustyfi | cached |
|---|---|---|---|---|
| latexcmds | 1.38 s | 1.34 s | **0.48 s** | 0.32 s |
| xpath | 12.66 s | 3.33 s | 4.04 s | 0.38 s |
| enumitem | 3.18 s | 3.12 s | **1.27 s** | 0.42 s |
| easytable | 3.63 s | 3.56 s | **1.61 s** | 0.46 s |
| figbox | 3.26 s | 3.07 s | **1.86 s** | 0.51 s |
| slydifi | 2.26 s | 1.75 s | **1.21 s** | 0.44 s |

`xpath` is the one loss, and it is the one document dominated by user-level
arithmetic rather than layout: it measures interpreter against VM. Against
upstream's default (non-bytecode) interpreter it is still 3.1× faster.

## Known gaps

- `figbox` comes out one page short of upstream — a line-packing difference.
- Fonts are named by file or hash entry, not by package: a document asking for
  `fonts-junicode:Junicode-Bold` falls back to a name heuristic.
- Cross-version `deco` crosses both ways now, including through optional
  arguments and nested module signatures — but not through an *open* optional
  row (nothing names the labels to forward) or a functor signature member.
- `font` and 0.1's `paren` are stand-in types, so neither crosses generations.
- A 0.0.6 package that WRITES `code` in a `type` declaration is refused rather
  than crossing: 0.0.6 has no `code` spelling, so that text would silently
  acquire 0.1's meaning on the way in. Ordinary staged exports — the inferred
  kind, from `&e` — are unaffected and cross both ways.

## The manual

The manual is written in SATySFi and typeset by the port itself, so every
feature it uses is one that has to keep working.

- [manual.pdf](https://raw.githubusercontent.com/yasuo-ozu/rustyfi/main/manual/manual.pdf)
  · [source](https://raw.githubusercontent.com/yasuo-ozu/rustyfi/main/manual/manual.saty)
- [`manual/logo.saty`](https://github.com/yasuo-ozu/rustyfi/blob/main/manual/logo.saty)
  — the logo above is not an image file but a document, drawn entirely with
  `satysfi-xpath` ([notes](https://github.com/yasuo-ozu/rustyfi/blob/main/manual/logo.md))

```console
$ make -C manual        # manual.pdf, logo.pdf, logo.png
```

## Development

```text
crates/
  rustyfi-syntax/         mode-stack lexer and grammar (CST) for both dialects
  rustyfi-lang/           elaboration, typechecker, evaluator, primitives
  rustyfi-backend/        boxes and glue, line and page breaking, math
  rustyfi-loader/         @require/@import resolution and load order
  rustyfi-pdf/            PDF writer, font embedding
  rustyfi-html/           the two HTML backends
  rustyfi-satyrographos/  package manager
  rustyfi/                the binary
lib-rustyfi/              bundled packages: dist/ (0.0) and dist-v01/ (0.1)
scripts/                  fidelity and benchmark harnesses, font fetcher
```

The grammar is derived, not hand-written, using the
[`syan`](https://crates.io/crates/syan) parser framework: the CST types *are*
the grammar. `cargo test --workspace` runs 1745 tests; CI adds the corpus
regression and the layout-fidelity comparison above.

## License

MIT — see [LICENSE](LICENSE).

Two sets of files bundled here are not covered by it and keep their own terms.
The fonts `scripts/download-fonts.sh` fetches carry the IPA Font License v1.0,
SIL OFL 1.1, the GUST Font License and DejaVu's, each copied next to the font it
covers. The SATySFi packages under `lib-rustyfi/` are upstream's, LGPL-3.0.
