//! Parse → unparse round-trip at the token level: the CST is lossless, so
//! unparsing must reproduce exactly the lexed token sequence (spans aside).

use rustyfi_syntax::cst::{self, parse_file};
use rustyfi_syntax::token::{Atom, Token};
use rustyfi_syntax::lex;
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
fn stage_headers() {
    // `Header::Stage` — accepted and round-tripped like `@require:`/
    // `@import:` (see `cst.rs`'s doc comment: treated as an inert no-op).
    assert_roundtrip("@stage: persistent\nlet x = 1 in x");
    assert_roundtrip("@stage: 0\nlet x = 1 in x");
    assert_roundtrip("@stage: 1\nlet x = 1 in x");
    assert_roundtrip("@require: list\n@stage: persistent\nlet x = 1 in x");
}

#[test]
fn headers_and_prelude() {
    assert_roundtrip("@require: stdjabook\nlet x = 1 in x");
    assert_roundtrip("let f a b = a in f 1 2");
    assert_roundtrip("let x = 1\nlet y = 2\nin y");
}

/// `nonrecdecargpart` (`parser.mly:610-614`) — a NON-recursive `let` may put
/// a `|` between the name (or its `: τ` ascription) and the argument list,
/// which `let-rec` already had here but plain `let` did not. `azmath`'s
/// `util.satyh` opens with the ascribed form and failed at its first binding
/// without it.
///
/// Unlike `let-rec`'s, this `|` introduces NO further clauses: upstream's
/// `nonrecdecargpart` has no `nxrecdecpar` tail, so it is purely a separator.
#[test]
fn non_recursive_let_accepts_a_leading_bar() {
    // All four `nonrecdecargpart` alternatives, in the order they appear
    // upstream.
    assert_roundtrip("let f : int -> int = 1 in f");
    assert_roundtrip("let f : int -> int | x = x in f 1");
    assert_roundtrip("let f | x = x in f 1");
    assert_roundtrip("let f x = x in f 1");
    // Top-level (`TopLet`) as well as expression-level (`Expr::LetIn`).
    assert_roundtrip("let f : int -> int | x = x\nin f 1");
}

#[test]
fn records_lists_functions() {
    assert_roundtrip("(| title = {T}; size = 3pt |)");
    assert_roundtrip("(| |)");
    assert_roundtrip("[1; 2; 3]");
    assert_roundtrip("[]");
    assert_roundtrip("fun x -> x");
    assert_roundtrip("let apply = fun f x -> f x in apply");
    // `fun`'s parameters are full `patbot`s upstream (`parser.mly`'s
    // `argpats = list(patbot)`), not merely variables — a tuple-
    // destructuring parameter (used by the bundled `list.satyg`'s
    // `mapi-adjacent`) must parse and round-trip like any other `patbot`.
    assert_roundtrip("fun (a, b) x -> a");
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
    let cst::ast::CmdTail::Args { first, rest, semi } = tail else {
        panic!("expected args tail");
    };
    assert!(semi.is_some());
    assert!(matches!(&**first, cst::ast::AppArg::Atom { .. }));
    assert_eq!(rest.len(), 1, "head + one more argument");
}

