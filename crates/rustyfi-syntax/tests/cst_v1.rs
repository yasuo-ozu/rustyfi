//! `cst_v1` round-trip and negative tests — the S5 slice's own test plan
//! items 2 and 3 (`docs/plans/rustyfi-0-1-0-support.md` cst_v1 design spec
//! §7): parse+unparse a hand-written 0.1-syntax snippet and assert every
//! `FileV1`/`Bind`/`Expr` node round-trips losslessly (`Parse` ∘ `Unparse`
//! = id at the token level), plus assert that SATySFi-0.1-invalid input is
//! rejected. Lowering/e2e tests (spec items 4-6) are out of scope for this
//! slice — `rustyfi-syntax` only produces `FileV1`/`parse_file_v1`.

use rustyfi_syntax::cst_v1::{self, parse_file_v1};
use rustyfi_syntax::lexer::lex_with_version;
use rustyfi_syntax::token::{Atom, Token};
use rustyfi_syntax::version::RustyfiVersion;
use syan::parse::Unparse;

fn assert_roundtrip_v1(src: &str) {
    let file = parse_file_v1(src).unwrap_or_else(|e| panic!("v1 parse failed on {src:?}: {e}"));
    let mut out = Vec::<Atom>::new();
    file.unparse(&mut (&mut out)).unwrap();
    let orig: Vec<Token> = lex_with_version(src, RustyfiVersion::V0_1)
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

/// Sub-slice 2b: expression-level `let rec … and … in` — the full
/// `and`-chain (Slice 1's single-clause restriction is retired).
#[test]
fn document_let_rec_and_in() {
    assert_roundtrip_v1(
        "let rec even n = if n <= 0 then true else odd n\n\
         and odd n = if n <= 0 then false else even n in even 4",
    );

    let file = parse_file_v1(
        "let rec even n = odd n and odd n = even n in even 4",
    )
    .unwrap();
    let cst_v1::FileV1::Document { body, .. } = file else {
        panic!("expected a document file");
    };
    let cst_v1::ast::Expr::LetRecIn { first, ands, .. } = body else {
        panic!("expected Expr::LetRecIn");
    };
    assert_eq!(first.name.name, "even");
    assert_eq!(ands.len(), 1);
    assert_eq!(ands[0].clause.name.name, "odd");
}

/// Sub-slice 2b: expression-level `let mutable x <- init in body`.
#[test]
fn document_let_mutable_in() {
    assert_roundtrip_v1("let mutable c <- 0 in c <- !c + 1");

    let file = parse_file_v1("let mutable c <- 0 in c").unwrap();
    let cst_v1::FileV1::Document { body, .. } = file else {
        panic!("expected a document file");
    };
    let cst_v1::ast::Expr::LetMutableIn { name, .. } = body else {
        panic!("expected Expr::LetMutableIn");
    };
    assert_eq!(name.name, "c");
}

/// Sub-slice 2b exclusion: 0.0.6 has no `Expr::LetInlineIn`/`LetBlockIn` at
/// all (`LETHORZ`/`LETVERT` are top-level-only), so there is nothing to
/// transcribe an expression-level `let inline`/`let block` to — a parse
/// error, documented.
#[test]
fn let_inline_or_block_in_is_a_parse_error() {
    assert!(parse_file_v1(r"let inline ctx \c = 0 in 1").is_err());
    assert!(parse_file_v1("let block ctx +c = 0 in 1").is_err());
}

/// math-split spec §4.1/§6.3 test 8: `val math` now parses via
/// `Bind::ValueMath`. `ctx` is MANDATORY — unlike `val inline`/`val block`,
/// there is no lightweight ctx-less form (contrast `bind_inline`'s two
/// productions) — so `val math \m = e` (no ctx variable before the `\cmd`)
/// stays a parse error, just for a different reason than before this spec
/// (a missing `ctx: VarTok`, not "no arm exists").
#[test]
fn val_math_without_ctx_is_a_parse_error() {
    assert!(parse_file_v1(r"module M = struct val math \m = e end").is_err());
}

/// math-split spec §4.1/§6.3 test 8: `val math ctx \f m = e` (no `with sub
/// sup`) parses to `Bind::ValueMath` with `scripts: None`.
#[test]
fn val_math_with_ctx_parses() {
    let src = r"module M = struct val math ctx \f m = e end";
    assert_roundtrip_v1(src);
    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 1);
    let cst_v1::Bind::ValueMath { ctx, cmd, params, scripts, .. } = &binds[0] else {
        panic!("expected Bind::ValueMath, got {:?}", binds[0]);
    };
    assert_eq!(ctx.name, "ctx");
    assert!(matches!(cmd, rustyfi_syntax::leaf::AnyHorzCmdTok::Plain(t) if t.name == r"\f"));
    assert_eq!(params.len(), 1);
    assert!(scripts.is_none());
}

/// math-split spec §4.1/§6.3 test 8: `val math ctx \f with sub sup = e`
/// parses to `Bind::ValueMath` with `scripts: Some(..)`.
#[test]
fn val_math_with_scripts_parses() {
    let src = r"module M = struct val math ctx \f with sub sup = e end";
    assert_roundtrip_v1(src);
    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 1);
    let cst_v1::Bind::ValueMath { ctx, params, scripts, .. } = &binds[0] else {
        panic!("expected Bind::ValueMath, got {:?}", binds[0]);
    };
    assert_eq!(ctx.name, "ctx");
    assert!(params.is_empty(), "no additional params before `with`");
    let scripts = scripts.as_ref().expect("Some(ScriptsParamV1)");
    assert_eq!(scripts.sub.name, "sub");
    assert_eq!(scripts.sup.name, "sup");
}

