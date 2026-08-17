# satysfi-rust-converted

A native Rust clone of [SATySFi](https://github.com/gfngfn/SATySFi) (reference:
upstream **v0.0.6**), using the [syan2](../syan2) parser framework for the
grammar.

## Status: milestone 1 — thin end-to-end slice

A small `.saty` document compiles to a real PDF with wrapped, justified text:

```console
$ cargo run -p satysfi-cli -- crates/satysfi-cli/tests/fixtures/minimal.saty -o out.pdf
```

What works:

- **Lexer** (`satysfi-syntax`): full case-for-case port of the v0.0.6
  `lexer.mll` — five mode states (program / vertical / horizontal / active /
  math) with the same stack discipline, eagerly lexing the whole file.
- **Parser**: syan2 derives over custom `WithSpan<Token, Span>` atoms; a
  `#[recurse]` grammar module for the Expr ↔ text SCC. Milestone-1 subset:
  headers, top-level `let`, `fun`, application, records, lists, string/int/
  float/length literals, inline text with `\cmd` and block text with `+cmd`.
  Token-level `Unparse` round-trip is tested.
- **Elaboration** (`satysfi-lang`): CST → `Ast` with scope resolution; the
  function signature is the seam where the phase-3 HM typechecker slots in.
- **Evaluator**: tree-walker with closures and a `PrimDef` registry shaped
  for incremental porting of the vminst primitive inventory (`read-inline`,
  `read-block`, `line-break`, `page-break` under their real names;
  `document`, `+p`, `\emph` are milestone-1 natives pending real stdlib
  loading).
- **Backend** (`satysfi-backend`): `Length`, horzBox-vocabulary box/glue
  model, greedy first-fit line breaking with glue justification (same input
  model as the future Knuth–Plass port), single-column page breaking.
- **PDF** (`satysfi-pdf`): `pdf-writer` output with base-14 Helvetica
  (regular/bold/oblique) and hardcoded AFM advance tables — WinAnsi/ASCII
  text only until real font loading (phase 5).

## Layout

```
crates/
  satysfi-syntax/    Span, Token, mode-stack lexer, ParseStream, grammar (CST)
  satysfi-backend/   Length, boxes/glue, Context, FontMetrics seam, line/page break
  satysfi-lang/      Ast, elaborate (typecheck seam), Value, evaluator, primitives
  satysfi-pdf/       pdf-writer backend + base-14 metrics
  satysfi-cli/       satysfi-rust <in.saty> -o <out.pdf>
```

Requires a checkout of `syan2` at `../syan2` (path dependency).

## Roadmap

1. ✅ Thin end-to-end slice
2. Full surface language (binops, if/match/patterns, let-rec, modules parse,
   `@require:`/`@import:` multi-file loading)
3. Real typechecker (HM + rows + command types, behind the elaborate seam)
4. Full primitive inventory + loading the real `dist/` stdlib
5. Real fonts (ttf-parser + subsetter, CID/Type0) and Unicode line breaking
6. Knuth–Plass line breaking, full page model, graphics, hyphenation
7. Math mode; 8. images/annotations/cross-refs; 9. polish/perf
