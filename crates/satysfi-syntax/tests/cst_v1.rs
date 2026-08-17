//! `cst_v1` round-trip and negative tests — the S5 slice's own test plan
//! items 2 and 3 (`docs/plans/satysfi-0-1-0-support.md` cst_v1 design spec
//! §7): parse+unparse a hand-written 0.1-syntax snippet and assert every
//! `FileV1`/`BindV1`/`Expr` node round-trips losslessly (`Parse` ∘ `Unparse`
//! = id at the token level), plus assert that SATySFi-0.1-invalid input is
//! rejected. Lowering/e2e tests (spec items 4-6) are out of scope for this
//! slice — `satysfi-syntax` only produces `FileV1`/`parse_file_v1`.

use satysfi_syntax::cst_v1::{self, parse_file_v1};
use satysfi_syntax::lexer::lex_with_version;
use satysfi_syntax::token::{Atom, Token};
use satysfi_syntax::version::SatysfiVersion;
use syan::parse::Unparse;

fn assert_roundtrip_v1(src: &str) {
    let file = parse_file_v1(src).unwrap_or_else(|e| panic!("v1 parse failed on {src:?}: {e}"));
    let mut out = Vec::<Atom>::new();
    file.unparse(&mut (&mut out)).unwrap();
    let orig: Vec<Token> = lex_with_version(src, SatysfiVersion::V0_1)
        .unwrap()
        .into_iter()
        .map(|a| a.slot)
        .collect();
    let re: Vec<Token> = out.into_iter().map(|a| a.slot).collect();
    assert_eq!(orig, re, "v1 token round-trip mismatch for {src:?}");
}

// ---- FileV1::Document -------------------------------------------------------

#[test]
fn document_minimal_expression() {
    assert_roundtrip_v1("3");
    assert_roundtrip_v1("f x y");
    assert_roundtrip_v1("(f 1) 2.5 3pt");
}

#[test]
fn document_let_forms() {
    assert_roundtrip_v1("let x = 1 in x");
    assert_roundtrip_v1("let f a b = a in f 1 2");
    assert_roundtrip_v1("let rec f n = f n in f");
    // `let pat = value in body` — a non-bare-variable pattern.
    assert_roundtrip_v1("let (a, b) = (1, 2) in a");
    assert_roundtrip_v1("let [a, b] = [1, 2] in a");
}

#[test]
fn document_let_open() {
    // `let open Name in body` — 0.1 requires the leading `let`, unlike
    // 0.0.6's bare `open Name in body`.
    assert_roundtrip_v1("let open M in x");
}

#[test]
fn document_if_and_fun() {
    assert_roundtrip_v1("if x then 1 else 2");
    assert_roundtrip_v1("if a then if b then 1 else 2 else 3");
    assert_roundtrip_v1("fun x -> x");
    assert_roundtrip_v1("fun x y -> x");
}

#[test]
fn document_match_with_end() {
    // Mandatory `end` — the one structural addition over 0.0.6's `match`.
    assert_roundtrip_v1("match x with | 0 -> 1 | n -> n end");
    assert_roundtrip_v1("match l with | [] -> 0 | x :: rest -> x end");
    // No `when` guards in 0.1: a bare `pat -> body` per arm.
    let file = parse_file_v1("match x with | 0 -> 1 | n -> n end").unwrap();
    let cst_v1::FileV1::Document { body, .. } = file else {
        panic!("expected a document file");
    };
    let cst_v1::ast::Expr::Match { end_kw: _, rest, .. } = body else {
        panic!("expected a match expression");
    };
    assert_eq!(rest.len(), 1, "one `| n -> n` continuation");
}

#[test]
fn document_comma_record_and_list() {
    // `,`-separated record and list — the headline 0.1 delta.
    assert_roundtrip_v1("(| title = 1, size = 2 |)");
    assert_roundtrip_v1("(| |)");
    assert_roundtrip_v1("[1, 2, 3]");
    assert_roundtrip_v1("[]");
    assert_roundtrip_v1("(| r with a = 1, b = 2 |)");
}

#[test]
fn document_tuple() {
    assert_roundtrip_v1("(1, 2)");
    assert_roundtrip_v1("(1, 2, 3)");
    assert_roundtrip_v1("(1)");
}

#[test]
fn document_field_access() {
    assert_roundtrip_v1("x#y");
    assert_roundtrip_v1("x#y#z");
}

#[test]
fn document_overwrite() {
    assert_roundtrip_v1("c <- 1 + 2");
}

#[test]
fn document_operator_chain() {
    assert_roundtrip_v1("1 + 2 * 3 - 4");
    assert_roundtrip_v1("1 :: 2 :: []");
}

#[test]
fn document_inline_and_block_text() {
    assert_roundtrip_v1("{ Hello, world! }");
    assert_roundtrip_v1(r"{ Hello \emph{strong} text }");
    assert_roundtrip_v1("'< +p { one } +p { two } >");
}

#[test]
fn document_math() {
    assert_roundtrip_v1("${x^2+\\frac{a}{b}}");
}

/// `(command \cmd)` — a first-class reference to an inline command as a
/// value (upstream `parser_v1.mly:906-908`), needed by `v01-mini.satyh`'s
/// `get-initial-context 440pt (command \math)` (§2.1 of the finale spec).
#[test]
fn document_command_reference() {
    assert_roundtrip_v1(r"(command \math)");

    let file = parse_file_v1(r"(command \math)").unwrap();
    let cst_v1::FileV1::Document { body, .. } = file else {
        panic!("expected a document file");
    };
    let cst_v1::ast::Expr::Ops(chain) = body else {
        panic!("expected an operator-chain expression");
    };
    let cst_v1::ast::Atomic::Paren { inner, .. } = chain.head.head else {
        panic!("expected a parenthesized atomic");
    };
    let inner_expr: &cst_v1::ast::Expr = &inner.first;
    let cst_v1::ast::Expr::Ops(inner_chain) = inner_expr else {
        panic!("expected the parenthesized body to be an operator chain");
    };
    let cst_v1::ast::Atomic::Command { name, .. } = &inner_chain.head.head else {
        panic!("expected an `Atomic::Command` inside the parens");
    };
    let satysfi_syntax::leaf::AnyHorzCmdTok::Plain(cmd) = name else {
        panic!("expected a sigil-only command name");
    };
    assert_eq!(cmd.name, "\\math");
}