/// math-split spec §6.3 test 8: under V0_0_6, `math` stays an ordinary
/// identifier (the keyword gate is V0_1-only) — `let math = 3` still
/// lexes `math` as a plain `Var`, never `Token::Math`.
#[test]
fn math_keyword_is_gated_to_v0_1() {
    let toks = lex_with_version("val math ctx \\f m = e", RustyfiVersion::V0_1).unwrap();
    assert!(toks.iter().any(|a| matches!(a.slot, Token::Math)));
    let toks = lex_with_version("let math = 3", RustyfiVersion::V0_0_6).unwrap();
    assert!(toks.iter().all(|a| !matches!(a.slot, Token::Math)));
    assert!(toks.iter().any(|a| matches!(&a.slot, Token::Var(s) if s == "math")));
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

/// Language-completeness sweep gaps 2+3: `Expr::Fun.params` widened from
/// `Vec<VarTok>` to `Vec<PatBot>` — a wildcard parameter (`fun _ -> …`) and
/// a tuple-destructuring parameter (`fun (a, b) -> …`, the `list.satyg`
/// `mapi-adjacent`-shaped case, mixed with a plain-variable parameter in
/// the same `fun`) both now parse and round-trip, matching upstream
/// `parser_v1.mly:849-863`'s `argpats = list(patbot)`.
#[test]
fn document_fun_wildcard_and_tuple_destructure_params() {
    assert_roundtrip_v1("fun _ -> 1");
    assert_roundtrip_v1("fun _ x -> x");
    assert_roundtrip_v1("fun (a, b) -> a");
    assert_roundtrip_v1("fun (a, b) x -> a");
    let file = parse_file_v1("fun (i, acc) x -> acc").unwrap();
    let cst_v1::FileV1::Document { body, .. } = file else {
        panic!("expected a document file");
    };
    let cst_v1::ast::Expr::Fun { params, .. } = body else {
        panic!("expected a fun expression");
    };
    assert_eq!(params.len(), 2, "the tuple param plus the plain-variable param");
    assert!(
        params[0].opts.is_none()
            && matches!(
                &params[0].body,
                cst_v1::ast::ParamBody::Pat(cst_v1::ast::PatBot::Paren { .. })
            ),
        "first param should be the (i, acc) tuple pattern, got {:?}",
        params[0]
    );
    assert!(
        params[1].opts.is_none()
            && matches!(
                &params[1].body,
                cst_v1::ast::ParamBody::Pat(cst_v1::ast::PatBot::Var(_))
            ),
        "second param should be the plain variable x, got {:?}",
        params[1]
    );
}

// ---- SATySFi 0.1 labeled optional arguments (optional-arg-rows incr. 1) ----

#[test]
fn document_labeled_optional_arguments() {
    // Application bundle `?(l = e, …) arg` paired with its positional arg.
    assert_roundtrip_v1("f ?(a = 1, b = x + 1) y");
    // Multi-label subset / reordering at the call site.
    assert_roundtrip_v1("add ?(scale = 3, bias = 1) 2");
    // A bundled tuple-pattern PARAMETER unit.
    assert_roundtrip_v1("fun ?(a = x) (p, q) -> p");
    // A `let` param bundle plus a plain param.
    assert_roundtrip_v1("let add ?(bias = b, scale = s) x = x in add 1");
    // A bundle heading a bare-constructor argument (`BundledCtor`).
    assert_roundtrip_v1("f ?(a = 1) None");
}

/// optional-arg-rows increment 3a: a `?(l = x, …)` labeled-optional bundle
/// on an inline/block command's OWN parameter list — grammar-wise this
/// already parsed at increment 1 (`cst_v1::Param.opts`), this pins it stays
/// that way once `lower_command_params` actually consumes it (rather than
/// erroring) rather than a grammar regression.
#[test]
fn command_param_bundle_round_trips() {
    assert_roundtrip_v1(
        "module M = struct\n\
         val inline ctx \\c ?(a = x, b = y) t = t\n\
         end",
    );
    assert_roundtrip_v1(
        "module M = struct\n\
         val block ctx +sec ?(label = l, outline-title = o) title inner = inner\n\
         end",
    );
}

#[test]
fn old_optional_sigils_no_longer_lex_under_v01() {
    // SATySFi 0.1 dropped the fused `?:`/`?*` sigils: under V0_1 a bare `?`
    // is the only `?`-headed token, so `?:`/`?*` lex as `?` + `:`/`*` — the
    // parse then fails (no grammar consumes them).
    assert!(parse_file_v1("f ?: 1").is_err());
    assert!(parse_file_v1("f ?*").is_err());
    // Confirm the lexer itself: no `Optional`/`Omission` token under V0_1.
    let toks: Vec<Token> = lex_with_version("?: ?*", RustyfiVersion::V0_1)
        .unwrap()
        .into_iter()
        .map(|a| a.slot)
        .collect();
    assert!(
        !toks.iter().any(|t| matches!(t, Token::Optional | Token::Omission)),
        "V0_1 must not lex `?:`/`?*` as fused optional tokens, got {toks:?}"
    );
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
    let rustyfi_syntax::leaf::AnyHorzCmdTok::Plain(cmd) = name else {
        panic!("expected a sigil-only command name");
    };
    assert_eq!(cmd.name, "\\math");
}

/// Language-completeness sweep gap 4: `(command \Mod.cmd)` — the SAME
/// first-class command-reference syntax as [`document_command_reference`]
/// above, but module-qualified (upstream's `parser_v1.mly:906-908`
/// `backslash_cmd` accepts `LONG_HORZCMD`, not just bare `HORZCMD`). The
/// grammar (`Atomic::Command { name: AnyHorzCmdTok, .. }`) already had an
/// `AnyHorzCmdTok::Mod` arm and needed no CST change — the gap was purely
/// the lexer's program-mode `\` handling never scanning a dotted path (see
/// `lexer.rs` tests' `program_mode_qualified_command`).
#[test]
fn document_qualified_command_reference() {
    assert_roundtrip_v1(r"(command \Mod.cmd)");

    let file = parse_file_v1(r"(command \Mod.cmd)").unwrap();
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
    let rustyfi_syntax::leaf::AnyHorzCmdTok::Mod(cmd) = name else {
        panic!("expected a module-qualified command name");
    };
    assert_eq!(cmd.mods, vec!["Mod".to_string()]);
    assert_eq!(cmd.name, "\\cmd");
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

/// Sub-slice 2b: `val rec … and …` — lossless round-trip; parsed shape has
/// `ValueRec` with 1 `and`.
#[test]
fn library_val_rec_and_chain() {
    let src = "module M = struct\n\
               val rec even n = odd n\n\
               and odd n = even n\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 1);
    let cst_v1::Bind::ValueRec { first, ands, .. } = &binds[0] else {
        panic!("expected a ValueRec bind, got {:?}", binds[0]);
    };
    assert_eq!(first.name.name, "even");
    assert_eq!(ands.len(), 1);
    assert_eq!(ands[0].clause.name.name, "odd");
}

/// Sub-slice 2b: `val mutable x <- e`.
#[test]
fn library_val_mutable() {
    let src = "module M = struct\n\
               val mutable c <- 0\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 1);
    assert!(matches!(&binds[0], cst_v1::Bind::ValueMutable { name, .. } if name.name == "c"));
}

/// Sub-slice 2b: `type` binds — the variant/synonym split, postfix-tyvars
/// parse (`u 'a`), products (`int * int`), and prefix application
/// (`option t`, 1 argument — the arity guard only fires at LOWERING, so a
/// 2-argument application is pinned as a lowering-error test instead, not
/// here; see `v1/lower.rs`'s `type_app_arity_2_is_a_lower_error`).
#[test]
fn library_type_binds() {
    let src = "module M = struct\n\
               type t = | A of int * int | B\n\
               and u 'a = t -> option t\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 1);
    let cst_v1::Bind::Type { first, ands, .. } = &binds[0] else {
        panic!("expected a Type bind, got {:?}", binds[0]);
    };
    assert_eq!(first.name.name, "t");
    assert_eq!(ands.len(), 1);
    assert_eq!(ands[0].bind.name.name, "u");
    assert_eq!(ands[0].bind.tyvars.len(), 1, "`u 'a` — one postfix tyvar");
    let cst_v1::TypeBodyV1::Variant { first: a_def, rest, .. } = &first.body else {
        panic!("expected t's body to be a variant");
    };
    assert_eq!(a_def.ctor.name, "A");
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].def.ctor.name, "B");
    let cst_v1::TypeBodyV1::Synonym(_) = &ands[0].bind.body else {
        panic!("expected u's body to be a synonym");
    };
}

