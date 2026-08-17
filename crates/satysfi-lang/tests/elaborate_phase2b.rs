//! End-to-end coverage for the phase-2b elaborator additions: real SATySFi
//! *source text* run through `parse_file` -> `elaborate` -> `eval::Interp`,
//! exercising `let-mutable`/`<-`/`while`/`before` (through the
//! paren-wrapping the grammar requires to attach `before` to a non-`OpChain`
//! construct — see `cst.rs`'s doc comment on `OpChain::before`), `#label`
//! field access, `(| .. with .. |)` record update, itemize-tree
//! reconstruction, quoted math values, (untyped) modules/`open`, the
//! backtick-literal space/indentation omission, and the untyped `Some`/`None`
//! desugaring of supplied/omitted optional arguments (`?:`/`?*`).

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::ast::{Ast, IText};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives};

struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, c: char, size: Length) -> Option<Length> {
        if c.is_ascii() {
            Some(size * 0.5)
        } else {
            None
        }
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

fn eval_str(src: &str) -> Result<Value, satysfi_lang::CompileError> {
    let file = satysfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let ast = elaborate::elaborate(&file, &scope)?;
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    Ok(interp.eval(&env, &ast)?)
}

fn int(src: &str) -> i64 {
    match eval_str(src).unwrap() {
        Value::Int(n) => n,
        other => panic!("{src:?} evaluated to {other:?}, not an int"),
    }
}

fn elaborate_only(src: &str) -> Ast {
    let file = satysfi_syntax::parse_file(src).unwrap();
    let scope = elaborate::Scope::new(primitives::base_env().names());
    elaborate::elaborate(&file, &scope).unwrap()
}

// ---- let-mutable / overwrite / while / before, from source -----------------

#[test]
fn counter_via_let_mutable_and_before() {
    // `before` (`OpChain::before`) only attaches to an operator-chain head,
    // never directly to `Overwrite`/`WhileDo` (a documented CST
    // simplification — see `cst.rs`'s doc comment on `OpChain`), so an
    // `Overwrite`/`WhileDo` used as the left operand of `before` must be
    // parenthesized first (a plain `Paren` atomic, which *is* a valid
    // `OpChain` head).
    let src = "let-mutable c <- 0 in (c <- !c + 2) before !c";
    assert_eq!(int(src), 2);
}

#[test]
fn while_loop_sums_one_through_five() {
    let src = "\
        let-mutable sum <- 0 in \
        let-mutable i <- 0 in \
        (while !i < 5 do (i <- !i + 1) before (sum <- !sum + !i)) before !sum";
    assert_eq!(int(src), 15);
}

// ---- field access / record update, from source -----------------------------

#[test]
fn field_access_on_a_record_literal() {
    assert_eq!(int("let r = (| a = 1; b = 2 |) in r#a"), 1);
    assert_eq!(int("let r = (| a = 1; b = 2 |) in r#b"), 2);
}

#[test]
fn record_update_replaces_one_field_and_keeps_the_rest() {
    let src = "\
        let r = (| a = 1; b = 2 |) in \
        let r2 = (| r with a = 10 |) in \
        r2#a";
    assert_eq!(int(src), 10);
    let src_b = "\
        let r = (| a = 1; b = 2 |) in \
        let r2 = (| r with a = 10 |) in \
        r2#b";
    assert_eq!(int(src_b), 2);
}

// ---- itemize tree shape -----------------------------------------------------

/// Extract the plain-text content of an `IText::Text` sequence with no
/// commands/embeds (as produced by `{ .. }` segments containing only bare
/// characters/spaces).
fn plain_text(elems: &[IText]) -> String {
    elems
        .iter()
        .map(|e| match e {
            IText::Text(s) => s.clone(),
            other => panic!("expected plain IText::Text, got {other:?}"),
        })
        .collect()
}

/// Unwrap one `Ctor("Item", (text, children))` node into its parts, as
/// produced by the elaborator's itemize-tree lowering (see `elaborate.rs`'s
/// `itemize`/`item_node_to_ast`).
fn unwrap_item(ast: &Ast) -> (String, Vec<Ast>) {
    match ast {
        Ast::Ctor(name, Some(payload)) => {
            assert_eq!(name, "Item");
            match payload.as_ref() {
                Ast::Tuple(parts) if parts.len() == 2 => {
                    let text = match &parts[0] {
                        Ast::InlineText(elems) => plain_text(elems),
                        other => panic!("expected InlineText, got {other:?}"),
                    };
                    let children = match &parts[1] {
                        Ast::List(items) => items.clone(),
                        other => panic!("expected List, got {other:?}"),
                    };
                    (text, children)
                }
                other => panic!("expected a 2-tuple payload, got {other:?}"),
            }
        }
        other => panic!("expected Ctor(\"Item\", ..), got {other:?}"),
    }
}

#[test]
fn itemize_builds_ctor_item_tree_with_correct_depths() {
    // `{ * a * b ** c }`: two depth-1 items ("a", "b"), then a depth-2 item
    // ("c") nested under "b" (the *last* depth-1 item so far) — transcribed
    // from parser.mly's `make_list_to_itemize`/`insert_last`.
    let ast = elaborate_only("{ * a * b ** c }");
    let (root_text, root_children) = unwrap_item(&ast);
    assert_eq!(root_text, "", "the dummy root item has empty text");
    assert_eq!(root_children.len(), 2, "two depth-1 items: 'a' and 'b'");

    let (a_text, a_children) = unwrap_item(&root_children[0]);
    assert_eq!(a_text.trim(), "a");
    assert!(a_children.is_empty(), "'a' has no nested items");

    let (b_text, b_children) = unwrap_item(&root_children[1]);
    assert_eq!(b_text.trim(), "b");
    assert_eq!(b_children.len(), 1, "'c' nests under 'b'");

    let (c_text, c_children) = unwrap_item(&b_children[0]);
    assert_eq!(c_text.trim(), "c");
    assert!(c_children.is_empty());
}

#[test]
fn itemize_rejects_an_illegal_depth_jump() {
    // Depth 3 directly after depth 1 (skipping depth 2) is illegal
    // (parser.mly:343, `"syntax error: illegal item depth .."`).
    let file = satysfi_syntax::parse_file("{ * a *** b }").unwrap();
    let scope = elaborate::Scope::new(primitives::base_env().names());
    let err = elaborate::elaborate(&file, &scope).unwrap_err();
    assert!(err.to_string().contains("illegal item depth"));
}

#[test]
fn inline_text_without_bullets_stays_plain_inline_text() {
    let ast = elaborate_only("{ hello }");
    match ast {
        Ast::InlineText(_) => {}
        other => panic!("expected plain InlineText, got {other:?}"),
    }
}

// ---- quoted math ------------------------------------------------------------

#[test]
fn math_text_quotes_to_a_math_value() {
    let val = eval_str("${x^2}").unwrap();
    assert!(
        matches!(val, Value::MathText { .. }),
        "expected a math value, got {val:?}"
    );
}

#[test]
fn qualified_math_command_resolves_to_the_mangled_key() {
    // `\M.cmd` in math mode should elaborate against the module-qualified
    // scope key `"M.\cmd"` (see `qualify_key`'s doc comment on the
    // name-mangling scheme), the same way `\M.cmd` already does in inline
    // text.
    let src = "module M = struct let-math \\cmd x = x end in ${\\M.cmd{1}}";
    let ast = elaborate_only(src);
    // `module M = struct .. end in body` elaborates `\cmd`'s `let-math`
    // binding under a collision-proof MANGLED key (`"$M.\cmd"` — module-
    // member bug fix: `push_named_binding` no longer binds a separate bare
    // `"\cmd"` `LetMathIn` that a later reference could shadow something
    // else with, see that function's doc comment), wrapping a `LetIn` that
    // re-binds the module-qualified key `M.\cmd` to `Var("$M.\cmd")`,
    // wrapping `body`; unwrap both to reach the `${..}` literal.
    let satysfi_lang::ast::Ast::LetMathIn(mangled_name, _value, rest) = ast else {
        panic!("expected LetMathIn at the top, got {ast:?}");
    };
    assert_eq!(mangled_name, "$M.\\cmd");
    let satysfi_lang::ast::Ast::LetIn(qualified_name, _alias_value, body) = *rest else {
        panic!("expected LetIn (the qualified alias) nested inside, got {rest:?}");
    };
    assert_eq!(qualified_name, "M.\\cmd");
    let satysfi_lang::ast::Ast::MathText(elems) = *body else {
        panic!("expected a MathText literal, got {body:?}");
    };
    assert_eq!(elems.len(), 1);
    match &elems[0] {
        satysfi_lang::ast::MathElem::Cmd { name, .. } => {
            assert_eq!(name, "M.\\cmd");
        }
        other => panic!("expected MathElem::Cmd, got {other:?}"),
    }
}

#[test]
fn unqualified_reference_to_a_module_only_math_command_is_unbound() {
    // Without the `M.` qualifier, `\cmd` is never brought into scope by a
    // `module M = struct .. end` binding (mirrors
    // `module_unqualified_name_is_out_of_scope_after_end` for plain `let`).
    let src = "module M = struct let-math \\cmd x = x end in ${\\cmd{1}}";
    let err = eval_str(src).unwrap_err();
    assert!(
        err.to_string().contains("unbound"),
        "expected an unbound-command error, got {err}"
    );
}

// ---- modules / open ---------------------------------------------------------

#[test]
fn module_qualified_reference_resolves() {
    let src = "module M = struct let x = 1 end in M.x";
    assert_eq!(int(src), 1);
}

#[test]
fn module_unqualified_name_is_out_of_scope_after_end() {
    // Per v0.0.6 semantics: after `end`, only the qualified name is visible.
    let src = "module M = struct let x = 1 end in x";
    let err = eval_str(src).unwrap_err();
    assert!(err.to_string().contains("unbound"));
}

#[test]
fn open_brings_a_qualified_name_into_unqualified_scope() {
    let src = "module M = struct let x = 1 end in open M in x";
    assert_eq!(int(src), 1);
}

#[test]
fn nested_module_mangles_recursively() {
    let src = "\
        module M = struct \
            module N = struct let x = 1 end \
        end in M.N.x";
    assert_eq!(int(src), 1);
}

// ---- backtick-literal space/indentation omission ----------------------------

#[test]
fn multiline_literal_strips_common_indentation() {
    // Body (between backticks): "\n  first\n  second\n  third\n". A bare
    // backtick literal defaults to `omit_pre = true` / `omit_post = true`
    // (see `lexer.rs`); `omit_post` strips exactly the *one* trailing
    // newline before the closing backtick, then `min_indent_space`/
    // `shave_indent` (parser.mly's header, transcribed in `elaborate.rs`'s
    // `omit_spaces`) strip the common 2-space indentation from every *real*
    // content line. The blank leading line (an immediate `\n` right after
    // the opening backtick) is not itself removed — `omit_pre` only strips
    // literal `' '` characters, never `'\n'` — so it survives as a literal
    // leading newline in the resulting string; this is a faithful
    // transcription of v0.0.6's `omit_spaces`, not a bug in this port.
    let src = "let s = `\n  first\n  second\n  third\n` in s";
    match eval_str(src).unwrap() {
        Value::Str(s) => assert_eq!(s, "\nfirst\nsecond\nthird"),
        other => panic!("expected a string, got {other:?}"),
    }
}

#[test]
fn single_line_literal_is_unaffected_by_omit_spaces() {
    match eval_str("let s = `hello` in s").unwrap() {
        Value::Str(s) => assert_eq!(s, "hello"),
        other => panic!("expected a string, got {other:?}"),
    }
}

// ---- optional / omitted arguments --------------------------------------------

#[test]
fn optional_application_desugars_to_some() {
    // `f ?:(e)` (`AppArg::Optional`) desugars, untyped, straight to
    // `Apply(f, Some(e))` — the call-site model `frontend-completion.md`
    // Sub-area 2 specifies (see `elaborate.rs`'s `app_arg_to_ast`).
    assert_eq!(
        int("(fun x -> match x with | Some n -> n | None -> 0) ?:(5)"),
        5
    );
}

#[test]
fn omission_application_desugars_to_none() {
    // `f ?*` (`AppArg::Omission`) desugars to `Apply(f, None)`.
    assert_eq!(
        int("(fun x -> match x with | Some n -> n | None -> 0) ?*"),
        0
    );
}