#[test]
fn deep_nesting_is_unbounded() {
    // Nest inline commands well past the engine depth (4): the runtime
    // re-entry must keep going. The enums on this recursion path (`Atomic`,
    // `AppExpr`/`AppArg`, `InlineElem`, ...) make for large stack frames, so
    // run on a thread with a generously large stack rather than the default
    // (8 MiB), which some test-harness configurations no longer clear at
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
    // `${x}` parses (`Atomic::MathText`); a shape math genuinely rejects
    // (an unclosed group) still reports a position.
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

// ---- operators, if/match/let-rec, patterns, tuples, ctors, --------------
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
fn destructuring_let() {
    // `Expr::LetPatternIn` — a plain, non-recursive `let` whose target is a
    // general pattern rather than a bare variable (`list.satyg`'s
    // `mapi-adjacent` uses this: `let (_, acc) = .. in reverse acc`).
    assert_roundtrip("let (a, b) = (1, 2) in a");
    assert_roundtrip("let (_, acc) = (1, 2) in acc");
    assert_roundtrip("let Some (x) = y in x");
}

#[test]
fn multi_clause_pattern_let_rec() {
    // SATySFi's multi-clause pattern-matching function-definition sugar
    // (`option.satyg`/`list.satyg` use this pervasively, e.g. `let-rec map |
    // f [] = [] | f (x :: xs) = (f x) :: map f xs`).
    assert_roundtrip("let-rec map | f (None) = None | f (Some(v)) = Some(f v) in map");
    assert_roundtrip(
        "let-rec map\n\
         | f []        = []\n\
         | f (x :: xs) = (f x) :: map f xs\n\
         in map",
    );
    assert_roundtrip("let-rec filter | _ [] = [] | p (x :: xs) = filter p xs in filter");
    // A single clause with a non-variable pattern (no `|` continuation at
    // all) also exercises the general (match-based) desugaring path.
    assert_roundtrip("let-rec first (x :: xs) = x in first");
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
fn applied_and_product_types_in_signatures() {
    // Postfix type-constructor application (`'a option`, `'a list`) and
    // product types (`'a * 'b`) inside a `module .. : sig .. end` — the
    // shapes `option.satyg`/`list.satyg`'s signatures need (`TypeApp`/
    // `TypeProd`, `cst.rs`).
    assert_roundtrip(
        "module M : sig\n\
         val f : ('a -> 'b) -> 'a option -> 'b option\n\
         end = struct\n\
         let f g x = g x\n\
         end",
    );
    assert_roundtrip(
        "module M : sig\n\
         val g : ('a -> 'a -> bool) -> 'a -> ('a * 'b) list -> 'b option\n\
         end = struct\n\
         let g eq a l = None\n\
         end",
    );
    assert_roundtrip(
        "module M : sig\n\
         val h : ('a list) list -> 'a list\n\
         end = struct\n\
         let h l = []\n\
         end",
    );
}

#[test]
fn module_qualified_type_application() {
    // Bug 4: `<ty> Mod.t` — a module-qualified postfix type-constructor
    // application (`TypeApp::AppliedMod`, `cst.rs`), the gap blocking the
    // whole `satysfi-base` ecosystem (SLyDIFi/easytable/figbox/code-printer),
    // which spells this as `Eq.t`/`Ord.t`/`SlydifiParam.t`/`Frame.frame`.
    assert_roundtrip(
        "module M : sig\n\
         type 'a t\n\
         val make : 'a -> 'a t\n\
         end = struct\n\
         type 'a t = 'a\n\
         let make x = x\n\
         end\n\
         module N : sig\n\
         val x : int M.t\n\
         end = struct\n\
         let x = M.make 1\n\
         end\n\
         in 0",
    );
    // A type-variable argument (`'content Frame.frame`).
    assert_roundtrip(
        "module M : sig\n\
         type 'a t\n\
         val make : 'a -> 'a t\n\
         end = struct\n\
         type 'a t = 'a\n\
         let make x = x\n\
         end\n\
         module N : sig\n\
         val x : 'content M.t\n\
         end = struct\n\
         let x = M.make x\n\
         end\n\
         in 0",
    );
    // Bare (0-ary) qualified type name, e.g. `Eq.t` alone — no preceding
    // argument (`TypeAtom::NameMod`).
    assert_roundtrip(
        "module M : sig\n\
         type t\n\
         val x : M.t\n\
         end = struct\n\
         type t = int\n\
         let x = 1\n\
         end\n\
         in 0",
    );
}

/// Bug 4's real-world shape: `satysfi-base`'s `ord.satyg` defines `ordering
/// Eq.t` (an applied qualified ctor) as a `val` type in a signature — this
/// must parse into `TypeApp::AppliedMod` with the right `arg`/`ctor` split,
/// not merely round-trip.
#[test]
fn module_qualified_type_application_shape() {
    let file = parse_file(
        "module Ordering : sig\n\
         val eq : ordering Eq.t\n\
         end = struct\n\
         let eq = 0\n\
         end",
    )
    .unwrap();
    let [cst::TopBinding::Module { sig: Some(sig), .. }] = file.prelude.as_slice() else {
        panic!("expected a single `module .. : sig .. end` prelude binding, got {:?}", file.prelude);
    };
    let [cst::SigItem::Val { ty, .. }] = sig.items.as_slice() else {
        panic!("expected a single `val` sig item, got {:?}", sig.items);
    };
    let cst::ast::TypeExpr::Atom(prod) = ty else {
        panic!("expected a bare (arrow-less) type, got {ty:?}");
    };
    // `ordering Eq.t` → `head = ordering` (the argument), `rest = [Eq.t]` (the
    // module-qualified constructor, last atom).
    let cst::ast::TypeApp { head, rest } = &prod.first;
    assert!(matches!(head, cst::ast::TypeAtom::Name(n) if n.name == "ordering"));
    assert_eq!(rest.len(), 1);
    let cst::ast::TypeAtom::NameMod(ctor) = &rest[0] else {
        panic!("expected a NameMod constructor, got {:?}", rest[0]);
    };
    assert_eq!(ctor.mods, vec!["Eq".to_string()]);
    assert_eq!(ctor.name, "t");
}

/// Bug 4's OTHER reported gap — `satysfi-base`'s `'a 'e result` (TWO
/// un-parenthesized arguments before the ctor) — now parses: `TypeApp` is an
/// N-ary greedy atom run (`head` + `rest`, the last atom the constructor; see
/// `cst.rs`'s `TypeApp` doc comment), so `'a 'e result` is `head = 'a`,
/// `rest = ['e, result]`.
#[test]
fn multi_argument_postfix_type_application_parses() {
    let file = parse_file(
        "module M : sig\n\
         val x : 'a 'e result\n\
         end = struct\n\
         let x = 0\n\
         end",
    )
    .expect("N-ary postfix type application ('a 'e result) must parse");
    let [cst::TopBinding::Module { sig: Some(sig), .. }] = file.prelude.as_slice() else {
        panic!("expected a `module .. : sig .. end`, got {:?}", file.prelude);
    };
    let [cst::SigItem::Val { ty, .. }] = sig.items.as_slice() else {
        panic!("expected a single `val`, got {:?}", sig.items);
    };
    let cst::ast::TypeExpr::Atom(prod) = ty else {
        panic!("expected a bare type, got {ty:?}");
    };
    // head = 'a, rest = ['e, result] — two type-variable arguments then the
    // `result` constructor as the final atom.
    assert!(matches!(&prod.first.head, cst::ast::TypeAtom::Var(_)), "{:?}", prod.first.head);
    assert_eq!(prod.first.rest.len(), 2, "{:?}", prod.first.rest);
    assert!(matches!(&prod.first.rest[0], cst::ast::TypeAtom::Var(_)));
    assert!(matches!(&prod.first.rest[1], cst::ast::TypeAtom::Name(n) if n.name == "result"));
}

#[test]
fn naming_form_bang_and_before() {
    // Bug 5: `(!)`/`(before)` in the `( ‹op› )` NAMING form only
    // (`leaf.rs`'s `NamingOpTok`) — upstream's `binop` nonterminal (used
    // ONLY by the two naming productions, never infix chaining) accepts
    // `UNOP_EXCLAM`/`BEFORE`, unlike the ordinary infix operator chain.
    assert_roundtrip("let (!) x = x in !3");
    assert_roundtrip(
        "module M : sig\n\
         val (!) : int -> int\n\
         end = struct\n\
         let (!) x = x\n\
         end",
    );
    assert_roundtrip(
        "module M : sig\n\
         val (before) : int -> int -> int\n\
         end = struct\n\
         let (before) x y = x\n\
         end",
    );
    // The bare, atomic-expression form (`ast::Atomic::OpRef`) accepts them
    // too, since it shares `OpNameTok`/`NamingOpTok` with the naming form.
    assert_roundtrip("let (!) x = x in (!) 3");
    // Ordinary prefix `!`/`!!` dereference (a SEPARATE grammar path,
    // `AppExpr`'s `excl: Option<UnopExclamTok>`) must still parse — pinned
    // here as well as in `deref_unop`, to show the two paths don't interfere.
    assert_roundtrip("!x");
    assert_roundtrip("!!x");
    // Ordinary infix `before` (`OpChain`'s `BeforeTail`, a SEPARATE grammar
    // path from the naming form) must still parse.
    assert_roundtrip("a before b");
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

// ---- mutables, deref, field access, record update, optional -------------
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
fn command_call_with_leading_optional_args() {
    // `\ref?:(x){text}` / `\ref?*{text}` — an optional/omitted `narg`
    // *before* the mandatory group arg (`CmdTail::Args`'s `first`/`rest`,
    // each an `AppArg`, so unlike the general application chain's `AppExpr`
    // — whose head must be a plain atom — a command's *first* argument may
    // itself be `?:`/`?*`).
    assert_roundtrip("{ \\ref?:(x){text} }");
    assert_roundtrip("{ \\ref?*{text} }");
    assert_roundtrip("{ \\ref?:(x)?:(y){text} }");
}

#[test]
fn optional_argument_type_grammar() {
    // `?->` (optional-argument function arrow) and `ty?` (optional
    // command-argument type) — gap 2.
    assert_roundtrip(
        "module M : sig\n\
         val f : 'a -> config ?-> block-text -> document\n\
         end = struct\n\
         let f x c bt = bt\n\
         end",
    );
    assert_roundtrip(
        "module M : sig\n\
         direct +section : [string?; string?; inline-text; block-text] block-cmd\n\
         end = struct\n\
         end",
    );
    assert_roundtrip(
        "module M : sig\n\
         val g : [int; string?] math-cmd\n\
         end = struct\n\
         end",
    );
}

#[test]
fn math_command_names_plain_and_qualified() {
    use rustyfi_syntax::leaf::AnyMathCmdTok;

    let file = parse_file(r"${\cmd{x}}").unwrap();
    let Some(cst::ast::Expr::Ops(chain)) = file.body else {
        panic!("expected Ops body");
    };
    let cst::ast::Atomic::MathText { elems, .. } = chain.head.head else {
        panic!("expected math text");
    };
    let cst::ast::MathBot::Cmd { name, .. } = &elems[0].base else {
        panic!("expected a math command");
    };
    assert!(matches!(name, AnyMathCmdTok::Plain(t) if t.name == r"\cmd"));

    let file = parse_file(r"${\Mod.cmd{x}}").unwrap();
    let Some(cst::ast::Expr::Ops(chain)) = file.body else {
        panic!("expected Ops body");
    };
    let cst::ast::Atomic::MathText { elems, .. } = chain.head.head else {
        panic!("expected math text");
    };
    let cst::ast::MathBot::Cmd { name, .. } = &elems[0].base else {
        panic!("expected a math command");
    };
    match name {
        AnyMathCmdTok::Mod(t) => {
            assert_eq!(t.mods, vec!["Mod".to_string()]);
            assert_eq!(t.name, r"\cmd");
        }
        AnyMathCmdTok::Plain(_) => panic!("expected a qualified math command"),
    }
}

#[test]
fn math_lists() {
    // Gap 3: a leading `|` puts the math area in list mode (`mathblock`,
    // parser.mly:1059-1066). Parsing always accepted these — the shape is
    // documented here; the rejection was elaboration's.
    assert_roundtrip("${| a | b |}");
    assert_roundtrip("${|}");
    assert_roundtrip("${||}");
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
    assert_roundtrip("${\\Mod.cmd{x}}");
}

#[test]
fn math_optional_args_round_trip() {
    // Gap 4: `matharg`'s `?:`- supplied (`MathArg::Optional`), `?*`-omitted
    // (`MathArg::Omission`), and plain (`MathArg::Plain`) shapes, the latter
    // two exercising the `!`-escape body forms too (`MathArgBody`'s
    // `ParenEscape`).
    assert_roundtrip("${\\cmd?:{x}{y}}");
    assert_roundtrip("${\\cmd?*{y}}");
    assert_roundtrip("${\\cmd?:!(3){y}}");
}

#[test]
fn math_optional_args_cst_shape() {
    use rustyfi_syntax::leaf::AnyMathCmdTok;

    let file = parse_file(r"${\cmd?:{x}{y}}").unwrap();
    let Some(cst::ast::Expr::Ops(chain)) = file.body else {
        panic!("expected Ops body");
    };
    let cst::ast::Atomic::MathText { elems, .. } = chain.head.head else {
        panic!("expected math text");
    };
    let cst::ast::MathBot::Cmd { name, args } = &elems[0].base else {
        panic!("expected a math command");
    };
    assert!(matches!(name, AnyMathCmdTok::Plain(t) if t.name == r"\cmd"));
    assert_eq!(args.len(), 2);
    match &args[0] {
        cst::ast::MathArg::Optional { body, .. } => {
            assert!(matches!(body, cst::ast::MathArgBody::Math { .. }));
        }
        other => panic!("expected an Optional matharg, got {other:?}"),
    }
    match &args[1] {
        cst::ast::MathArg::Plain(body) => {
            assert!(matches!(body, cst::ast::MathArgBody::Math { .. }));
        }
        other => panic!("expected a Plain matharg, got {other:?}"),
    }

    let file = parse_file(r"${\cmd?*{y}}").unwrap();
    let Some(cst::ast::Expr::Ops(chain)) = file.body else {
        panic!("expected Ops body");
    };
    let cst::ast::Atomic::MathText { elems, .. } = chain.head.head else {
        panic!("expected math text");
    };
    let cst::ast::MathBot::Cmd { args, .. } = &elems[0].base else {
        panic!("expected a math command");
    };
    assert_eq!(args.len(), 2);
    assert!(matches!(args[0], cst::ast::MathArg::Omission(_)));
    match &args[1] {
        cst::ast::MathArg::Plain(body) => {
            assert!(matches!(body, cst::ast::MathArgBody::Math { .. }));
        }
        other => panic!("expected a Plain matharg, got {other:?}"),
    }
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
fn open_module_expression() {
    // `Mod.(e)` ≡ `open Mod in e` (`Atomic::OpenModule`).
    assert_roundtrip(
        "module M = struct\n\
         let x = 3\n\
         end\n\
         M.(x + 1)",
    );
    assert_roundtrip(
        "module M = struct\n\
         let x = 3\n\
         end\n\
         M.(x, x)",
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

#[test]
fn command_value() {
    // `(command \cmd)` — gap 1: a first-class reference to an inline
    // command's own binding.
    assert_roundtrip("let-inline \\m ctx = ctx in (command \\m)");
    assert_roundtrip("get-initial-context 100pt (command \\m)");
}

#[test]
fn sig_constraint_suffix() {
    // `constraint 'a :: (| l1 : ty1; … |)` as a per-item suffix on a
    // `SigItem` (gap 3; `parser.mly:526-530` — a per-item suffix, not a
    // standalone item).
    assert_roundtrip(
        "module M : sig\n\
         val document : 'a -> config ?-> block-text -> document\n\
         constraint 'a :: (| title : inline-text; author : inline-text |)\n\
         end = struct\n\
         let document x c bt = bt\n\
         end",
    );
    // Multi-field record kind, matching the real `stdja.satyh:29-34` shape.
    assert_roundtrip(
        "module M : sig\n\
         val document : 'a -> config ?-> block-text -> document\n\
         constraint 'a :: (|\n\
         title : inline-text;\n\
         author : inline-text;\n\
         show-toc : bool;\n\
         show-title : bool;\n\
         |)\n\
         end = struct\n\
         let document x c bt = bt\n\
         end",
    );
}

#[test]
fn stdja_sig_block_parses() {
    // The whole `sig … end` block of the real upstream `stdja.satyh:24-51`
    // (command values, command types, `?->`, and the `constraint` suffix
    // all together) — the acceptance gate. Trimmed to the constructs
    // this port models (no tuple-of-`string*float*float` font vals needed
    // for the gate, but included anyway since `TypeProd` already supports
    // them).
    assert_roundtrip(
        "module StdJa : sig\n\
         val default-config : config\n\
         val document : 'a -> config ?-> block-text -> document\n\
         constraint 'a :: (|\n\
         title : inline-text;\n\
         author : inline-text;\n\
         show-toc : bool;\n\
         show-title : bool;\n\
         |)\n\
         val font-latin-roman : string * float * float\n\
         direct \\ref : [string] inline-cmd\n\
         direct \\ref-page : [string] inline-cmd\n\
         direct \\figure : [inline-text; block-text] inline-cmd\n\
         direct +p : [inline-text] block-cmd\n\
         direct +pn : [inline-text] block-cmd\n\
         direct +section : [string?; string?; inline-text; block-text] block-cmd\n\
         direct +subsection : [string?; string?; inline-text; block-text] block-cmd\n\
         direct \\emph : [inline-text] inline-cmd\n\
         end = struct\n\
         let default-config = default-config\n\
         let document x c bt = bt\n\
         let font-latin-roman = (`f`, 1., 0.)\n\
         end",
    );
}