/// Sub-slice 2b: `bound_identifier` retirement — `val (+++) a b = a`.
#[test]
fn library_op_named_value() {
    let src = "module M = struct\n\
               val (+++) a b = a\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 1);
    assert!(matches!(&binds[0], cst_v1::Bind::Value { name, .. } if name.name == "+++"));
}

/// G2: closed type-level records (`(| l : ty |)`) now parse (`TypeAtom::
/// Record`) — round-trips, and the field list/names come through as
/// expected. The `| ?'r` row-var tail form NOW ALSO parses (optional-arg-rows
/// increment 2 — see `type_record_rowvar_tail_now_parses_and_round_trips`
/// below).
#[test]
fn type_record_round_trips() {
    let src = "module M = struct\n\
               type t = (| x : int |)\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    let cst_v1::Bind::Type { first, .. } = &binds[0] else {
        panic!("expected a Type bind, got {:?}", binds[0]);
    };
    let cst_v1::TypeBodyV1::Synonym(ty) = &first.body else {
        panic!("expected t's body to be a synonym, got {:?}", first.body);
    };
    let cst_v1::ast::TypeExpr::Atom(cst_v1::ast::TypeProd { first: app, .. }) = ty else {
        panic!("expected a bare TypeProd, got {ty:?}");
    };
    let cst_v1::ast::TypeApp::Atom(cst_v1::ast::TypeAtom::Record { inner, .. }) = app else {
        panic!("expected TypeAtom::Record, got {app:?}");
    };
    let names: Vec<String> = inner.fields.iter().map(|f| f.name.name.clone()).collect();
    assert_eq!(names, vec!["x".to_string()]);
}

/// Multiple fields (with a trailing comma) and a nested field type.
#[test]
fn type_record_multi_field_and_nested_round_trips() {
    assert_roundtrip_v1(
        "module M = struct\n\
         type t = (| title : inline-text, count : int, |)\n\
         end",
    );
    assert_roundtrip_v1(
        "module M = struct\n\
         type t = (| f : int -> bool, p : int * int |)\n\
         end",
    );
}

/// A closed record type used as a `Fun` domain, and in a sig `val` decl —
/// the scout's G2 acceptance shape.
#[test]
fn type_record_as_fun_domain_and_in_sig_round_trips() {
    assert_roundtrip_v1(
        "module M = struct\n\
         signature S = sig\n\
         val document : (| title : inline-text |) -> int\n\
         end\n\
         end",
    );
}

/// optional-arg-rows increment 2: the open form's `| ?'r` row-var tail now
/// parses (round-trips) and its `row_tail` comes through with the expected
/// row-variable name.
#[test]
fn type_record_rowvar_tail_now_parses_and_round_trips() {
    let src = "module M = struct\n\
               type t = (| x : int | ?'r |)\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    let cst_v1::Bind::Type { first, .. } = &binds[0] else {
        panic!("expected a Type bind, got {:?}", binds[0]);
    };
    let cst_v1::TypeBodyV1::Synonym(ty) = &first.body else {
        panic!("expected t's body to be a synonym, got {:?}", first.body);
    };
    let cst_v1::ast::TypeExpr::Atom(cst_v1::ast::TypeProd { first: app, .. }) = ty else {
        panic!("expected a bare TypeProd, got {ty:?}");
    };
    let cst_v1::ast::TypeApp::Atom(cst_v1::ast::TypeAtom::Record { inner, .. }) = app else {
        panic!("expected TypeAtom::Record, got {app:?}");
    };
    let names: Vec<String> = inner.fields.iter().map(|f| f.name.name.clone()).collect();
    assert_eq!(names, vec!["x".to_string()]);
    let tail = inner.row_tail.as_ref().expect("expected a row_tail");
    assert_eq!(tail.var.name, "r");
}

/// `=` is the record-EXPRESSION field separator, not the record-TYPE one —
/// `(| x = int |)` must still fail to parse as a type.
#[test]
fn type_record_eq_separator_is_a_parse_error() {
    assert!(parse_file_v1(
        "module M = struct\n\
         type t = (| x = int |)\n\
         end"
    )
    .is_err());
}

/// `;` is the 0.0.6 record-type field separator, not 0.1's `,` — still a
/// parse error under V0_1.
#[test]
fn type_record_semicolon_separator_is_a_parse_error() {
    assert!(parse_file_v1(
        "module M = struct\n\
         type t = (| x : int; y : int |)\n\
         end"
    )
    .is_err());
}

/// The `stdja-mini.satyh` transliteration from the cst_v1 design spec's §3
/// (Slice-1 `Bind` ground truth), covering all three `Bind` arms plus a
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
    assert!(matches!(binds[0], cst_v1::Bind::Value { .. }));
    assert!(matches!(binds[1], cst_v1::Bind::ValueBlock { .. }));
    assert!(matches!(binds[2], cst_v1::Bind::ValueInline { .. }));
    assert!(matches!(binds[3], cst_v1::Bind::ValueInline { .. }));
}

