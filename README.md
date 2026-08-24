<p align="center">
  <img src="https://raw.githubusercontent.com/yasuo-ozu/rustyfi/refs/heads/main/manual/logo.png" width="160" alt="rustyfi logo: a gear with the word rustyfi engraved between braces">
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
same output, faster compilation, no OCaml toolchain to install.

It speaks both dialects: **0.0** (upstream v0.0.x) and **0.1** (`dev-0-1-0`),
and a document in one may use packages from the other.

<p align="center">
  <a href="https://yasuo-ozu.github.io/rustyfi-playground/">
    <img src="https://img.shields.io/badge/%E2%96%B6%20Playground-typeset%20in%20your%20browser-7a4fd6?style=for-the-badge" alt="Open the rustyfi playground">
  </a>
  &nbsp;
  <a href="https://yasuo-ozu.github.io/rustyfi-packages/">
    <img src="https://img.shields.io/badge/%F0%9F%93%A6%20Packages-browse%20the%20index-1f7a44?style=for-the-badge" alt="Browse the rustyfi package index">
  </a>
</p>

<p align="center">
  <sub>
    The <a href="https://yasuo-ozu.github.io/rustyfi-playground/">playground</a> runs this
    typesetter compiled to WebAssembly — your document never leaves the tab, and there is no
    server. The <a href="https://yasuo-ozu.github.io/rustyfi-packages/">package index</a> is a
    static mirror of Satyrographos' repository that <code>rustyfi install</code> can read
    directly.
  </sub>
</p>

## Install

### From latest release

```console
$ # User-wide installation (~/.local/)
$ curl -fsSL https://raw.githubusercontent.com/yasuo-ozu/rustyfi/main/install.sh | bash

$ # System-wide installation (/usr/*)
$ curl -fsSL https://raw.githubusercontent.com/yasuo-ozu/rustyfi/main/install.sh | sudo bash

$ # Or manual prefix
$ curl -fsSL https://raw.githubusercontent.com/yasuo-ozu/rustyfi/main/install.sh | sudo bash -s -- --prefix /opt/rustyfi
$ curl -fsSL https://raw.githubusercontent.com/yasuo-ozu/rustyfi/main/install.sh | PREFIX=/opt/rustyfi sudo bash

$ rustyfi --version
```

### From source

`install.sh` doubles as the installer for a checkout — run inside one, it uses
the binary you just built instead of downloading anything:

```console
$ git clone https://github.com/yasuo-ozu/rustyfi && cd rustyfi
$ cargo build --release --bin rustyfi
$ sh download-fonts.sh      # IPAex, Junicode, Latin Modern — pinned, ~175 MB
$ ./install.sh                      # --prefix DIR to put it elsewhere
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

### A project with a `Satyristes`

Describe the project in a `Satyristes` (no opam file is required by rustyfi):

```lisp
(version 0.0.2)

(library
  (name    "mylib")
  (version "0.1.0")
  (sources ((packageDir "src"))))

(libraryDoc
  (name             "mylib-doc")
  (version          "0.1.0")
  (workingDirectory "doc")
  (build            ((rustyfi "manual.saty")))
  (sources          ((doc "manual.pdf" "doc/manual.pdf"))))
```

`(library …)` says what the project *publishes*: `(packageDir "src")` installs
every `.satyh`/`.satyg` under `src/` as the package `mylib`. Install it into a
project-local root (`.rustyfi/`) and it becomes `@require:`-able:

```console
$ rustyfi install . --dest .rustyfi
installed mylib 0.1.0 (1 path(s)):
  dist/packages/mylib

$ rustyfi list --dest .rustyfi
mylib 0.1.0 (lang 0.0, 1 files)
  .rustyfi/dist/packages/mylib
```

`(libraryDoc …)` is a build target rather than a package: `rustyfi build` runs
its `(build …)` commands in `(workingDirectory …)`, then installs what
`(sources …)` names.

```console
$ rustyfi build
  rustyfi manual.saty
