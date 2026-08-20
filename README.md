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

That is the whole install. The binary lands in `bin/`, its packages and fonts in
`lib/rustyfi/`, the man page in `share/man/man1/` — and `rustyfi` searches
`~/.local/lib/rustyfi` and `/usr/local/lib/rustyfi` on its own, so nothing needs
configuring and Japanese renders out of the box. Unpacked anywhere else, point
`$RUSTYFI_LIB_ROOT` at the `lib/rustyfi` inside it.

Archives are built for Linux (x86_64, aarch64), macOS (Intel and Apple silicon)
and Windows (x86_64, a `.zip` with the same `bin/ lib/ share/` layout).

### From source

```console
$ git clone https://github.com/yasuo-ozu/rustyfi && cd rustyfi
$ cargo build --release --bin rustyfi
$ sh scripts/download-fonts.sh      # IPAex, Junicode, Latin Modern — pinned, ~175 MB
```

A clone fetches its own fonts: they are not committed, each carrying its own
licence. Without them you still get PDFs, in the base-14 fonts, and Japanese
will not render.

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
3. `~/.local/lib/rustyfi`, then `/usr/local/lib/rustyfi` and `/usr/lib/rustyfi`

All of them are searched in that order, so a package a project installed for
itself layers over the system one rather than hiding it — and a clone needs no
configuration at all.

Roughly 30 upstream packages ship with it, including `stdja`, `stdjabook`,
`stdjareport`, `itemize`, `code`, `math`, `tabular`, `annot` and `proof`, plus
the 0.1 tree (`std-ja`, `inline`, `block`, `map`, `set`, …) under `dist-v01/`.

To install someone else's package, the same binary is a Satyrographos analog:

```console
$ rustyfi satyrographos install ./satysfi-xpath   # a local path, a .tar.gz, or a registry
$ rustyfi satyrographos list
```

It reads upstream `Satyristes` manifests, keeps a project lockfile, and verifies
registry downloads by sha256.

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
| `--target-version <v>` | `0.0` (default) or `0.1`; a `use` header auto-selects `0.1` |
| `--font <file>` | use a TrueType/OpenType file as the regular face |
| `--font-dir <dir>` | font root holding `dist/hash/fonts.rustyfi-hash` |
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
- Cross-version `deco` crosses one way (a 0.0 package's deco used from a 0.1
  document), not yet the reverse.
- `font` and 0.1's `paren` are stand-in types.

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
the grammar. `cargo test --workspace` runs 1587 tests; CI adds the corpus
regression and the layout-fidelity comparison above.

## License

MIT — see [LICENSE](LICENSE).

Two sets of files bundled here are not covered by it and keep their own terms.
The fonts `scripts/download-fonts.sh` fetches carry the IPA Font License v1.0,
SIL OFL 1.1, the GUST Font License and DejaVu's, each copied next to the font it
covers. The SATySFi packages under `lib-rustyfi/` are upstream's, LGPL-3.0.