/// Sub-slice 2a: a nested `module N = struct … end` bind round-trips
/// losslessly and the parsed shape is exactly the nested `Bind::Module`.
#[test]
fn library_nested_module_bind() {
    let src = "module M = struct\n\
               val x = 1\n\
               module N = struct\n\
               val y = 2\n\
               end\n\
               val z = 3\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { name, binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(name.name, "M");
    assert_eq!(binds.len(), 3);
    assert!(matches!(binds[0], cst_v1::Bind::Value { .. }));
    let cst_v1::Bind::Module {
        name: inner_name,
        sig_annot: None,
        body,
        ..
    } = &binds[1]
    else {
        panic!("expected binds[1] to be a nested module bind");
    };
    let cst_v1::ast::ModExpr::Struct { binds: inner_binds, .. } = &*body.0 else {
        panic!("expected a struct-literal body");
    };
    assert_eq!(inner_name.name, "N");
    assert_eq!(inner_binds.len(), 1);
    assert!(matches!(&*inner_binds[0].0, cst_v1::Bind::Value { .. }));
    assert!(matches!(binds[2], cst_v1::Bind::Value { .. }));
}

/// Sub-slice 2c retires the Slice-1/2a struct-literal-only restriction:
/// `module M = N` (a bare module alias) now parses fine, as
/// `ModExpr::Var` — it is Sub-slice 2d's `LowerError` at LOWERING time, not
/// a parse error (see `v1/lower.rs`'s `module_alias_is_a_lower_error`).
/// What remains a parse error is `FileV1`'s own top-level shape: neither of
/// these has any `FileV1` production at all.
#[test]
fn module_alias_at_top_level_is_still_a_parse_error() {
    // A bare `module M = N` (no `struct … end`) is not `main_lib` — the
    // top-level library production is always `MODULE UPPER
    // option(sig_annot) EXACT_EQ STRUCT bind* END`, never a bare modexpr.
    assert!(parse_file_v1("module M = N").is_err());
    assert!(
        parse_file_v1(
            "module M = struct\n\
             val x = 1\n\
             end\n\
             module P = M"
        )
        .is_err(),
        "a second top-level form after the library's closing `end` has no \
         FileV1 shape at all"
    );
}

/// Regression pin: a document's module-qualified variable reference still
/// parses as `Atomic::VarWithMod` (unaffected by the new `Bind::Module`
/// arm — `VarWithMod` was already Slice-1 grammar).
#[test]
fn document_qualified_var_still_parses_as_var_with_mod() {
    assert_roundtrip_v1("M.x");
    let file = parse_file_v1("M.x").unwrap();
    let cst_v1::FileV1::Document { body, .. } = file else {
        panic!("expected a document file");
    };
    let cst_v1::ast::Expr::Ops(chain) = body else {
        panic!("expected an operator-chain expression");
    };
    assert!(matches!(
        chain.head.head,
        cst_v1::ast::Atomic::VarWithMod(_)
    ));
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
        "/../../lib-rustyfi/dist/packages/stdja-mini.satyh"
    ))
    .expect("lib-rustyfi/dist/packages/stdja-mini.satyh must exist");
    assert!(parse_file_v1(&v006_src).is_err());
}

// ---- Sub-slice 2c: module/signature grammar ----------------------------

/// §5.1 item 1: `main_lib`'s `:>` signature annotation.
#[test]
fn library_sig_annot() {
    let src = "module M :> sig val x : int end = struct\n\
               val x = 1\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { sig_annot, .. } = file else {
        panic!("expected a library file");
    };
    let sig_annot = sig_annot.expect("sig_annot must be Some");
    let cst_v1::ast::SigExpr::Bot(cst_v1::ast::SigBotV1::Sig { decls, .. }) = &*sig_annot.sig_.0
    else {
        panic!("expected SigExpr::Bot(SigBotV1::Sig), got {:?}", sig_annot.sig_.0);
    };
    assert_eq!(decls.len(), 1);
    assert!(
        matches!(&*decls[0].0, cst_v1::ast::Decl::Val { name, .. } if name.name == "x"),
        "{:?}",
        decls[0].0
    );
}

/// §5.1 item 2: the bind-level `option(sig_annot)`.
#[test]
fn library_nested_module_sig_annot() {
    let src = "module M = struct\n\
               module N :> sig val y : int end = struct\n\
               val y = 2\n\
               end\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 1);
    let cst_v1::Bind::Module { name, sig_annot, .. } = &binds[0] else {
        panic!("expected a nested module bind");
    };
    assert_eq!(name.name, "N");
    assert!(sig_annot.is_some());
}

/// §5.1 item 3: module aliases and (possibly long) module paths.
#[test]
fn library_module_alias_and_paths() {
    let src = "module M = struct\n\
               module N = struct val x = 1 end\n\
               module P = N\n\
               module Q = A.B.C\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 3);
    let cst_v1::Bind::Module { body: p_body, .. } = &binds[1] else {
        panic!("expected binds[1] to be P's module bind");
    };
    assert!(
        matches!(
            &*p_body.0,
            cst_v1::ast::ModExpr::Var(cst_v1::ast::ModChainV1::Single(_))
        ),
        "{:?}",
        p_body.0
    );
    let cst_v1::Bind::Module { body: q_body, .. } = &binds[2] else {
        panic!("expected binds[2] to be Q's module bind");
    };
    let cst_v1::ast::ModExpr::Var(cst_v1::ast::ModChainV1::Long(long)) = &*q_body.0 else {
        panic!("expected Q's body to be Var(Long), got {:?}", q_body.0);
    };
    assert_eq!(long.mods, vec!["A".to_string(), "B".to_string()]);
    assert_eq!(long.name, "C");
}

/// §5.1 item 4: a functor literal bind (also exercises `VarWithMod` `X.x`
/// inside the functor body).
#[test]
fn library_functor_bind() {
    let src = "module M = struct\n\
               module F = fun (X : sig val x : int end) -> struct val y = X.x end\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    let cst_v1::Bind::Module { body, .. } = &binds[0] else {
        panic!("expected F's module bind");
    };
    assert!(
        matches!(&*body.0, cst_v1::ast::ModExpr::Functor { .. }),
        "{:?}",
        body.0
    );
}

