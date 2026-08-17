//! Parse → unparse round-trip at the token level: the CST is lossless, so
//! unparsing must reproduce exactly the lexed token sequence (spans aside).

use satysfi_syntax::cst::{self, parse_file};
use satysfi_syntax::token::{Atom, Token};
use satysfi_syntax::lex;
use syan::parse::Unparse;

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
    let Some(cst::ast::Expr::Ops(chain)) = file.body else {
        panic!("expected Ops body");
    };
    let cst::ast::Atomic::InlineText { elems, .. } = chain.head.head else {
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
    let cst::ast::Expr::Ops(arg_chain) = &**args else {
        panic!("expected application chain");
    };
    assert_eq!(arg_chain.head.args.len(), 1, "head + one more argument");
}

#[test]
fn deep_nesting_is_unbounded() {
    // Nest inline commands well past the engine depth (4): the runtime
    // re-entry must keep going. Phase 2b widened several of the enums on
    // this recursion path (`Atomic`, `AppExpr`/`AppArg`, `InlineElem`, ...),
    // which grows each stack frame along the way; run on a thread with a
    // generously large stack rather than relying on the default (8 MiB),
    // which some default test-harness configurations no longer clear at
    // this depth.
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
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || assert_roundtrip(&src))
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn parse_errors_have_positions() {
    // `let` with no body expression.
    let err = parse_file("let x = in x").unwrap_err();
    assert!(err.span.start.line >= 1);
    // Phase 2b adds the math grammar (`Atomic::MathText`), so `${x}` now
    // parses; a shape math genuinely rejects (an unclosed group) still
    // reports a position.
    assert_roundtrip("${x}");
    let err = parse_file("${x").unwrap_err();
    assert!(err.span.start.line >= 1);
}

#[test]
fn deep_parse_error_span_reaches_past_first_line() {
    // A genuine failure on line 2 (a stray `)` where an expression is
    // expected). syan's `ParseError` now carries the failure span directly,
    // replacing the old high-water mark. Because our `Span::migrate` is a
    // *union* (min-start .. max-end), the reported span *starts* on line 1 (the
    // first attempted token); the signal that the parser progressed past line 1
    // is the span *end*, which lands on line 2.
    let err = parse_file("let x = 1\nlet y = )").unwrap_err();
    assert!(err.span.end.line >= 2);
}

// ---- phase 2a: operators, if/match/let-rec, patterns, tuples, ctors, ----
// ---- text embeds, type declarations -------------------------------------

#[test]
fn operator_chains() {
    assert_roundtrip("1 + 2 * 3 - 4");
    assert_roundtrip("a +. b");
    assert_roundtrip("x ^ y");
    assert_roundtrip("1 :: 2 :: []");
    assert_roundtrip("a mod b");
}

#[test]
fn if_then_else() {
    assert_roundtrip("if x then 1 else 2");
    assert_roundtrip("if a then if b then 1 else 2 else 3");
}

#[test]
fn let_and_let_rec_local() {
    assert_roundtrip("let x = 1 in x + 1");
    assert_roundtrip("let-rec f n = f n in f");
    assert_roundtrip("let-rec even n = odd n and odd n = even n in even 4");
}

#[test]
fn match_expressions() {
    assert_roundtrip("match x with | 0 -> `a` | n when n -> `b` | _ -> `c`");
    assert_roundtrip("match l with | [] -> 0 | x :: rest -> x");
}

#[test]
fn tuples_and_parens() {
    assert_roundtrip("(1, 2)");
    assert_roundtrip("(1, 2, 3)");
    assert_roundtrip("(1)");
}

#[test]
fn constructors() {
    assert_roundtrip("Some 1");
    assert_roundtrip("None");
    assert_roundtrip("match o with | Some x -> x | None -> 0");
}

#[test]
fn let_inline_top_level() {
    assert_roundtrip("let-inline ctx \\bold inner = x in y");
}

#[test]
fn text_embeds() {
    assert_roundtrip("{ a #name; b }");
    assert_roundtrip("'< #content; >");
}

#[test]
fn type_declaration() {
    assert_roundtrip("type t = | A | B of int\nin 0");
}

