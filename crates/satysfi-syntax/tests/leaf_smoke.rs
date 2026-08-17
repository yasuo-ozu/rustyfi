//! Prove the syan derives work over the custom SATySFi atoms: one derived
//! struct/enum parse plus a token-level unparse round-trip.

use satysfi_syntax::leaf::*;
use satysfi_syntax::token::Atom;
use satysfi_syntax::{lex, TokenStream};
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

fn atoms(src: &str) -> TokenStream {
    let mut v = lex(src).unwrap();
    v.pop(); // drop Eoi for these fragment tests
    TokenStream::new(v)
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
    let stmt: LetStmt = Parse::parse(TokenStream::new(orig.clone())).unwrap();
    let mut out = Vec::<Atom>::new();
    stmt.unparse(&mut (&mut out)).unwrap();
    let orig_toks: Vec<_> = orig.into_iter().map(|a| a.slot).collect();
    let out_toks: Vec<_> = out.into_iter().map(|a| a.slot).collect();
    assert_eq!(orig_toks, out_toks);
}

#[test]
fn parse_error_reports_high_water() {
    let mut ts = TokenStream::new(lex("let answer 42").unwrap());
    let res: Result<LetStmt, _> = Parse::parse(&mut ts);
    assert!(res.is_err());
    // The parser got as far as the missing `=`.
    assert_eq!(ts.high_water_span().start.line, 1);
}