/// §5.1 item 5: functor application, plus the App-vs-Var backtrack pin.
#[test]
fn library_functor_application() {
    let src = "module M = struct\n\
               module P = F X\n\
               module Q = F.G X.Y\n\
               module R = N\n\
               val z = 1\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 4);
    let cst_v1::Bind::Module { body: p_body, .. } = &binds[0] else {
        panic!("expected P's module bind");
    };
    assert!(
        matches!(
            &*p_body.0,
            cst_v1::ast::ModExpr::App {
                func: cst_v1::ast::ModChainV1::Single(_),
                arg: cst_v1::ast::ModChainV1::Single(_),
            }
        ),
        "{:?}",
        p_body.0
    );
    let cst_v1::Bind::Module { body: q_body, .. } = &binds[1] else {
        panic!("expected Q's module bind");
    };
    assert!(
        matches!(
            &*q_body.0,
            cst_v1::ast::ModExpr::App {
                func: cst_v1::ast::ModChainV1::Long(_),
                arg: cst_v1::ast::ModChainV1::Long(_),
            }
        ),
        "{:?}",
        q_body.0
    );
    let cst_v1::Bind::Module { body: r_body, .. } = &binds[2] else {
        panic!("expected R's module bind");
    };
    assert!(
        matches!(&*r_body.0, cst_v1::ast::ModExpr::Var(cst_v1::ast::ModChainV1::Single(_))),
        "the trailing `val z = 1` must not be swallowed as a second chain: {:?}",
        r_body.0
    );
    assert!(matches!(binds[3], cst_v1::Bind::Value { .. }));
}

/// §5.1 item 6: a `signature` bind plus full `decl` coverage inside its
/// `sig … end` body — all eight `Decl` forms, `sig end` (empty).
#[test]
fn library_signature_bind_and_full_decl_coverage() {
    let src = "module M = struct\n\
               signature S = sig\n\
               type t :: o\n\
               type m :: o -> o\n\
               type u 'a = t\n\
               val x : int\n\
               val map 'a 'b : ('a -> 'b) -> t -> t\n\
               val \\emph : int\n\
               val +p : int\n\
               module N : sig val y : int end\n\
               signature T = sig end\n\
               include T\n\
               end\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 1);
    let cst_v1::Bind::Signature { name, sig_, .. } = &binds[0] else {
        panic!("expected a Bind::Signature, got {:?}", binds[0]);
    };
    assert_eq!(name.name, "S");
    let cst_v1::ast::SigExpr::Bot(cst_v1::ast::SigBotV1::Sig { decls, .. }) = &*sig_.0 else {
        panic!("expected SigExpr::Bot(SigBotV1::Sig)");
    };
    assert_eq!(decls.len(), 10, "{decls:?}");

    let cst_v1::ast::Decl::TypeOpaque { name: t_name, kind: t_kind, .. } = &*decls[0].0 else {
        panic!("expected decls[0] = t to be TypeOpaque, got {:?}", decls[0].0);
    };
    assert_eq!(t_name.name, "t");
    assert!(t_kind.rest.is_empty());

    let cst_v1::ast::Decl::TypeOpaque { name: m_name, kind: m_kind, .. } = &*decls[1].0 else {
        panic!("expected decls[1] = m to be TypeOpaque, got {:?}", decls[1].0);
    };
    assert_eq!(m_name.name, "m");
    assert_eq!(m_kind.rest.len(), 1);

    assert!(matches!(&*decls[2].0, cst_v1::ast::Decl::Type { .. }), "{:?}", decls[2].0);

    assert!(
        matches!(&*decls[3].0, cst_v1::ast::Decl::Val { name, .. } if name.name == "x"),
        "{:?}",
        decls[3].0
    );

    let cst_v1::ast::Decl::Val { name: map_name, quant, .. } = &*decls[4].0 else {
        panic!("expected decls[4] = map to be Decl::Val, got {:?}", decls[4].0);
    };
    assert_eq!(map_name.name, "map");
    assert_eq!(quant.len(), 2);

    assert!(matches!(&*decls[5].0, cst_v1::ast::Decl::ValHorzCmd { .. }), "{:?}", decls[5].0);
    assert!(matches!(&*decls[6].0, cst_v1::ast::Decl::ValVertCmd { .. }), "{:?}", decls[6].0);

    let cst_v1::ast::Decl::Module { name: n_name, .. } = &*decls[7].0 else {
        panic!("expected decls[7] = N to be Decl::Module, got {:?}", decls[7].0);
    };
    assert_eq!(n_name.name, "N");

    let cst_v1::ast::Decl::Signature { name: t2_name, sig_: t2_sig, .. } = &*decls[8].0 else {
        panic!("expected decls[8] = T to be Decl::Signature, got {:?}", decls[8].0);
    };
    assert_eq!(t2_name.name, "T");
    let cst_v1::ast::SigExpr::Bot(cst_v1::ast::SigBotV1::Sig { decls: t2_decls, .. }) = &**t2_sig
    else {
        panic!("expected T's body to be sig ... end");
    };
    assert!(t2_decls.is_empty(), "`sig end` — the empty decl list");

    assert!(matches!(&*decls[9].0, cst_v1::ast::Decl::Include { .. }), "{:?}", decls[9].0);
}

/// §5.1 item 7: `with [path] type … and …` refinement.
#[test]
fn library_with_type() {
    let src = "module M :> S with type t = int and u = bool = struct\n\
               type t = int\n\
               and u = bool\n\
               val x = 1\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { sig_annot, .. } = file else {
        panic!("expected a library file");
    };
    let sig_annot = sig_annot.expect("sig_annot must be Some");
    let cst_v1::ast::SigExpr::WithType { base, path, binds, .. } = &*sig_annot.sig_.0 else {
        panic!("expected SigExpr::WithType, got {:?}", sig_annot.sig_.0);
    };
    assert!(matches!(base, cst_v1::ast::SigBotV1::Var(v) if v.name == "S"));
    assert!(path.is_none());
    assert_eq!(binds.0.ands.len() + 1, 2, "a chain of 2");

    let src2 = "module M2 :> S with A.B type t = int = struct\n\
                val x = 1\n\
                end";
    assert_roundtrip_v1(src2);
    let file2 = parse_file_v1(src2).unwrap();
    let cst_v1::FileV1::Library { sig_annot: sig_annot2, .. } = file2 else {
        panic!("expected a library file");
    };
    let sig_annot2 = sig_annot2.expect("sig_annot must be Some");
    let cst_v1::ast::SigExpr::WithType { path: path2, .. } = &*sig_annot2.sig_.0 else {
        panic!("expected SigExpr::WithType, got {:?}", sig_annot2.sig_.0);
    };
    assert!(matches!(path2, Some(cst_v1::ast::ModChainV1::Long(_))));
}

