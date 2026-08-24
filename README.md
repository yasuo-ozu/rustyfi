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
| `--format <fmt>` | `pdf` (default), `html` or `markdown` — see [HTML output](#html-output) and [Markdown output](#markdown-output) |
| `--unicode-math` | markdown only: equations as their characters (`x²`) instead of drawn SVG — see [How math is written](#how-math-is-written) |
| `--katex` | html and markdown: equations as LaTeX in math delimiters, for a KaTeX/MathJax reader |
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

`--katex` writes the equation as LaTeX in `\(…\)`/`\[…\]` instead, for a page
that runs KaTeX or MathJax — see [How math is written](#how-math-is-written),
including what a re-derivation from laid-out glyphs cannot give back.

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

### What does not survive

Everything Markdown has no way to say is **dropped, not approximated**:

- **frames, decorations and borders** — a blockquote is not a frame;
- **alignment** — `\align-center` is a pair of `inline-fil`s and there is no
  alignment syntax; nothing about the text depends on it;
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

## How math is written

`${\frac{a}{b}}` is parsed, elaborated, evaluated **and laid out** during
compilation. What reaches a backend is a flat list of glyphs with coordinates
plus a couple of filled paths for the fraction bar and the radical sign; there
is no `\frac` node anywhere, and no backend can serialize one. So every
rendering below is a *recovery* from geometry, and which one you want depends
on where the file is going to be read.

| | markdown | html | what it emits |
|---|---|---|---|
| *(default)* | ✔ | ✔ | an inline `<svg>` of glyph outlines |
| `--unicode-math` | ✔ | — | the characters, in reading order |
| `--katex` | ✔ | ✔ | LaTeX in math delimiters |

The two flags are mutually exclusive, and both are an error with
`--format pdf`, which typesets the equation itself.

### Default: outlined SVG

Each glyph is drawn as a `<path>` taken from the document's **own** face, at
the coordinates the layout computed. This is what the PDF draws, so it is the
only mode that reproduces it — and because the outline travels with the file,
it renders the same for a reader who has never heard of Latin Modern Math. A
`<text>` naming the face would not: where the reader lacks it, the
substitute's advances are not the ones each glyph's absolute offset was
computed against, and the equation collides with itself (measured on
`\forall \epsilon \: \exists \delta` at 12pt, the port reserves 7.992pt for
`∀` where a substituted face draws 12.000, so `ε` lands inside the
quantifier).

The characters are kept **behind** the drawing as invisible `<text>`, so an
equation can still be selected, copied, searched with the browser's own
in-page find, and read aloud by a screen reader. That is verified in a real
headless browser rather than assumed.

The cost, in Markdown, is real: a renderer that sanitizes HTML — GitHub's
comment fields, most static-site pipelines — drops the `<svg>` and leaves
nothing in its place. That is what the other two modes are for.

### `--unicode-math`: the characters

```console
$ rustyfi --format markdown --unicode-math doc.saty
```

The glyphs sorted by their own x offsets and written out, with two pieces of
two-dimensional structure recovered: **scripts** become Unicode
superscript/subscript characters where one exists (`x²`, `∑ₐᵇ`) and `^q`/`_q`
where none does, and **fractions** are split at the bar — which survives as a
wide flat fill — into `(a+b)/(c+d)`.

This is the only form that is **text**: it survives a sanitizing renderer, it
reads in a terminal, `grep` finds it, and it needs nothing of the reader at
all. Markdown-only, for the same reason — an HTML page is markup by
definition and can always draw the real thing.

**What is lost**: radicals (the sign is a path, not a glyph, so `√` is not
written and `√(1-v²/c²)` reads as its contents alone), matrices and aligned
environments, nested fractions beyond one level, and anything whose meaning is
carried by position rather than by its characters.

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
paragraph is written in the display form; nothing else can be alone in a
block.

It is a **re-derivation, not a round trip**, and the difference is worth
being precise about. What comes back: fractions (`\frac{a+b}{c}`), scripts and
limits grouped correctly (`\sum\limits_{k=1}^{n}`, not the `\sum_{k}_{=}_{1}`
that a naive per-glyph emitter produces and that KaTeX refuses to render),
around 180 symbols by name, and the alphabet of a styled letter — `ℝ` really
does come back as `\mathbb{R}`, because SATySFi writes the style into the
codepoint rather than beside it.

What does **not** come back, in every case because the information is not in
the box stream rather than because it is unimplemented:

| construct | what you get instead | why |
|---|---|---|
| `\sqrt{x}` | the radicand, unwrapped | the radical sign is a drawn path, not a glyph; there is no `√` to key on |
| matrices, `\begin{aligned}` | the cells, in x order | the arrangement is carried by position and no bar delimits it |
| a fraction inside a fraction | flattened into the outer one | recovery is one bar deep |
| `\text{…}` | its characters, in math mode | the run is folded into one glyph record with no mark that it was upright |
| `\left(…\right)` | the delimiter characters | a grown delimiter arrives as one record per assembly part; the size is lost, the character is not |
| `\,` `\;` `\quad` | approximated | all of them are "a gap wider than the threshold" by then |
| colour | dropped | no `\color` is emitted for something that cannot be measured back |

Anything whose name is not in the symbol table falls through as the character
itself, escaped where LaTeX reserves it — so an unrecognised symbol renders as
what the document set, never as a guess and never as nothing.

One thing deliberately **not** re-emitted: the spacing around a binary
operator or a relation. SATySFi's layout inserts it by the same rules LaTeX
uses, so writing it back as `\ ` on top of the space LaTeX adds anyway would
render the equation wider than the PDF. `x + 1` is emitted as `x+1`. The
spacing that *is* kept is the kind LaTeX would not supply — the word spaces
inside a `\text` run, which are otherwise the only trace that a space was set
there at all.

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
