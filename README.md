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

### Publishing your own

`rustyfi publish` is the `opam publish` step: it reads the `Satyristes` you are
standing in and writes a package definition into a checkout of the repository,
pointing at a tarball **you** have already released.

```console
$ rustyfi publish --url https://example.org/great-package-1.0.0.tar.gz \
                  --archive ./dist/great-package-1.0.0.tar.gz \
                  --registry ~/src/satyrographos-repo --commit
```

`--archive` is a local copy of that same tarball, hashed to supply the
`sha256`; pass `--sha256 HEX` instead if you already have the digest, or both
to have them cross-checked. The default output is Satyrographos' own OPAM shape
(`packages/satysfi-<name>/satysfi-<name>.<version>/opam`), so what it writes is
installable by real Satyrographos too; a repository using this port's native
`packages/<name>.toml` index gets that instead. Which one is detected from what
the repository already holds, and `--shape opam|toml` decides when that is
undecidable — it is never guessed. `--dry-run` prints the definition and writes
nothing.

If a `(library …)` block has no `.opam` file beside its `Satyristes`, one is
created — the file an opam pin of your source tree reads, and the one
`(opam "…")` names. On a terminal you are asked for the fields the manifest
cannot supply (`license:`, `homepage:`, `authors:` …), with defaults taken from
your git remote and `git config`; `--no-wizard`, a pipe or a CI job writes the
derived file unasked. An existing `.opam` is never touched. It deliberately
carries no `url { }` block: a source tree is not a released tarball, and the
url/checksum pair belongs only to the repository entry that pins one.

Nothing is uploaded and nothing is pushed. With `--commit` the definition is
committed on a branch (`--branch NAME`) in your checkout, and the `git push`
you would run next is printed for you to run yourself. Re-publishing a version
that already exists needs `--force`, since that version is what a consumer pins.

With several repositories configured and no `--registry`, `$RUSTYFI_REGISTRY`
or project `(registry …)`, `publish` lists them and stops rather than picking
one: `search` and `install` consult them all, but a release goes into exactly
one.

## Useful options