/// §5.1 item 8: `include` binds.
#[test]
fn library_include_bind() {
    let src = "module M = struct\n\
               include N\n\
               include A.B.C\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    assert_eq!(binds.len(), 2);
    let cst_v1::Bind::Include { body: b0, .. } = &binds[0] else {
        panic!("expected binds[0] to be a Bind::Include, got {:?}", binds[0]);
    };
    assert!(matches!(&*b0.0, cst_v1::ast::ModExpr::Var(cst_v1::ast::ModChainV1::Single(_))));
    let cst_v1::Bind::Include { body: b1, .. } = &binds[1] else {
        panic!("expected binds[1] to be a Bind::Include, got {:?}", binds[1]);
    };
    assert!(matches!(&*b1.0, cst_v1::ast::ModExpr::Var(cst_v1::ast::ModChainV1::Long(_))));
}

/// §5.1 item 9: a `:>` signature PATH annotation.
#[test]
fn library_sig_path_annot() {
    let src = "module M :> A.B.S = struct\n\
               val x = 1\n\
               end";
    assert_roundtrip_v1(src);

    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { sig_annot, .. } = file else {
        panic!("expected a library file");
    };
    let sig_annot = sig_annot.expect("sig_annot must be Some");
    assert!(matches!(&*sig_annot.sig_.0, cst_v1::ast::SigExpr::Bot(cst_v1::ast::SigBotV1::Path(_))));
}

// ---- Sub-slice 2c: negative pins -----------------------------------------

/// §5.1 item 10 (the left-recursion correction's behavioral fingerprint):
/// upstream rejects chained `with`, and so does the bot+suffix encoding.
#[test]
fn with_cannot_chain() {
    assert!(parse_file_v1(
        "module M :> S with type t = int with type u = bool = struct\n\
         val x = 1\n\
         end"
    )
    .is_err());
}

/// §5.1 item 11: 0.1's annotation sigil is `:>`, never 0.0.6's `: sig …
/// end` shape.
#[test]
fn bare_colon_module_annotation_is_a_parse_error() {
    assert!(parse_file_v1("module M : sig end = struct val x = 1 end").is_err());
}

/// §5.1 item 12 (phase 5 — staged decls have no `Decl` arm yet).
#[test]
fn staged_val_decl_is_a_parse_error() {
    assert!(parse_file_v1(
        "module M = struct\nsignature S = sig val ~x : int end\nend"
    )
    .is_err());
}

/// §5.1 item 13 (phase 5 — macro decls lex as `HorzMacro`/`VertMacro`, not
/// `HorzCmdTok`/`VertCmdTok`, so no `Decl` arm accepts them).
#[test]
fn macro_decl_is_a_parse_error() {
    assert!(parse_file_v1(
        "module M = struct\nsignature S = sig val \\m@ : int end\nend"
    )
    .is_err());
}

/// §5.1 item 14 (phase 4 — no `ROWVAR` token, so `?'r` cannot appear where
/// a `Decl::Val` expects a `quant`/`colon`).
#[test]
fn row_quantifier_is_a_parse_error() {
    assert!(parse_file_v1(
        "module M = struct\nsignature S = sig val f (?'r :: (| a |)) : int end\nend"
    )
    .is_err());
}

// ---- Ld3a: HeaderV1 (the `use`-header union grammar) ------------------------

/// Every `use`-header form round-trips, pulled from the real `saphe-split`
/// demo headers (`demo/demo.saty`, `demo/local.satyh`), plus the Legacy
/// `@`-header the union grammar also accepts.
#[test]
fn header_v1_use_forms_round_trip() {
    // `use package [open] mod_chain`
    assert_roundtrip_v1("use package Tabular\n3");
    assert_roundtrip_v1("use package open Stdlib\n3");
    assert_roundtrip_v1("use package open Stdlib.Logo\n3");
    // `use [open] mod_chain of `relpath``
    assert_roundtrip_v1("use open Local of `./local`\n3");
    assert_roundtrip_v1("use Local of `./local`\n3");
    // bare `use [open] mod_chain`
    assert_roundtrip_v1("use Local\n3");
    assert_roundtrip_v1("use open Local\n3");
    // Legacy `@`-headers, still accepted by the one V0_1 grammar.
    assert_roundtrip_v1("@require: pervasives\n3");
    assert_roundtrip_v1("@import: helper\n3");
}

/// A library (`module … = struct … end`) carrying a `use` header round-trips,
/// and multiple header families coexist in one file (the union grammar).
#[test]
fn header_v1_on_library_and_mixed_families() {
    assert_roundtrip_v1("use package Stdlib.List\nmodule Local = struct\nval x = 1\nend");
    assert_roundtrip_v1(
        "use package open Stdlib\n\
         use package Tabular\n\
         use open Local of `./local`\n\
         @require: pervasives\n3",
    );
}

/// The `HeaderV1::display_name` helper the loader uses for diagnostics.
#[test]
fn header_v1_display_names() {
    use cst_v1::{FileV1, HeaderV1};
    let file = parse_file_v1(
        "use package open Stdlib.Logo\n\
         use open Local of `./local`\n\
         use Sibling\n\
         @require: pervasives\n3",
    )
    .unwrap();
    let FileV1::Document { headers, .. } = file else {
        panic!("expected a document");
    };
    let names: Vec<String> = headers.iter().map(HeaderV1::display_name).collect();
    assert_eq!(
        names,
        vec![
            "use package Stdlib.Logo".to_string(),
            "use Local of `./local`".to_string(),
            "use Sibling".to_string(),
            "@require: pervasives".to_string(),
        ]
    );
}

// ============================================================================
// Sub-slice 2d-2: `inline […]`/`block […]` command types, `M.t` LONG_LOWER
// qualified type paths (§4-A of the opaque-types spec, U18).
// ============================================================================

