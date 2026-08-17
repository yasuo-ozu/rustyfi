//! Prove the syan derives work over the custom SATySFi atoms: one derived
//! struct/enum parse plus a token-level unparse round-trip.

use rustyfi_syntax::leaf::*;
use rustyfi_syntax::token::Atom;
use rustyfi_syntax::{lex, Span};
use syan::parse::{Parse, Unparse};

#[derive(syan::parse::Parse, syan::parse::Unparse, Debug)]
struct LetStmt {
    let_kw: KwLet,
    name: VarTok,
    eq: DefEqTok,
    value: IntTok,
}

// Non-recursive on purpose: self-recursion in a plain derive is an E0275
// where-cycle; the real grammar uses a `#[recurse]` module for that.
#[derive(syan::parse::Parse, syan::parse::Unparse, Debug)]
enum Atomic {
    Int(IntTok),
    Length(LengthTok),
    Var(VarTok),
    Paren(ParenGroup<Box<IntTok>>),
}

// `Vec<Atom>` is itself a parse source now (syan's `IntoParseStream for Vec`).
fn atoms(src: &str) -> Vec<Atom> {
    let mut v = lex(src).unwrap();
    v.pop(); // drop Eoi for these fragment tests
    v
}

#[test]
fn derived_struct_parse() {
    let stmt: LetStmt = Parse::parse(atoms("let answer = 42")).unwrap();
    assert_eq!(stmt.name.name, "answer");
    assert_eq!(stmt.value.value, 42);
    assert_eq!(stmt.let_kw.0.start.line, 1);
}

#[test]
fn derived_enum_backtracks() {
    let a: Atomic = Parse::parse(atoms("3pt")).unwrap();
    assert!(matches!(a, Atomic::Length(_)));
    let a: Atomic = Parse::parse(atoms("(7)")).unwrap();
    match a {
        Atomic::Paren(g) => assert_eq!(g.slot.value, 7),
        other => panic!("expected paren, got {other:?}"),
    }
}

#[test]
fn unparse_round_trips_tokens() {
    let mut orig = lex("let answer = 42").unwrap();
    orig.pop();
    let stmt: LetStmt = Parse::parse(orig.clone()).unwrap();
    let mut out = Vec::<Atom>::new();
    stmt.unparse(&mut (&mut out)).unwrap();
    let orig_toks: Vec<_> = orig.into_iter().map(|a| a.slot).collect();
    let out_toks: Vec<_> = out.into_iter().map(|a| a.slot).collect();
    assert_eq!(orig_toks, out_toks);
}

#[test]
fn parse_error_carries_failure_span() {
    // `let answer 42` — the parser reaches the missing `=`.
    let res: Result<LetStmt, _> = Parse::parse(lex("let answer 42").unwrap());
    let err = res.unwrap_err();
    // syan's span-carrying `ParseError` yields the failure span directly
    // (replacing the old high-water mark). The input is single-line, so the
    // recovered span sits on line 1.
    let span = err.span_of::<Span>().expect("parse error carries a span");
    assert_eq!(span.start.line, 1);
}

/// Bug 5, leaf level: `OpNameTok` (the `( ‹op› )` naming form) now accepts
/// `!`/`before` via `NamingOpTok`, not just an ordinary [`BinOpTok`].
#[test]
fn op_name_tok_accepts_bang_and_before() {
    let toks = atoms("(!)");
    let op: OpNameTok = Parse::parse(toks).unwrap();
    assert_eq!(op.name, "!");

    let toks = atoms("(before)");
    let op: OpNameTok = Parse::parse(toks).unwrap();
    assert_eq!(op.name, "before");

    // Every ordinary `BinOpTok` alternative still works through the naming
    // form too (unwidened by this change).
    let toks = atoms("(+++)");
    let op: OpNameTok = Parse::parse(toks).unwrap();
    assert_eq!(op.name, "+++");
}

/// `!`/`before` inside `( .. )` must NOT leak into the ordinary infix
/// `BinOpTok` (used for the operator-chain grammar, `cst.rs`'s `OpRhs`) —
/// this is the naming-only scoping the fix is required to preserve.
#[test]
fn bin_op_tok_still_excludes_bang_and_before() {
    let toks = atoms("!");
    let res: Result<BinOpTok, _> = Parse::parse(toks);
    assert!(res.is_err(), "`!` must stay excluded from BinOpTok (infix chain)");

    let toks = atoms("before");
    let res: Result<BinOpTok, _> = Parse::parse(toks);
    assert!(res.is_err(), "`before` must stay excluded from BinOpTok (infix chain)");
}