// ---- FileV1::Library ---------------------------------------------------------

#[test]
fn library_val_forms() {
    assert_roundtrip_v1(
        "module M = struct\n\
         val x = 1\n\
         end",
    );
}

/// The `stdja-mini.satyh` transliteration from the cst_v1 design spec's §3
/// (Slice-1 `BindV1` ground truth), covering all three `BindV1` arms plus a
/// comma-record inside `document`'s call, one tuple (`page-break`'s
/// argument), and one `let open M in`-shaped local use elsewhere in the
/// file's expr layer (via a small standalone snippet below — `stdja-mini`
/// itself has no module reference to open).
#[test]
fn library_v01_mini_transliteration() {
    let src = "\
module StdjaMini = struct
val document record bt =
  let ctx = get-initial-context 160mm () in
  page-break (210mm, 297mm) ctx (read-block ctx bt)

val block ctx +p it =
  line-break true true ctx (read-inline ctx it ++ inline-fil)

val inline ctx \\emph it =
  read-inline (set-font-key 2 ctx) it

val inline ctx \\bold it =
  read-inline (set-font-key 1 ctx) it
end
";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, name, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(name.name, "StdjaMini");
    assert_eq!(binds.len(), 4);
    assert!(matches!(binds[0], cst_v1::BindV1::Value { .. }));
    assert!(matches!(binds[1], cst_v1::BindV1::ValueBlock { .. }));
    assert!(matches!(binds[2], cst_v1::BindV1::ValueInline { .. }));
    assert!(matches!(binds[3], cst_v1::BindV1::ValueInline { .. }));
}

#[test]
fn library_with_headers() {
    // `@require:`/`@import:` headers are byte-identical between 0.0.6 and
    // 0.1 — a 0.1-syntax library can still carry them.
    assert_roundtrip_v1(
        "@require: stdlib\n\
         module M = struct\n\
         val x = 1\n\
         end",
    );
}

// ---- Negative tests (spec item 3) --------------------------------------------

#[test]
fn stage_header_is_a_lex_error_under_v0_1() {
    let err = parse_file_v1("@stage: 0\nlet x = 1 in x").unwrap_err();
    assert!(
        err.message.contains("@stage:") || err.message.to_lowercase().contains("stage"),
        "expected the error to mention the '@stage:' header, got: {}",
        err.message
    );
}

#[test]
fn hyphenated_let_forms_are_rejected_under_v0_1() {
    // `let-rec`/`let-inline`/`let-block`/`let-mutable` have no counterpart
    // in the 0.1 grammar (0.1 spells these `let rec`/`val inline`/`val
    // block`/... instead) — the v1 grammar has no rule that ever consumes
    // these tokens, so feeding them is a parse error.
    assert!(parse_file_v1("let-rec f n = f n in f").is_err());
    assert!(parse_file_v1("let-inline ctx \\emph it = it in x").is_err());
    assert!(parse_file_v1("let-block ctx +p it = it in x").is_err());
    assert!(parse_file_v1("let-mutable c <- 0 in c").is_err());
}

#[test]
fn when_while_before_are_rejected_under_v0_1() {
    // 0.1's grammar has no `WHEN`/`WHILE`/`BEFORE` tokens at all (confirmed
    // by grep of `parser_v1.mly`); the v1 `Expr`/`MatchArm` grammar has no
    // rule for any of them.
    assert!(parse_file_v1("match x with | n when n -> 1 end").is_err());
    assert!(parse_file_v1("while true do 1").is_err());
    assert!(parse_file_v1("1 before 2").is_err());
}

#[test]
fn semicolon_separated_list_is_rejected_under_v0_1() {
    // 0.1's `Atomic::List`/`Atomic::Record` are strictly `,`-separated.
    assert!(parse_file_v1("[1; 2]").is_err());
    assert!(parse_file_v1("(| a = 1; b = 2 |)").is_err());
}

#[test]
fn match_without_end_is_rejected_under_v0_1() {
    // Unlike 0.0.6, `end` is mandatory.
    assert!(parse_file_v1("match x with | 0 -> 1 | n -> n").is_err());
}

#[test]
fn bare_open_without_let_is_rejected_under_v0_1() {
    // 0.1 requires `let open Name in body`; a bare `open Name in body`
    // (0.0.6's shape) has no v1 grammar rule.
    assert!(parse_file_v1("open M in x").is_err());
}

#[test]
fn v0_0_6_source_is_not_valid_v0_1_library_syntax() {
    // The plain 0.0.6 stdja-mini.satyh (top-level `let`/`let-inline`/
    // `let-block` with no enclosing `module ... = struct ... end`) has no
    // 0.1 `FileV1` shape at all: neither a bare `Document` body (it isn't
    // one expression — it's several top-level bindings with no `in`) nor a
    // `Library` (no `module` header).
    let v006_src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib-satysfi/dist/packages/stdja-mini.satyh"
    ))
    .expect("lib-satysfi/dist/packages/stdja-mini.satyh must exist");
    assert!(parse_file_v1(&v006_src).is_err());
}