/// `inline [τ, …]`/`block […]` round-trip in sig `val` position, including a
/// two-element list and a bare `[]`.
#[test]
fn command_type_round_trips() {
    assert_roundtrip_v1(
        "module M = struct\n\
         signature S = sig\n\
         val \\show : inline [int, inline-text]\n\
         val +put : block []\n\
         end\n\
         end",
    );
}

/// `inline [t] -> t` — a command type used as a function DOMAIN (so `->`
/// still parses correctly around the bracketed list).
#[test]
fn command_type_as_fun_domain_round_trips() {
    assert_roundtrip_v1(
        "module M = struct\n\
         signature S = sig\n\
         val f : inline [int] -> int\n\
         end\n\
         end",
    );
}

/// `M.t`, `M.t int`, `A.B.t` — `LONG_LOWER` qualified type names, bare and
/// applied, including a two-level module path.
#[test]
fn long_lower_type_name_round_trips() {
    assert_roundtrip_v1(
        "module M = struct\n\
         signature S = sig\n\
         val f : N.t\n\
         val g : N.t int\n\
         val h : A.B.t\n\
         end\n\
         end",
    );
}

/// Shape assertions for the command-type/LONG_LOWER grammar: the parsed tree
/// actually contains the arms the round-trip tests above only check
/// byte-for-byte re-emission of.
#[test]
fn command_type_and_long_lower_shapes() {
    let src = "module M = struct\n\
               signature S = sig\n\
               val \\show : inline [int, inline-text]\n\
               val +put : block [t]\n\
               val f : N.t\n\
               val g : N.t int\n\
               end\n\
               end";
    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    let cst_v1::Bind::Signature { sig_, .. } = &binds[0] else {
        panic!("expected a Bind::Signature, got {:?}", binds[0]);
    };
    let cst_v1::ast::SigExpr::Bot(cst_v1::ast::SigBotV1::Sig { decls, .. }) = &*sig_.0 else {
        panic!("expected SigExpr::Bot(SigBotV1::Sig)");
    };
    assert_eq!(decls.len(), 4, "{decls:?}");

    let cst_v1::ast::Decl::ValHorzCmd { ty: show_ty, .. } = &*decls[0].0 else {
        panic!("expected decls[0] to be ValHorzCmd, got {:?}", decls[0].0);
    };
    let cst_v1::ast::TypeExpr::Atom(cst_v1::ast::TypeProd { first, .. }) = show_ty else {
        panic!("expected a bare TypeProd, got {show_ty:?}");
    };
    let cst_v1::ast::TypeApp::InlineCmdTy { args, .. } = first else {
        panic!("expected TypeApp::InlineCmdTy, got {first:?}");
    };
    assert_eq!(args.len(), 2, "{args:?}");

    let cst_v1::ast::Decl::ValVertCmd { ty: put_ty, .. } = &*decls[1].0 else {
        panic!("expected decls[1] to be ValVertCmd, got {:?}", decls[1].0);
    };
    let cst_v1::ast::TypeExpr::Atom(cst_v1::ast::TypeProd { first: put_first, .. }) = put_ty else {
        panic!("expected a bare TypeProd, got {put_ty:?}");
    };
    assert!(matches!(put_first, cst_v1::ast::TypeApp::BlockCmdTy { .. }), "{put_first:?}");

    let cst_v1::ast::Decl::Val { ty: f_ty, .. } = &*decls[2].0 else {
        panic!("expected decls[2] to be Decl::Val, got {:?}", decls[2].0);
    };
    let cst_v1::ast::TypeExpr::Atom(cst_v1::ast::TypeProd { first: f_first, .. }) = f_ty else {
        panic!("expected a bare TypeProd, got {f_ty:?}");
    };
    let cst_v1::ast::TypeApp::Atom(cst_v1::ast::TypeAtom::LongName(n_t)) = f_first else {
        panic!("expected TypeAtom::LongName, got {f_first:?}");
    };
    assert_eq!(n_t.mods, vec!["N".to_string()]);
    assert_eq!(n_t.name, "t");

    let cst_v1::ast::Decl::Val { ty: g_ty, .. } = &*decls[3].0 else {
        panic!("expected decls[3] to be Decl::Val, got {:?}", decls[3].0);
    };
    let cst_v1::ast::TypeExpr::Atom(cst_v1::ast::TypeProd { first: g_first, .. }) = g_ty else {
        panic!("expected a bare TypeProd, got {g_ty:?}");
    };
    let cst_v1::ast::TypeApp::AppliedLong { ctor, .. } = g_first else {
        panic!("expected TypeApp::AppliedLong, got {g_first:?}");
    };
    assert_eq!(ctor.mods, vec!["N".to_string()]);
    assert_eq!(ctor.name, "t");
}

/// Negatives: `math […]` now PARSES (math-package completion M1 — see
/// `math_command_type_head_round_trips`/`math_command_type_shape` below;
/// this test used to pin it as a parse error under increment 3b). A
/// `?`-suffixed slot (`int?`) still never parses (0.1 has no such
/// positional-optional marker at all, only the closed-map `?(…)` prefix),
/// and a `?(…)`-prefixed slot with NO mandatory type following the bundle
/// (`[?(l : int)]` alone, nothing after the bundle) still fails — the
/// bundle prefix is optional, but the slot's own `ty` is not
/// (`TypeCmdArgItemV1.opts: Option<..>`, `ty: TyErasedV1`, not
/// `Option<TyErasedV1>`). A well-formed `?(l:τ,…) τ_arg` DOES now parse —
/// see `command_type_opt_labels_round_trips` (optional-arg-rows increment
/// 3a, "roadmap phase 4" landed).
#[test]
fn command_type_negatives() {
    assert!(parse_file_v1(
        "module M = struct\n\
         signature S = sig\n\
         val \\show : math [int]\n\
         end\n\
         end"
    )
    .is_ok());
    assert!(parse_file_v1(
        "module M = struct\n\
         signature S = sig\n\
         val \\show : inline [int?]\n\
         end\n\
         end"
    )
    .is_err());
    assert!(parse_file_v1(
        "module M = struct\n\
         signature S = sig\n\
         val \\show : inline [?(l : int)]\n\
         end\n\
         end"
    )
    .is_err());
}