```

## Package management

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

## Useful options

| flag | what it does |
|---|---|
| `-o <path>` | output path (default: the input with a `.pdf` extension) |
| `--format <fmt>` | `pdf` (default) or `html` — see [HTML output](#html-output) |
| `--lib-root <dir>` | where `@require:` looks for packages |
| `--lang <v>` | `0.0` (default) or `0.1`; a `use` header auto-selects `0.1` |
| `--font <file>` | use a TrueType/OpenType file as the regular face |
| `--font-dir <dir>` | font root holding `dist/hash/fonts.satysfi-hash` |
| `--no-cache` | bypass the compile cache |
| `--no-aux` | do not read or write the `.satysfi-aux` cross-reference file |
| `--timing` | per-phase timing to stderr (load / typecheck / eval / render) |

## HTML output

```console
$ rustyfi --format html doc.saty  # a web page
```

`--format html` writes **one continuous, self-contained web document**. There
are no pages in it: it is built from the flat block stream as it stood
*before* page breaking, so nothing is cut at a page boundary and there are no
headers, footers or page numbers. The browser does the typesetting — real
`<p>` paragraphs it breaks, hyphenates and justifies itself, at whatever
width the window happens to be.

What survives as structure:

- **headings**, from `register-outline` paired with the destination frame the
  doc class wraps each title in — a structural match on the destination name,
  never a guess from font size. They get real `id=` anchors, but no table of
  contents is generated: a document that wants one typesets it, and a second
  generated copy above the title duplicated it in every real manual;
- **lists** as `<ul>`/`<ol>`/`<li>`, **emphasis** as `<em>`/`<strong>`, where
  the list/emphasis commands opt in by emitting the inert `list-mark` /
  `inline-mark` boxes (the bundled 0.1 `itemize`, `std-ja` and `v01-mini` do).
  A list package that does not — the corpus `enumitem` — still indents, because
  the indentation a `block-frame-breakable` folds into its lines' offsets is
  recovered as a `margin-left`;
- **tables** as real `<table>`/`<tr>`/`<td>`, with **the rules the document
  actually drew**: which grid lines exist is read off the table's own rule
  graphics, so `easytable`'s three-rule booktabs look stays three rules
  instead of becoming a full grid;
- **framed blocks** keep their own decoration — a `stdjabook` title box, a
  `+code` panel — because the deco callback the PDF path already runs is
  recorded box-local and replayed as a `background` (a flat panel) or a
  stretched `<svg>` (anything else). A frame whose deco draws nothing, which
  is most of them, still draws nothing;
- **code blocks** as code: a fixed-pitch face reaches the browser as a
  monospace stack, and its line breaks survive as `<br>` rather than
  collapsing to spaces. The face is the only signal that separates a `+code`
  block from a wrapped paragraph — both are consecutive lines in the box
  stream, because `code.satyh` calls `line-break` once per source line
  exactly as the line breaker does per wrapped line;
- **links** as `<a href>`, to a URL or to an in-document anchor;
- **footnotes** as an `<aside>` immediately below the paragraph that
  references them — there is no page foot to collect them at, and putting
  them where they are read beats sending the reader to the end;
- **images** as `<img>` with the picture inlined as a data URI (a JPEG passes
  through byte-for-byte; a picture placed repeatedly is emitted once and
  shared);
- **centring and flush-right**, recovered from the `inline-fil` that produced
  them.

Math, graphics and rules become inline `<svg>` sized to fit the line. That is
deliberate: they are drawings, not text. Math in particular is flattened to
positioned glyphs during evaluation, so no MathML structure survives to
recover.

**Every math glyph is drawn as an outline path**, taken from the face the
document was typeset in, and math is the one thing in the page that does not
depend on the reader's fonts at all. It is also the one place where it could
not: each glyph carries an absolute offset computed against that face's
metrics, so a reader without it gets a substitute whose advances are wrong and
the equation runs into itself — `∀` is 7.992pt wide in Latin Modern Math and
12.000 in a common substitute, which is enough to bury the next symbol. The
characters are not lost in the process: each equation also carries invisible,
selectable text at the real positions, so an equation can still be
selected, copied, found with the browser's own in-page search and read by a
screen reader. It is what makes the output larger than it would otherwise be —
about 40% on the most math-heavy document in the corpus, a couple of percent
on the rest.

Nothing is fetched and nothing is executed — no external stylesheet, no
script, no remote font. Fonts are otherwise **named**, not embedded: a
reflowed document is not metric-faithful by construction, so pinning the exact
face for ordinary prose would buy nothing and cost megabytes (with the bundled
Japanese faces, one manual came to 20 MB). The reader gets the real face if
they have it and a sensible generic if they do not.

`--format html-reflow` is accepted as an alias for `html`.

### What does not survive

- **A code block's indentation is lost.** Its line breaks survive, as the
  code-blocks bullet above says, but the leading whitespace that made them
  worth keeping does not: indentation reaches the box stream as glue, and
  glue collapses to a single space so the browser can rejoin ordinary prose.
  Preserving it would mean `white-space: pre`, which is the one thing a
  reflowable backend must not do. The `<br>`s also stop for the rest of a
  paragraph once a proportional run appears in it — the fixed-pitch face is
  the only signal that the breaks are the author's, and a mixed paragraph
  withdraws it.
- **Block-frame decorations are not drawn.** A `block-frame-breakable`'s deco
  is a lang-side callback this backend cannot run, and packages use the
  construct for section bodies, list items and quotation blocks as readily as
  for anything with a rule around it, so `.frame` is a plain grouping element
  with margins. The class is there to restyle.
- **`load-pdf-image` shows a labelled box.** Rasterizing an embedded PDF page
  is out of scope for an HTML writer, so it keeps the right size and says
  what it is rather than emitting a broken image.
- **A `draw-text` run's text follows its drawing** instead of sitting at its
  point inside it: HTML content cannot live inside an `<svg>`, and there are
  no page coordinates left to place it at.

## Editor support

`rustyfi lsp` is a Language Server Protocol server speaking over stdio. Point
your editor's LSP client at it for the `satysfi` language:

```console
$ rustyfi lsp                # detect each file's generation from its own text
$ rustyfi lsp --lang 0.1     # analyse everything as 0.1
```

It answers **diagnostics**, **hover**, **go-to-definition**, **completion**
and **symbols** — for both SATySFi generations, and on half-typed buffers.

### Diagnostics

**Lex and parse errors** are reported for any buffer at all, under whichever
SATySFi generation the file is written in. Both 0.0.6 and 0.1 are supported,
and the generation is chosen per file the same way a compile chooses it for the
entry document — a `use` header or a `val` head selects 0.1, a `@stage:` header
or a `let-*` head selects 0.0, and a file that signals neither is checked
against both rather than guessed at. Measured against every
`.saty`/`.satyh`/`.satyg` file in this repository — 247 of them, 64 of which
are 0.1 — it reports **no diagnostics at all on files that compile**, in 0.56 s
for the whole set (30 ms worst case).

An analysis is also bounded: both grammars backtrack exponentially on some
half-typed inputs — 11.5 seconds on one 14 KB buffer, and climbing — so a parse
caps how much backtracking it may do and says so plainly when it hits the cap,
rather than freezing the editor. `rustyfi` itself does the same, with a larger
cap that scales with the file: a compiler is asked once and can afford to try
harder than an editor asked on every keystroke, but neither should run forever.
A parse stopped by the cap is reported as having given up, never as a syntax
error.

**Type errors** are reported for a document whose program can be resolved. A
type error in SATySFi is a property of a whole *program*, not of a file — the
entry document plus every `@require:`d package, in dependency order — so the
server resolves that program first, exactly as a compile does (`rustyfi-loader`
against the same library roots, then elaboration, typechecking and `:>` seal
checking; it stops before evaluation, so no fonts and no pages). The **buffer's
own text** stands in for its file, so unsaved edits are what gets checked.

Three things follow from doing it that way, and each is deliberate:

- **When the program cannot be resolved, nothing is reported.** No library root
  configured, a `@require:` naming a package that is not installed, a `use`
  header document (whose packaging mode resolves dependencies from a
  pre-solved `rustyfi-deps.yaml` and has no seam for an in-memory buffer): all
  of these fall back to the parse tier and say nothing. A wall of "cannot
  resolve" on a file that is not at fault is worse than silence.
- **Library buffers are parse-only unless you ask.** `rustyfi lsp
  --check-libraries` typechecks a `.satyh`/`.satyg` too, as a dependency of a
  synthetic document carrying its own headers. It is off by default because
  SATySFi's global-merge module model lets a library use a module it never
  `@require:`s — `satysfi-base`'s `tabular2.satyh` calls `Color.black` and
  requires only `list` and `table` — which is valid and cannot typecheck alone.
  Swept over every library this repository ships, 76 of 77 bundled packages and
  68 of 68 resolvable corpus sources check clean; the exceptions are listed by
  name, with reasons, in `crates/rustyfi-lsp/tests/project.rs`.
- **An error from another file is not drawn on yours.** Spans in this port
  carry no file identity, and the program under analysis is a merge of many
  files, so a span is only trusted when its own `(line, column, byte)` triple
  matches this buffer. Otherwise the diagnostic goes to the top of the file and
  says where it really came from.

The cost is the reason the two tiers are separate: a parse is under a
millisecond, while resolving and typechecking a real document is 2 ms for a
two-file one and 100–200 ms for one with a full document class behind it (28
files, release build). `--no-typecheck` turns the whole-program tier off.

### Hover, go-to-definition and completion

All three answer from one cursor → syntax mapping over the buffer, and all
three under the same rule: *say only what the file proves.*

- **Hover** names what is under the cursor — an inline command, a module, a
  variant constructor, a record label — and, when the file binds it, how it was
  bound and on which line. Where the author wrote a type (an ascription, a
  `sig`'s `val`, a synonym) it is shown, quoted from the buffer; no type is
  ever inferred, so none is ever wrong. A name that comes from a `@require:`d
  package still gets an answer, and that answer says it comes from elsewhere.
- **Go to definition** jumps within the file, honouring shadowing, the five
  identifier namespaces (`\cmd`, `+cmd`, math `\cmd`, values, types) and
  `Module.member` paths, and it jumps from a `@require:`/`@import:` header to
  the file it names — resolved by the compiler's own loader, so the editor
  cannot disagree with the build. Where it cannot be sure it returns nothing:
  an `open` of a module the file cannot see makes every name bound before it
  unresolvable, because that `open` may be shadowing them.
- **Completion** offers names actually in scope, and is deliberately quiet.
  `\` in inline text offers inline commands, `\` inside `${…}` offers math
  commands, `+` offers block commands, `M.` offers `M`'s own members, and a
  bare word in prose offers nothing at all.

All three keep working on a buffer that does not parse — and on one that does
not even *lex*, which is what `{\emp` is the moment you start typing a command:
everything written before the break is still answered about.

### Symbols

**Symbols** fill the outline pane, the breadcrumb and "go to symbol", for one
file (`textDocument/documentSymbol`) and across the project
(`workspace/symbol`). Both generations' declaration forms are covered —
0.0.6's `let`/`let-rec`/`let-inline`/`let-block`/`let-math`/`let-mutable`/
`type`/`module … : sig … end` and the `direct` items in a signature, 0.1's
`val` family with its `~`/`persistent ~` stage qualifiers, `type`,
`signature`, `include` and nested `module`s, and both header families. A
module's members are its *children*, so a library folds down to one entry
rather than thirty. Over the same 247-file corpus the outline extraction finds
9,843 declarations in 1.2 s, and the only file that yields nothing is the one
that is zero bytes long.

The walk is deliberately structural — no name resolution, no types, no
following `@require:` — so it cannot produce a *wrong* answer, only an
incomplete one, and it works on a half-typed buffer: it reads the top-level
declaration sequence one declaration at a time, so an unfinished `let` at the
bottom of the file costs you that one symbol rather than the whole outline.

### Configuration

`--lib-root <dir>` (or `$RUSTYFI_LIB_ROOT`, or the client's
`initializationOptions.libRoot`) serves both halves that need one: following a
`@require:` header to its package file, and resolving a buffer's dependency
graph for the type tier. `@import:`, being relative to the importing file,
needs no configuration. `initializationOptions` accepts `lang`, `libRoot` (a
string or an array), `checkLibraries` and `typecheck`, with the command line
winning wherever both speak.

### As a library

Everything except the whole-program tier and `workspace/symbol` is also
available as a plain library function, with no LSP types in its signature, no
filesystem access and no default features needed —
`rustyfi_lsp::analyze(source, lang) -> Vec<Diag>`,
`rustyfi_lsp::document_symbols(source, lang) -> Vec<Symbol>`, and
`build_model` / `hover` / `definition` / `completions`. So a browser editor
compiled to `wasm32-unknown-unknown` gets exactly what the desktop one does.
The two exceptions are where the filesystem enters: the whole-program tier is
`rustyfi_lsp::project::check`, behind the (default-on) `typecheck` feature, and
searching a project for a symbol means reading it, so that part lives in the
server half.

## Performance

Minimum CPU time over three interleaved runs against SATySFi
0.0.11, all five configurations measured in one pass (`benchmark.py`):

| doc | pages | rustyfi cold | rustyfi cached | SATySFi | `--bytecomp` | warm aux |
|---|---|---|---|---|---|---|
| latexcmds | 12 | **0.22 s** | **0.08 s** | 1.07 s | 1.04 s | 0.67 s |
| enumitem | 27 | **0.96 s** | **0.12 s** | 2.54 s | 2.33 s | 1.46 s |
| easytable | 19 | **1.34 s** | **0.14 s** | 2.91 s | 2.85 s | 1.19 s |
| figbox | 21 | 1.28 s | **0.16 s** | 2.55 s | 2.43 s | 1.12 s |
| slydifi | 30 | 1.21 s | **0.13 s** | 1.74 s | 1.28 s | 1.16 s |
| xpath | 11 | 3.00 s | **0.10 s** | 9.49 s | 2.65 s | 3.37 s |

## Known gaps

- Fonts are named by file or hash entry, not by package: a document asking for
  `fonts-junicode:Junicode-Bold` falls back to a name heuristic.
- Cross-version `deco` crosses both ways now, including through optional
  arguments and nested module signatures — but not through an *open* optional
  row (nothing names the labels to forward) or a functor signature member.
- `font` and 0.1's `paren` cross in neither direction, and no bridge would
  change that: both are representation forks rather than missing features.
  0.0.6 has no `font` type at all, and 0.1's `paren` takes a context where
  0.0.6 takes three explicit scalars, with no way to recover the axis.
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
  rustyfi-satyrographos/  package manager
  rustyfi/                the binary
lib-rustyfi/              bundled packages: dist/ (0.0) and dist-v01/ (0.1)
layout-tests/             layout fidelity gate, corpus, probes, measurement
install.sh, download-fonts.sh, benchmark.py
```

## License

MIT — see [LICENSE](./LICENSE).

Two sets of files bundled here are not covered by it and keep their own terms.
The fonts `download-fonts.sh` fetches carry the IPA Font License v1.0,
SIL OFL 1.1, the GUST Font License and DejaVu's, each copied next to the font it
covers. The SATySFi packages under `lib-rustyfi/` are upstream's, LGPL-3.0.
