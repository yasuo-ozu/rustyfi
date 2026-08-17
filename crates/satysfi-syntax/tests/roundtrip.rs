//! Parse → unparse round-trip at the token level: the CST is lossless, so
//! unparsing must reproduce exactly the lexed token sequence (spans aside).

use satysfi_syntax::cst::{self, parse_file};
use satysfi_syntax::token::{Atom, Token};
use satysfi_syntax::{lex, TokenStream};
use syan::parse::{Parse, Unparse};

fn assert_roundtrip(src: &str) {
    let file = parse_file(src).unwrap_or_else(|e| panic!("parse failed on {src:?}: {e}"));
    let mut out = Vec::<Atom>::new();
    file.unparse(&mut (&mut out)).unwrap();
    let orig: Vec<Token> = lex(src).unwrap().into_iter().map(|a| a.slot).collect();
    let re: Vec<Token> = out.into_iter().map(|a| a.slot).collect();
    assert_eq!(orig, re, "token round-trip mismatch for {src:?}");
}

#[test]
fn minimal_expression_file() {
    assert_roundtrip("3");
    assert_roundtrip("`hello`");
    assert_roundtrip("f x y");
    assert_roundtrip("(f 1) 2.5 3pt");
}

#[test]
fn headers_and_prelude() {
    assert_roundtrip("@require: stdjabook\nlet x = 1 in x");
    assert_roundtrip("let f a b = a in f 1 2");
    assert_roundtrip("let x = 1\nlet y = 2\nin y");
}

#[test]
fn records_lists_functions() {
    assert_roundtrip("(| title = {T}; size = 3pt |)");
    assert_roundtrip("(| |)");
    assert_roundtrip("[1; 2; 3]");
    assert_roundtrip("[]");
    assert_roundtrip("fun x -> x");
    assert_roundtrip("let apply = fun f x -> f x in apply");
    assert_roundtrip("()");
}

#[test]
fn inline_text() {
    assert_roundtrip("{ Hello, world! }");
    assert_roundtrip(r"{ Hello \emph{strong} text }");
    assert_roundtrip(r"{ nested \emph{a \emph{b} c} here }");
    assert_roundtrip(r"{\skip(3pt);gap}");
    assert_roundtrip("{}");
}

#[test]
fn block_text() {
    assert_roundtrip("'< +p { one } +p { two } >");
    assert_roundtrip("'<>");
    assert_roundtrip("'<+clear;>");
    assert_roundtrip("'<+sec(1)<+p{a}>>");
}

#[test]
fn document_shape() {
    assert_roundtrip(
        "@require: stdjabook\n\
         let title-str = `Milestone`\n\
         in\n\
         document (| title = {T}; author = {me} |) '<\n\
           +p { Hello, world! }\n\
           +p { Second \\emph{paragraph} here. }\n\
         >",
    );
}

#[test]
fn cmd_args_are_application_chains() {
    // `\cmd(1)(2)` — the two program args arrive as one application chain.
    let file = parse_file(r"{\cmd(1)(2);}").unwrap();
    let cst::ast::Expr::App { head, .. } = file.body else {
        panic!("expected App body");
    };
    let cst::ast::Atomic::InlineText { elems, .. } = head else {
        panic!("expected inline text");
    };
    assert_eq!(elems.len(), 1);
    let cst::ast::InlineElem::Cmd { tail, .. } = &elems[0] else {
        panic!("expected command");
    };
    let cst::ast::CmdTail::Args { args, semi } = tail else {
        panic!("expected args tail");
    };
    assert!(semi.is_some());
    let cst::ast::Expr::App { args: chain, .. } = args.as_ref() else {
        panic!("expected application chain");
    };
    assert_eq!(chain.len(), 1, "head + one more argument");
}

#[test]
fn deep_nesting_is_unbounded() {
    // Nest inline commands well past the engine depth (4): the runtime
    // re-entry must keep going.
    let depth = 64;
    let mut src = String::from("{");
    for _ in 0..depth {
        src.push_str(r"\emph{");
    }
    src.push('x');
    for _ in 0..depth {
        src.push('}');
    }
    src.push('}');
    assert_roundtrip(&src);
}

#[test]
fn parse_errors_have_positions() {
    // `let` with no body expression.
    let err = parse_file("let x = in x").unwrap_err();
    assert!(err.span.start.line >= 1);
    // Math mode is lexed but not in the milestone-1 grammar.
    assert!(parse_file("${x}").is_err());
}

#[test]
fn high_water_mark_is_useful() {
    let atoms = lex("let x = 1\nlet y = ()()bogus\nin x").unwrap();
    let mut ts = TokenStream::new(atoms);
    let res: Result<cst::File, _> = Parse::parse(&mut ts);
    // Parse may or may not fail here depending on grammar generosity; if it
    // fails the span must point past line 1.
    if res.is_err() {
        assert!(ts.high_water_span().start.line >= 2);
    }
}