/// optional-arg-rows increment 3a: `inline [?(l:τ,…) τ_arg, …]` / `block
/// […]` — the `?(l : τ, …)` labeled-optional command-type row PREFIX,
/// round-tripping on a single- and a two-label bundle, mixed with a
/// trailing plain (unbundled) slot, plus the empty-list `opts: None` case
/// stays byte-identical (`command_type_round_trips`, above, untouched).
#[test]
fn command_type_opt_labels_round_trips() {
    assert_roundtrip_v1(
        "module M = struct\n\
         signature S = sig\n\
         val \\show : inline [?(a : int) inline-text, block-text]\n\
         val +put : block [?(a : int, b : bool) inline-text]\n\
         end\n\
         end",
    );
}

/// Shape assertion for the `?(l:τ,…)` command-type row: the parsed tree
/// actually carries `opts` with the right entries (surface order, NOT yet
/// sorted — sorting is a `typecheck.rs`-side concern, §7.3/§14 risk 3), and
/// a plain slot right after a bundled one still parses with `opts: None`.
#[test]
fn command_type_opt_labels_shape() {
    let src = "module M = struct\n\
               signature S = sig\n\
               val \\show : inline [?(b : bool, a : int) inline-text, block-text]\n\
               end\n\
               end";
    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    let cst_v1::Bind::Signature { sig_, .. } = &binds[0] else {
        panic!("expected a Bind::Signature, got {:?}", binds[0]);
    };
    let cst_v1::ast::SigExpr::Bot(cst_v1::ast::SigBotV1::Sig { decls, .. }) = &*sig_.0 else {
        panic!("expected SigExpr::Bot(SigBotV1::Sig)");
    };
    let cst_v1::ast::Decl::ValHorzCmd { ty, .. } = &*decls[0].0 else {
        panic!("expected decls[0] to be ValHorzCmd, got {:?}", decls[0].0);
    };
    let cst_v1::ast::TypeExpr::Atom(cst_v1::ast::TypeProd { first, .. }) = ty else {
        panic!("expected a bare TypeProd, got {ty:?}");
    };
    let cst_v1::ast::TypeApp::InlineCmdTy { args, .. } = first else {
        panic!("expected TypeApp::InlineCmdTy, got {first:?}");
    };
    assert_eq!(args.len(), 2, "{args:?}");
    let bundle = args[0].opts.as_ref().expect("first slot should carry a bundle");
    let labels: Vec<&str> = bundle.entries.iter().map(|e| e.label.name.as_str()).collect();
    assert_eq!(labels, vec!["b", "a"], "surface order preserved, not yet sorted");
    assert!(args[1].opts.is_none(), "second slot should carry no bundle");
}

/// Empty `?()` bundle in a command-type row PARSES (the grammar tolerates it
/// — non-emptiness is enforced later, at lowering: `v1/lower.rs::
/// lower_type_cmd_args`, mirroring every other `?(…)` bundle in this port).
#[test]
fn command_type_empty_opt_labels_parses_but_is_a_lower_concern() {
    assert_roundtrip_v1(
        "module M = struct\n\
         signature S = sig\n\
         val \\show : inline [?() inline-text]\n\
         end\n\
         end",
    );
}

// ============================================================================
// Math-package completion M1: `math […]` command-type head
// (`TypeApp::MathCmdTy`) — T-M1-roundtrip.
// ============================================================================

/// `math []`, `math [math-text, math-text]`, `math [?(a : int) int]` (a
/// labeled-optional row, inheriting inc3a's `TypeCmdArgItemV1.opts` for
/// free), and `math [list (math-text * inline-text)]` (the `\cases` shape,
/// upstream `math.satyh:394`) all round-trip byte-for-byte.
#[test]
fn math_command_type_head_round_trips() {
    assert_roundtrip_v1(
        "module M = struct\n\
         signature S = sig\n\
         val \\alpha : math []\n\
         val \\frac : math [math-text, math-text]\n\
         val \\derive : math [?(a : int) int]\n\
         val \\cases : math [list (math-text * inline-text)]\n\
         end\n\
         end",
    );
}

/// Shape assertion: the parsed tree actually carries `TypeApp::MathCmdTy`
/// (not merely round-tripping the source bytes), with the right arg count
/// and the labeled bundle on the first slot.
#[test]
fn math_command_type_head_shape() {
    let src = "module M = struct\n\
               signature S = sig\n\
               val \\derive : math [?(a : int) int, math-text]\n\
               end\n\
               end";
    let file = parse_file_v1(src).unwrap();
    let cst_v1::FileV1::Library { binds, .. } = file else {
        panic!("expected a library file");
    };
    let cst_v1::Bind::Signature { sig_, .. } = &binds[0] else {
        panic!("expected a Bind::Signature, got {:?}", binds[0]);
    };
    let cst_v1::ast::SigExpr::Bot(cst_v1::ast::SigBotV1::Sig { decls, .. }) = &*sig_.0 else {
        panic!("expected SigExpr::Bot(SigBotV1::Sig)");
    };
    let cst_v1::ast::Decl::ValHorzCmd { ty, .. } = &*decls[0].0 else {
        panic!("expected decls[0] to be ValHorzCmd, got {:?}", decls[0].0);
    };
    let cst_v1::ast::TypeExpr::Atom(cst_v1::ast::TypeProd { first, .. }) = ty else {
        panic!("expected a bare TypeProd, got {ty:?}");
    };
    let cst_v1::ast::TypeApp::MathCmdTy { args, .. } = first else {
        panic!("expected TypeApp::MathCmdTy, got {first:?}");
    };
    assert_eq!(args.len(), 2, "{args:?}");
    let bundle = args[0].opts.as_ref().expect("first slot should carry a bundle");
    let labels: Vec<&str> = bundle.entries.iter().map(|e| e.label.name.as_str()).collect();
    assert_eq!(labels, vec!["a"]);
    assert!(args[1].opts.is_none(), "second slot should carry no bundle");
}

/// Negative: `math` NOT followed by `[` in type position is still a parse
/// error (a bare `math` type name, or `math int`, never shaped up as any
/// known `TypeApp`/`TypeAtom` — `MathCmdTy` requires the bracketed list
/// immediately after the keyword, same as `InlineCmdTy`/`BlockCmdTy`).
#[test]
fn math_command_type_head_without_bracket_is_still_a_parse_error() {
    assert!(parse_file_v1(
        "module M = struct\n\
         signature S = sig\n\
         val \\show : math\n\
         end\n\
         end"
    )
    .is_err());
    assert!(parse_file_v1(
        "module M = struct\n\
         signature S = sig\n\
         val \\show : math int\n\
         end\n\
         end"
    )
    .is_err());
}