| flag | what it does |
|---|---|
| `-o <path>` | output path (default: the input with a `.pdf` extension) |
| `--format <fmt>` | `pdf` (default), `html`, `markdown` or `latex` — see [HTML output](#html-output), [Markdown output](#markdown-output) and [LaTeX output](#latex-output) |
| `--svg-math` | html and markdown: equations as SVG `<text>` + `<rect>` — markdown's default; see [How math is written](#how-math-is-written) |
| `--svg-outline-math` | html and markdown: equations as outline paths with selectable text behind them — html's default |
| `--katex` | html and markdown: equations as LaTeX in math delimiters, for a KaTeX/MathJax reader |
| `--mathml` | html and markdown: equations as MathML Core, laid out by the browser itself |
| `--unicode-math` | markdown only: equations as their characters (`x²`) |
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

`--svg-math` draws the same equations with SVG `<text>` and `<rect>` instead,
which is 0.70x the bytes raw and 0.57x gzipped and depends on the reader
having the document's faces; `--katex` writes them as LaTeX in `\(…\)`/`\[…\]`
for a page that runs KaTeX or MathJax; `--mathml` writes MathML Core, which the
browser lays out itself with nothing loaded and nothing run (0.53x raw, 0.42x
gzipped). See
[How math is written](#how-math-is-written), including what a re-derivation
from laid-out glyphs cannot give back.

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

## Markdown output

```console
$ rustyfi --format markdown doc.saty   # -> doc.md
```

`--format markdown` is a **subset of `--format html`**. It reads the same
pre-page-break block stream and recovers the same structure through the same
code — headings from `register-outline`, lists from the `list-mark` boxes,
tables from a `tabular`'s own cell positions, links, emphasis, the CJK
spacing rule, the line breaker's own hyphens. It then writes GitHub-flavoured
Markdown, which can say much less.

**Readability is the goal; layout fidelity is explicitly not.** Nothing here
tries to look like the PDF. What it tries to be is a document you would be
willing to read in a terminal, paste into an issue, or feed to a diff.

What survives: `#`..`######` headings, `-`/`1.` lists with real nesting,
`*emphasis*` and `**strong**`, GFM pipe tables, `[text](url)` links, fenced
code blocks, GFM `[^1]` footnotes with their definitions collected at the
foot, and reference-style images.

Two things come out **better** than in HTML, both because a fence is a
stronger container than a `<p>`:

- **a code block keeps its indentation.** In HTML it is lost, because the
  `inline-skip` carrying it collapses like any other glue. Here it is divided
  back into columns by the fixed-pitch character advance measured from the
  block's own runs — `code.satyh` sizes the indent in exact multiples of it,
  so this recovers the source's own column count rather than estimating it;
- **a footnote is a real footnote**, `[^1]` plus a definition, rather than an
  `<aside>` wedged after the paragraph because there is no page foot left.

Detecting a code block is also stricter than HTML's "every run is
fixed-pitch", which misses every `+code` block containing Japanese — a
fixed-pitch Latin face has no CJK glyphs, so those characters are set in the
document's own face and the paragraph reads as mixed. Markdown keys on the
structure instead: `code.satyh` ends every source line with an `inline-fil`,
and justified prose ends only its last line that way.

### The three decisions with no good answer

- **Math has three renderings, and a flag to choose between them** — see
  [How math is written](#how-math-is-written) below. It is the one decision
  here whose right answer depends on where the file will be *read* rather than
  on what the document says.
- **Graphics become `[graphic]`.** There is no Markdown for a vector drawing,
  and dropping them silently is the worst of the options: a reader of
  `xpath`'s manual would see paragraphs referring to figures that are not
  there, with no indication anything was missing. So each one leaves a named
  hole a reader can act on. Drawings whose INK is under 4pt in either
  dimension are dropped instead — the corpus is full of hairline rules, leader
  dots and heading underlines drawn as one-off graphics, and marking each of
  those would bury the real figures.
- **Images are reference-style, with a `data:` URI definition at the foot.**
  A compile produces ONE output path, so a sidecar directory would be a
  contract the CLI does not have and would break the moment the file is moved
  or pasted. Reference style is what keeps the prose legible: a figure is
  commonly a hundred kilobytes of base64, and a paragraph interrupted by two
  screens of it is not a readable file. `![image 1][md-img-1]` in the text,
  the payload at the bottom. Data URIs render in VS Code, Typora, pandoc and
  most local previewers; GitHub's image proxy refuses them, so there an image
  degrades to its alt text rather than to a broken path.

### An aligned equation is not a table

The `math` package builds `+align` — its multi-line equation block — out of a
`tabular`, so an aligned equation and a spreadsheet reach the backend in the
same box. Rendered literally, a two-row `+align` came out as a GFM pipe table
whose **column heading was an equation**: GFM's delimiter row always follows
row one, so the first line of the alignment was promoted to a header.

An aligned equation is recognized by its construction rather than by
resemblance — it draws no rules, every cell's only ink is an equation, and its
columns run right, left, right, … (the `inline-fil` placement `+align` uses,
and exactly the column pattern LaTeX's `aligned` is defined as). A grid that
draws its own lines, one with text in any cell, or one whose cells are
*centred* — a matrix — is a table and stays one.

What is written instead depends on what the mode can say:

- **`--katex`** writes one `$$\begin{aligned} … \end{aligned}$$`. The cell
  boundary is the `&` and the row boundary is the `\\`, so the document's own
  alignment survives exactly, including the second and later column pairs of a
  multi-column `+align`;
- **`--svg-math` / `--svg-outline-math`** write one block per row, with the
  row's cells joined — a row is one equation split at the alignment point —
  each centred like any other display equation (below). The column alignment
  *between* rows is lost: keeping it would mean re-deriving the solved grid
  geometry into a composed drawing, to buy an alignment that only exists for a
  reader whose renderer keeps the `<svg>` at all;
- **`--unicode-math`** keeps the grid. It writes characters, and a two-column
  text table is a defensible way to show an alignment in plain text — but the
  grid now gets an **empty header row**, so no equation is promoted to a
  heading. GFM has no headerless table, and an empty header is the only way to
  say it.

The block structure does not depend on whether `download-fonts.sh` has been
run: with no font store a drawing mode degrades to characters, but that is a
degradation of one equation's rendering, not of the document's shape.

### A displayed equation is centred

In the drawing modes a display equation — one that is the whole of its
paragraph, which is what "displayed" MEANS in the box stream — is wrapped in
`<div align="center">`.

This is the one exception to "alignment is dropped", and it is a consequence of
that rule rather than a hole in it. A drawn equation is **already raw HTML**:
the `<svg>` is in the file whatever this decides, so a wrapper around it costs
nothing in portability — a renderer that strips the `<div>` has necessarily
stripped the drawing too. Prose has no such standing, and centring a paragraph
would mean putting HTML into a file that had none. The alignment is also one
the document really asked for, and the HTML backend already honours it
(`data-align="center"`); dropping it here made the two backends disagree about
the same recovered fact.

`align`, not `style`, deliberately: GitHub sanitizes rendered Markdown through
`html-pipeline`'s `SanitizationFilter`, whose allowlist carries `align` for any
element and carries `style` for none, so `<div style="text-align:center">`
arrives as a bare `<div>`. The attribute is deprecated in HTML5 and every
browser still implements it; the sanitizer is the binding constraint.

The other two modes are untouched. `--katex` writes `$$…$$`, which a KaTeX
reader centres itself (`.katex-display { … text-align: center; }`) and which
GitHub reads as math only while the `$$` block stands alone — a wrapper would
turn the equation into literal text there. `--unicode-math` is the plain-text
mode and stays text.

### What does not survive

Everything Markdown has no way to say is **dropped, not approximated**:

- **frames, decorations and borders** — a blockquote is not a frame;
- **alignment** — `\align-center` is a pair of `inline-fil`s and there is no
  alignment syntax; nothing about the text depends on it. The one exception is
  a DRAWN display equation, which is already raw HTML and so can be centred for
  free — see [A displayed equation is centred](#a-displayed-equation-is-centred);
- **page breaks, running heads and folios** — already absent from the
  pre-page-break stream, and meaningless once reflowed;
- **colour, font and size** — no styling syntax outside emphasis and code;
- **a paragraph's recovered indentation** — four leading spaces is an indented
  CODE BLOCK, so reproducing it would be a lie. This is what the HTML backend
  uses to give the third-party `enumitem` its nesting, so an `enumitem` list
  arrives here as flat paragraphs. Its numbered labels (`(a)`, `(i)`) are
  typeset text and survive; its drawn bullets are graphics below the size
  threshold and do not;
- **table rules** — GFM has one table style, and no alignment colons are
  emitted either: a cell records where it was PLACED, not how its column was
  declared to align;
- **in-document anchors** — a `\ref` becomes plain text rather than a link.
  Markdown has no anchor scheme, and renderers invent heading anchors from the
  heading's own words, so `[Section 3](#sec:intro)` would go nowhere. The
  cross-reference text the document typeset is already what a reader needs.

One known rough edge: a **one-line** `+code` block that also contains non-Latin
text satisfies neither code-block test — it is not all fixed-pitch, and a
single line is trivially "all its lines end with a fil", which is true of every
one-line paragraph in the document. It comes out as inline code spans rather
than a fence. Nothing is lost, and a fence would cost false positives on every
short sentence that mentions a `\command`.

`--format md` is accepted as an alias for `markdown`.

## LaTeX output

```console
$ rustyfi --format latex doc.saty   # -> doc.tex
$ lualatex doc.tex
```

`--format latex` writes a **complete, compilable `.tex` document** —
`\documentclass`, a preamble, `\begin{document}` … `\end{document}` — not a
fragment to paste into one.

It is the same recovery as the other two backends, reading the same
pre-page-break block stream through the same code. What differs is that the
target is **another typesetter** rather than a reader or a browser, and LaTeX
can say most of what SATySFi can. So where HTML and Markdown drop or
approximate, this mostly does not:

| | markdown | html | latex |
|---|---|---|---|
| math | characters, or drawn | drawn `<svg>` | real `$\frac{a+b}{c}$` |
| drawings | an `<svg>` a sanitizer strips | an `<svg>` | a `tikzpicture` of the same paths |
| a `\ref` | plain text | `<a href="#…">` | a working `\hyperlink` |
| a label inside a drawing | flows after it | absolutely positioned | a `\node` at its own point |
| table rules | none — GFM has one style | per-cell CSS | `|` and `\hline` where drawn |
| a code block | a fence | a `<pre>` | `fancyvrb`'s `Verbatim` |
| CMYK colour | — | converted, lossily | `xcolor`'s own `cmyk` |

Layout fidelity is **not** the goal, and cannot be: LaTeX breaks the lines,
hyphenates and paginates itself. What is carried over is the document's paper
size — so a `slydifi` deck comes out as landscape slides rather than reflowed
onto A4 — and its measure, taken from the widest line in the flow.

### Which engine

The generated preamble says, in a comment at the top of the file, and enforces
it with `iftex` so a wrong engine fails immediately instead of dropping
glyphs:

- **Nothing above Latin-1** → it compiles under **pdflatex, xelatex and
  lualatex alike**. The mathematics is written with `amsmath`/`amssymb`
  command names rather than Unicode characters, and the only
  engine-conditional line is an `\ifPDFTeX`-guarded `fontenc`.
- **Any CJK** → **lualatex**, with `luatexja-fontspec` and the Harano Aji
  faces. pdfLaTeX cannot set CJK at all; XeLaTeX could, through `xeCJK`, but
  a generated file can only name one and `luatexja` is the one that also gets
  the JLreq inter-script spacing right.
- **Anything else above Latin-1** — Greek, Cyrillic, Hebrew, Arabic, an
  emoji, a bare `≤` in prose — → **xelatex or lualatex**, and pdflatex is
  refused with a named error rather than left to fail once per character.
  Note the honest limit here, which the preamble also states: a Unicode
  engine can ADDRESS those characters, but the default font may not contain
  them, and TeX reports a missing glyph as one log line and then exits 0. If
  a character is absent from the PDF, add a `fontspec` main font that covers
  the script. The backend does not pick one, because picking wrong is worse
  than saying so.

The preamble declares **only the packages the body turned out to use** —
`tikz`, `hyperref` and `fvextra` appear only if the document has a drawing, a
link and a code block respectively — so you can tell from the top of the file
what is in the rest of it.

### What survives

Headings (`\section*`…`\subparagraph*`, starred because the document typeset
its own numbering and LaTeX would otherwise add a second), `itemize` and
`enumerate` with real nesting, `tabular` with the rules the document actually
drew, `\emph`/`\textbf`, `\texttt`, `\footnote`, `\href`, `\hyperlink`
cross-references to `\hypertarget` anchors on the headings, `Verbatim` code
blocks that keep their indentation, and `tikzpicture` figures drawn from the
same vector paths the PDF writer strokes.

Every character LaTeX reserves — `# $ % & _ { } ~ ^ \`, plus `< > |`, which
are only correct under a stated font encoding — is escaped. That is not a
nicety: a bare `%` comments out the rest of the line and the document *still
compiles*, so `100% of them` would silently lose the other half of its
sentence.

### What does not

- **Raster images are a sized, dashed placeholder**, labelled `[image n]`. A
  compile produces one output path, so writing the pictures out as sidecar
  files would be a contract the CLI does not have and would break the moment
  the `.tex` is moved; LaTeX has no data-URI equivalent, because
  `\includegraphics` reads a file and nothing else. The placeholder is the
  size the image occupied, so the page around it stays honest.
- **Frames, decorations and alignment** are dropped, as in the other
  reflowed backends. LaTeX has boxes, but a frame's geometry was computed for
  a measure this document is not being set at.
- **Rule thickness and colour** in a table: `\hline` has one width for the
  whole table and no colour without `colortbl`. Which boundaries are ruled is
  kept; how heavily is not.
- **Column alignment** is `l` throughout. A cell records where it was
  *placed*, not how its column was declared to align, so anything else would
  be a guess — the same reason the Markdown backend emits no alignment colons.
- **A drawing bigger than the measure is scaled down** to fit. Not a
  preference: a `tikzpicture` is one unbreakable box, and LaTeX responds to
  one taller than `\textheight` by ending the page and trying again, forever.

### Known wrong, as opposed to known absent

Everything above is a deliberate simplification. These are cases where the
output is silently *wrong* or does not compile, found by an adversarial sweep
and listed so that a reader comparing the `.tex` to the PDF finds them here
rather than discovering them:

- a **footnote is numbered twice** under `stdjabook`/`stdjareport` — the note
  body already opens with the numeral the document typeset, and `\footnote`
  adds its own (the reference *marker* is already dropped);
- a **footnote inside a table cell loses its text**, and a **table nested
  inside a table cell is emitted before its parent** rather than inside it;
- a **list nested five deep** is `Too deeply nested` — four is LaTeX's own
  limit for `itemize`/`enumerate`, and it is not raised;
- **coordinates or a paper size past `\maxdimen`** (about 5.76 m) fail; TikZ
  evaluates a coordinate before applying the fit scale, so the scaling that
  saves an oversized drawing does not save these.

Math is `--katex`'s conversion — the same function, so
[What `--katex` cannot recover](#what---katex-cannot-recover) applies here word
for word. All four math flags are **refused** with `--format latex` rather than
ignored: a `.tex` reaches a math typesetter by definition, so it always writes
the LaTeX `--katex` asks for, and every other rendering would only lose
structure it can keep.

`--format tex` is accepted as an alias for `latex`.

## How math is written

`${\frac{a}{b}}` is parsed, elaborated, evaluated **and laid out** during
compilation. What reaches a backend is a flat list of glyphs with coordinates
plus a few filled paths for the fraction bar and the radical sign; there is no
`\frac` node anywhere, and no backend can serialize one. So every rendering
below is a *recovery* from geometry, and which one you want depends on where
the file is going to be read.

| | markdown | html | what it emits |
|---|---|---|---|
| `--svg-math` | **default** | ✔ | SVG `<text>` + `<rect>`/`<line>`, positioned by the layout |
| `--svg-outline-math` | ✔ | **default** | an outline `<path>` per glyph, characters kept behind it |
| `--katex` | ✔ | ✔ | LaTeX in math delimiters |
| `--mathml` | ✔ | ✔ | MathML Core elements, laid out by the browser |
| `--unicode-math` | ✔ | — | the characters, in reading order |

The five are mutually exclusive, and all five are an error with
`--format pdf`, which typesets the equation itself. **The defaults differ by
format on purpose**: an HTML page is self-contained and nobody reads it as
source, so it gets the rendering that reproduces the PDF exactly and depends
on nothing the reader has; a `.md` is read as source at least as often as it
is rendered, so it gets the compact one whose source says what it means.

### `--svg-math`: text and shapes — Markdown's default

Each glyph is an SVG `<tspan>` at the position the layout computed, all of them
in one `<text>`; each fraction bar and rule is a `<rect>` or a `<line>`. The
text is *real* text — it selects, copies, searches and reads aloud with no
invisible layer behind it — and the source says what it means:

```html
<svg … viewBox="0 0 38.95 12.08" style="overflow:visible; vertical-align:-2.41pt;">
  <rect x="0" y="4" width="20" height="0.48" fill="rgb(0,0,0)"/>
  <text class="math-text" style="font-family:'Latin Modern Math', …;font-size:12px;">
    <tspan x="0" y="8.54">∀</tspan><tspan x="7.99" y="8.54">𝜀</tspan> <tspan x="18.08" y="8.54">:</tspan>
  </text>
</svg>
```

Two costs, both real:

- **it depends on the reader having the document's faces.** Every glyph is
  positioned absolutely, so a substitute whose advances differ does not reflow
  — it collides. The family is named inline, so a reader who has Latin Modern
  Math gets exactly the PDF's glyphs and one who does not gets a near miss;
- **a MATH-table variant glyph has no character that names it.** A display-size
  `∑`, a stretched delimiter and an `ssty` script form are addressable only by
  glyph id, and `<text>∑</text>` draws the *base* glyph — whose ink is a
  different size, while the limits around it were centred on the variant's
  advance. So those glyphs keep an outline `<path>`, with their character
  carried invisibly beside it. On `latexcmds` that is 27 paths against 51
  `<text>` runs: the exception is small, and skipping it would silently
  reintroduce a measured misplacement in exactly the equations most likely to
  be displayed.

Verified against the mode below, which an audit compared glyph-for-glyph with
the rasterised PDF: across the corpus's **100 math boxes, every glyph lands at
the same coordinate and carries the same characters in both modes.**

**Displayed equations are pretty-printed; inline ones are not**, and that is a
CommonMark constraint rather than taste. A display equation is its own HTML
block, so a multi-line `<svg>` satisfies rule 7 and passes through whole. An
inline one sits mid-paragraph, where a renderer with `breaks: true` — many
Markdown pipelines, including this repo's own playground preview — inserts a
`<br>` at every newline, breaking the drawing. Neither shape ever contains a
blank line, which would terminate the HTML block in every implementation.

### `--svg-outline-math`: outlines — HTML's default

Each glyph is drawn as a `<path>` taken from the document's **own** face, at
the coordinates the layout computed. This is what the PDF draws, so it is the
only mode that reproduces it — and because the outline travels with the file,
it renders the same for a reader who has never heard of Latin Modern Math. A
`<text>` naming the face would not: measured on
`\forall \epsilon \: \exists \delta` at 12pt, the port reserves 7.992pt for
`∀` where a substituted face draws 12.000, so `ε` lands inside the quantifier.

The characters are kept **behind** the drawing as invisible `<text>`, so an
equation can still be selected, copied, searched with the browser's own
in-page find, and read aloud by a screen reader. That is verified in a real
headless browser rather than assumed.

The cost is size — an outline is hundreds of coordinates per glyph. On
`latexcmds`, Markdown's math-heaviest document, it is **2.3× the default raw
and 2.2× gzipped**. In HTML, where nothing strips markup and the page is meant
to be self-contained, that is the right trade and it is the default.

### `--unicode-math`: the characters

```console
$ rustyfi --format markdown --unicode-math doc.saty
```

The glyphs sorted by their own x offsets and written out, with the
two-dimensional structure recovered as far as Unicode can say it: **scripts**
become superscript/subscript characters where one exists (`x²`, `∑ₐᵇ`) and
`^q`/`_q` where none does; **fractions** are split at the bar — which survives
as a wide flat fill — into `(a+b)/(c+d)`, nesting included; and **delimiters**
and **radicals**, which are drawn as paths and have no character in the run at
all, come back as `(x+y)²`, `⌊x⌋`, `‖v‖` and `√(a+b)` — see
[what `--katex` recovers from the drawn paths](#what---katex-recovers-from-the-drawn-paths),
which is the same recovery.

This is the only form that is **text**: it survives a sanitizing renderer, it
reads in a terminal, `grep` finds it, and it needs nothing of the reader at
all. Markdown-only, for the same reason — an HTML page is markup by
definition and can always draw the real thing.

### `--katex`: LaTeX in delimiters

```console
$ rustyfi --format markdown --katex doc.saty
$ rustyfi --format html --katex doc.saty
```

`$…$` and `$$…$$` in Markdown, which is what GitHub, Pandoc, VS Code and
Typora read; `\(…\)` and `\[…\]` in HTML, which is what KaTeX's `auto-render`
and MathJax enable **by default** — neither turns on `$…$` for inline math
without configuration, so emitting it into a web page would show a literal
dollar sign on a reader's default setup. An equation that is the whole of its
paragraph is written in the display form.

It is a **re-derivation, not a round trip.** What comes back: fractions —
nested ones included — delimiters, radicals, scripts and limits grouped
correctly (`\sum\limits_{k=1}^{n}`), around 180 symbols by name, accents as
their commands, prose inside an equation as `\text{…}`, and the alphabet of a
styled letter — `ℝ` really does come back as `\mathbb{R}`, because SATySFi
writes the style into the codepoint.

### What `--katex` recovers from the drawn paths

`\paren`, `\sqbracket`, `\brace`, `\floor`, `\ceil`, `\abs`, `\norm`,
`\angle-bracket` and `\sqrt` have **no character anywhere in the box stream** —
`math-paren` and `math-radical` draw them as `Fill` and `Stroke` paths. They
are recovered from the SHAPE of the path: a delimiter is told from a rule by
being tall and narrow, its family by whether the outline curves, where its arms
are and whether its outer edge has a cusp, and its handedness by which side of
its own bounding box it is thick at at mid-height. The two halves of a pair
find each other by their vertical extent, which `math-paren` gives them
identically. A radical is exact rather than inferred: its sign and its overbar
share a left edge and a top edge to the last bit, because the layout builds
both out of the same two sums.

So `${\paren{a+b}^2}` comes back as `\left( a+b \right)^{2}`, and
`${x = \frac{-b \pm \sqrt{b^2-4ac}}{2a}}` as the quadratic formula. A shape
that matches none of the signatures — a third-party package's own
`math-paren` argument — still GROUPS its body, as `{…}`: no delimiter is
invented, and the script after it still binds to the whole group, which is the
part that was actually wrong.

### What `--katex` cannot recover

Every item here is information the box stream does not carry, not a gap in the
writer:

| construct | what you get instead | why |
|---|---|---|
| a matrix, or rows aligned INSIDE one `${…}` | the cells, in x order | the arrangement is carried by position, and no bar delimits it. (`+align` is a different thing — a real `tabular`, and it *is* recovered; see [An aligned equation is not a table](#an-aligned-equation-is-not-a-table)) |
| `\sqrt[n]{x}` | a plain `\sqrt{x}` | the layout carries the degree and deliberately does not draw it, so it is not in the box stream at all |
| `\lim`, `\sin`, `\max` | the letters, in math italic — the LIMIT on one is recovered, the operator's name is not, unless the two appear together | a `MathOp` run reaches the box stream as plain ASCII letter records, the same as a product of variables. Where a centred limit proves the run is an operator and the letters spell one, `\lim\limits_{x\to0}` does come back |
| a `\setsep` separator | dropped; the two parts run together | its bar is drawn by the same shape an `\abs` uses, so pairing it would close the wrong group |
| `x^{2^{3}}` | `x^{23}` | a script of a script is another small raised glyph, and nothing marks where one script's group ends |
| a fraction nested in BOTH halves of another, at equal width | the two inner halves may swap | `\frac{\frac{a}{b}}{\frac{c}{d}}` puts `b` and `c` at the same `dy` and the same `dx`; there is nothing left to tell them apart |
| `\text{…}` | recovered only when the run holds a space | the layout splits a run at each glue, so a `\text` of separate words arrives as separate records |
| `\,` `\;` `\quad` | approximated | all of them are "a gap over the threshold" by then |
| colour | dropped | not measurable back |

Anything whose name is not in the symbol table falls through as the character
itself, escaped where LaTeX reserves it — never a guess, never nothing. Four
things it deliberately does *not* do, each found by running it over the corpus
rather than over fixtures: it does not re-emit the spacing LaTeX inserts
itself (`x + 1` is emitted as `x+1`, which typesets as `x + 1`), it does not
let a control word swallow the next letter (`\partial` + `t` would concatenate
into an undefined command), it does not let two inline formulas run their
delimiters together into a stray `$$`, and it does not read a script as a
LIMIT merely because its midpoint happens to coincide with a run of base
glyphs — `${x^2+y^2+z^2-xy-yz-zx}` is exactly that coincidence.

### `--mathml`: structure the browser lays out

```console
$ rustyfi --format html --mathml doc.saty
$ rustyfi --format markdown --mathml doc.saty
```

The equation becomes MathML Core in the document's own tree — `<mfrac>`,
`<msub>`, `<msubsup>`, `<munderover>`, `<mover accent="true">`, `<mi>`, `<mn>`,
`<mo>`, `<mtext>` — and the browser typesets it. Nothing is loaded and nothing
runs, unlike `--katex`; the equation is real structure rather than a picture,
unlike either SVG mode, so a screen reader reads it as mathematics and the
browser's own in-page find works on it.

```html
<p class="para math-display"><math xmlns="http://www.w3.org/1998/Math/MathML"
   class="math-ml" display="block"><munderover><mo movablelimits="false">∑</mo
   ><mrow><mi>𝑘</mi><mo>=</mo><mn>1</mn></mrow><mi>𝑛</mi></munderover><mi>𝑘</mi></math></p>
```

**MathML Core, not MathML 3.** Core is the profile browsers actually implement
— Firefox always, Safari since 2013, Chromium since 109 — and its layout is
specified in CSS terms. Two of its restrictions shape the output and both look
like omissions until you know why:

- **`mathvariant` has one legal value in Core, `normal`.** It is not needed:
  SATySFi writes the style into the *codepoint* (`${\bold{R}}` is laid out as
  `𝐑` U+1D411), so bold, script, fraktur and double-struck survive as
  themselves. What the attribute *is* used for is the opposite case — a plain
  ASCII `x` reaching a backend is a letter the document set **upright**, since
  math italic would have been written as `𝑥`, so it is pinned with
  `mathvariant="normal"` against Core's automatic italicisation of a lone
  letter.
- **`movablelimits` is pinned off on every operator base.** The dictionary
  marks `∑` and friends movable, so a browser would re-decide whether their
  scripts go beside or under according to the display style — overwriting a
  position this port measured. `\sum`'s limits are centred and come out
  `<munderover>`; `\int`'s are set beside and come out `<msubsup>`; both stay
  put.

An equation alone in its paragraph is `display="block"`, which is not
decoration: in block display a browser sets `math-style: normal`, putting a big
operator's limits above and below at full size. Several math boxes in one
paragraph merge into **one** element — a formula is not one box.

**It is the same re-derivation `--katex` is**, from the same recovery, so
[the table above](#what---katex-cannot-recover) applies to it unchanged. Two
things it does recover that `--katex` structurally cannot, because MathML has
an element for the shape: an accent binds to its base (`<mover accent="true">`
rather than `x\hat{}` side by side), and a centred limit stays a limit on any
base (`\limits` is legal in LaTeX only after a large operator).

### What `--mathml` does about what it cannot recover

**Nothing is invented.** No `<msqrt>`, `<mroot>`, `<mtable>` or delimiter
`<mo>` is ever synthesized: every element written stands for something the
recovery actually found. An `<msqrt>` guessed from "there is a wide flat fill
here" would render as a fact, and confident-looking MathML for a mis-recovered
construct is worse than visibly rough text, because it reads as authoritative.

**What is done instead is to mark it.** A `<math>` whose rendering this layer
knows does not match the PDF carries `class="rustyfi-approx"`, for one of two
detectable reasons: an inked path that did not become a fraction bar (a
`math-paren` delimiter, a radical sign, an `\overline` — drawn in the PDF, with
no character to recover and nothing in Core that draws a path), or an accent
with no base in its run, which is emitted over an empty group and so renders
beside its character instead of over it. The class has no styling of its own;
it exists so the loss is countable:

```console
$ rustyfi --format html --mathml latexcmds-doc.saty && grep -c rustyfi-approx …
```

Measured: **6 of 48 equations in `latexcmds`, 279 of 561 in `azmath`** — and
in `azmath` 232 of those are one shape, its `accent.satyh` building every
accent out of `math-graphics` and two `draw-text`s, so the base and the mark
arrive as separate math boxes that cannot be paired. Degrading a marked run to
plain text instead was tried and rejected on that measurement: it would demote
correctly recovered fractions, scripts and limits along with the parenthesis it
cannot put back either way.

**In Markdown it is raw HTML**, so it shares the SVG modes' constraints: a
sanitizing renderer strips it, and every element is written on one line so that
a renderer with `breaks: true` cannot inject a `<br>` into it and no blank line
can ever terminate the HTML block. Verified end to end against `marked` with
`breaks` enabled, across `latexcmds` and `azmath`: 609 elements in, 609 out, no
`<br>` inside one and nothing escaped to literal text.

Size, on the same documents, against the outline mode:

| | `latexcmds` md | html | `azmath` md | html |
|---|---|---|---|---|
| `--svg-outline-math` | 1.00x | 1.00x | 1.00x | 1.00x |
| `--svg-math` | 0.43x | 0.70x | 0.40x | 0.56x |
| `--katex` | 0.16x | 0.51x | 0.03x | 0.15x |
| `--mathml` | **0.20x** | **0.53x** | **0.11x** | **0.19x** |

Gzipped the four sit at 1.00x / 0.45x / 0.25x / **0.27x** on `latexcmds`
Markdown — MathML is a little larger than the LaTeX it is derived from, which
is the cost of being markup rather than a language.

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
  rustyfi-html/           HTML and Markdown output, and the structure
                          recovery all three reflowed backends share
  rustyfi-latex/          LaTeX output: a complete, compilable .tex document
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
