# rustyfi-rust-converted

A native Rust clone of [SATySFi](https://github.com/gfngfn/SATySFi) (reference:
upstream **v0.0.6**), using the [syan2](../syan2) parser framework for the
grammar.

## Status: phases 1–4 core done; slices of 5/6; chimera CLI

A `.saty` document compiles to a real PDF with wrapped, justified text —
including binary operators (full v0.0.6 precedence ladder), `if`/`match`
with patterns and guards, local `let`/`let-rec`(+`and`), tuples, variant
constructors, user-defined commands via `let-inline`/`let-block`,
`let-mutable`/`<-`/`while`/`before`/`!`, record field access and functional
update, itemize (`*` bullets → `Item` constructor trees), math syntax
(parsed and quoted; typesetting is phase 7), modules (`module`/`struct`/
`open`, untyped name-mangling), `#var;` text embeds, and **multi-file
loading** — `@import:`/`@require:` resolve, dedupe, and topologically order
libraries via the `rustyfi-loader` crate (safegraph-backed):

```console
$ cargo run -p rustyfi-cli -- crates/rustyfi-cli/tests/fixtures/minimal.saty -o out.pdf
```

What works:

- **Lexer** (`rustyfi-syntax`): full case-for-case port of the v0.0.6
  `lexer.mll` — five mode states (program / vertical / horizontal / active /
  math) with the same stack discipline, eagerly lexing the whole file.
- **Parser**: syan2 derives over custom `WithSpan<Token, Span>` atoms; a
  `#[recurse]` grammar module for the Expr ↔ text SCC. Milestone-1 subset:
  headers, top-level `let`, `fun`, application, records, lists, string/int/
  float/length literals, inline text with `\cmd` and block text with `+cmd`.
  Token-level `Unparse` round-trip is tested.
- **Elaboration + typechecking** (`rustyfi-lang`): CST → `Ast` with scope
  resolution, then mandatory HM type inference (let-polymorphism at Rémy
  levels, row-polymorphic records, user variants, value restriction, real
  `InlineCmd`/`BlockCmd` command types checked at application sites).
- **Evaluator**: tree-walker with closures; the vminst-named `PrimDef`
  registry (~60 primitives). `document`/`+p`/`\emph` are **not natives** —
  they come from the in-repo `lib-rustyfi/dist/packages/stdja-mini.satyh`
  package, written in SATySFi and loaded through `@require:`.
- **Backend** (`rustyfi-backend`): `Length`, horzBox-vocabulary box/glue
  model, Knuth–Plass optimal line breaking (glue-breakpoint DP with
  badness/demerits per lineBreak.ml), single-column page breaking.
- **PDF** (`rustyfi-pdf`): base-14 Helvetica by default; TrueType metrics +
  CID/Type0 embedding with ToUnicode (`TtfFontStore`/`render_pdf_ttf`) for
  real fonts (CLI selection pending).
- **Chimera CLI**: one multicall binary dispatching on argv[0] —
  `rustyfi-rust` (compile + subcommands), `rustyfi` (compile), and
  `satyrographos` (package manager, plan phases 1–4 all implemented:
  `install`/`uninstall`/`list`/`status`/`search`/`update`; local paths,
  tar.gz archives, upstream `Satyristes` packages via a built-in
  S-expression reader, project `Satyrfile.toml` + lockfile with
  reconcile-driven installs, and sha256-verified remote registries —
  git or plain-dir indexes, HTTP behind an off-by-default feature;
  see docs/chimera-satyrographos-plan.md). `--target-version` selects
  the SATySFi language version (0.0.6 implemented; 0.1 recognized and
  rejected honestly).

## Layout

```
crates/
  rustyfi-syntax/    Span, Token, mode-stack lexer, ParseStream, grammar (CST)
  rustyfi-backend/   Length, boxes/glue, Context, FontMetrics seam, line/page break
  rustyfi-lang/      Ast, elaborate (typecheck seam), Value, evaluator, primitives
  rustyfi-loader/    @require/@import resolution, dependency graph, load order
  rustyfi-pdf/       pdf-writer backend + base-14 metrics
  rustyfi-satyrographos/  package manager: manifest/receipts/atomic install
  rustyfi-cli/       chimera binary: rustyfi-rust / rustyfi / satyrographos
```

Requires a checkout of the `syan` parser framework as a sibling `../syan2-ergo`
(path dependency; currently the `api-ergonomics` branch of `yasuo-ozu/syan`).

## Testing & CI

`cargo test --workspace` runs the unit/integration suite (315+ tests).

**Corpus regression** — `crates/rustyfi-syntax/tests/corpus.rs` runs the
lexer and parser over the author's real-world SATySFi packages
(`github.com/yasuo-ozu/rustyfi-*`) and guards against regressions. Because
this port is a **v0.0.x subset without stdlib loading**, real packages do not
compile end-to-end — most do not even fully parse yet — so the harness does
not assert "must compile". It enforces what is meaningful for a growing
front-end: (1) the frontend must **never panic** on real input, (2) a
**lex-coverage floor** (our `lexer.mll` port handles ~89% of real files; the
rest are the unsupported `@`-positioned string literal), and (3) a **parse
ratchet** (the count that fully parses is tracked and only ratchets up as the
grammar grows). It is driven by `$RUSTYFI_CORPUS_DIR` (a `:`-separated list of
repo roots) and **skips** when that is unset, so a plain `cargo test` without
the corpus checked out stays green:

```console
$ RUSTYFI_CORPUS_DIR=../rustyfi-class-jlreq:../rustyfi-latexcmds:../rustyfi-xpath \
    cargo test -p rustyfi-syntax --test corpus -- --nocapture
```

**GitHub Actions** (`.github/workflows/ci.yml`) runs the suite plus the corpus
job (cloning the `rustyfi-*` packages). Because `syan` is a path dependency,
CI checks out `yasuo-ozu/syan` (at `$SYAN_REF`, default `api-ergonomics`) into
a sibling `syan2-ergo/`. **That branch must be pushed to `yasuo-ozu/syan`** for
CI to compile; once it merges to syan's default branch, set `SYAN_REF` to
`main`.

## Roadmap

1. ✅ Thin end-to-end slice
2. ◕ Full surface language — done through phase 2b (binops, if/match,
   let-rec, tuples, ctors, commands, mutables, fields, items, math syntax,
   modules, multi-file loading); remaining: command macros (`\cmd@`),
   optional args at runtime (parse-and-reject today), `(| e with |)` inside
   signatures, `Mod.(…)`, tabular `|` separators, stages
3. ◕ Real typechecker — HM inference (Rémy levels, let-polymorphism, value
   restriction), row-polymorphic records, user variants, command-argument
   checking inside quoted text; a mandatory pipeline stage. Remaining:
   module signature enforcement, math command types, exhaustiveness, stages
4. ◕ Stdlib loading proven — `document`/`+p`/`\emph` live in the in-repo
   `stdja-mini` package (SATySFi source, typechecked, loaded via
   `@require:`); remaining: the broader vminst inventory (~200
   instructions) and compiling the real upstream `dist/` classes
5. ◔ Real fonts — TrueType metrics + CID/Type0 embedding with ToUnicode
   done (`TtfFontStore`/`render_pdf_ttf`; CLI still defaults to base-14);
   remaining: CLI font selection, subsetting, shaping/kerning, CFF,
   Unicode line-break classes
6. ◔ Paragraph/page model — Knuth–Plass line breaking done (drop-in DP,
   badness/demerits per lineBreak.ml); remaining: discretionaries/
   hyphenation, full page model, graphics
7. Math mode; 8. images/annotations/cross-refs; 9. polish/perf