#[test]
fn bar_is_not_a_binop() {
    // `|` is the match-arm separator, never an infix operator on its own
    // (only the multi-char `BinopBar` payload tokens like `|>` are).
    assert!(parse_file("1 + | 2").is_err());
}

#[test]
fn unary_minus() {
    assert_roundtrip("- x");
    assert_roundtrip("1 - - 2");
}

#[test]
fn pattern_shapes() {
    assert_roundtrip("match x with | (a, b) -> a");
    assert_roundtrip("match x with | y as z -> z");
    assert_roundtrip("match x with | [a; b] -> a | _ -> b");
    assert_roundtrip("match x with | Some (a, b) -> a | None -> c");
}

#[test]
fn type_declaration_shapes() {
    assert_roundtrip("type 'a opt = | N | S of 'a\nin 0");
    assert_roundtrip("type t = A\nin 0");
    assert_roundtrip("type f = | F of int -> int\nin 0");
}

#[test]
fn match_binds_greedily() {
    // A nested match absorbs the following arms (same resolution as the
    // OCaml grammar).
    let file = parse_file("match x with | 0 -> match y with | 1 -> 1 | _ -> 3").unwrap();
    let Some(cst::ast::Expr::Match { first, rest, .. }) = file.body else {
        panic!("expected match");
    };
    assert!(rest.is_empty(), "outer match has a single arm");
    let cst::ast::Expr::Match { rest: inner_rest, .. } = &*first.body.0 else {
        panic!("expected nested match body");
    };
    assert_eq!(inner_rest.len(), 1, "inner match took the remaining arm");
}

// ---- phase 2b: mutables, deref, field access, record update, optional ----
// ---- args, itemize, math, modules, library files ------------------------

#[test]
fn mutables_and_sequencing() {
    assert_roundtrip("let-mutable c <- 0 in c");
    assert_roundtrip("c <- 1 + 2");
    assert_roundtrip("while !c < 3 do c <- !c + 1");
    assert_roundtrip("a before b");
}

#[test]
fn deref_unop() {
    assert_roundtrip("!x");
    assert_roundtrip("!!x");
    assert_roundtrip("f !x !y");
}

#[test]
fn field_access() {
    assert_roundtrip("x#y");
    assert_roundtrip("x#y#z");
}

#[test]
fn record_update() {
    assert_roundtrip("(| r with a = 1 |)");
    assert_roundtrip("(| r with a = 1; b = 2 |)");
}

#[test]
fn optional_application_args() {
    assert_roundtrip("f ?:(1) x");
    assert_roundtrip("f ?* x");
}

#[test]
fn itemize_markers() {
    let file = parse_file("{ * a ** b }").unwrap();
    let Some(cst::ast::Expr::Ops(chain)) = file.body else {
        panic!("expected Ops body");
    };
    let cst::ast::Atomic::InlineText { elems, .. } = chain.head.head else {
        panic!("expected inline text");
    };
    assert!(matches!(elems[0], cst::ast::InlineElem::ItemBullet(_)));
    assert!(matches!(elems[2], cst::ast::InlineElem::ItemBullet(_)));
    assert_roundtrip("{ * a ** b }");
}

#[test]
fn math_round_trips() {
    assert_roundtrip("${x^2+\\frac{a}{b}}");
    assert_roundtrip("${a_1'}");
    assert_roundtrip("${\\cmd!(3){x}}");
    assert_roundtrip("{ a ${x+y} b }");
}

#[test]
fn modules_and_open() {
    assert_roundtrip(
        "module M = struct\n\
         let x = 1\n\
         end\n\
         open M in M.x",
    );
    assert_roundtrip(
        "module Outer = struct\n\
         module Inner = struct\n\
         let y = 2\n\
         end\n\
         end",
    );
}

#[test]
fn library_file_has_no_body() {
    let file = parse_file("let x = 1").unwrap();
    assert!(file.in_kw.is_none());
    assert!(file.body.is_none());
    assert_roundtrip("let x = 1");
    // Ordinary (non-library) forms still work unchanged.
    assert_roundtrip("let x = 1 in x");
    assert_roundtrip("3");
}
