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
| `--format <fmt>` | `pdf` (HTML is on the `html-support` branch) |
| `--lib-root <dir>` | where `@require:` looks for packages |
| `--lang <v>` | `0.0` (default) or `0.1`; a `use` header auto-selects `0.1` |
| `--font <file>` | use a TrueType/OpenType file as the regular face |
| `--font-dir <dir>` | font root holding `dist/hash/fonts.satysfi-hash` |
| `--no-cache` | bypass the compile cache |
| `--no-aux` | do not read or write the `.satysfi-aux` cross-reference file |
| `--timing` | per-phase timing to stderr (load / typecheck / eval / render) |

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
