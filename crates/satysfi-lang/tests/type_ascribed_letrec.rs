//! Acceptance coverage for Gap 1 (`class-signature-lang-gaps.md`-style):
//! type-ascribed multi-clause `let-rec`, i.e. `cst.rs`'s `RecBinding` now
//! accepting an optional `: ty` between the bound name and its
//! `[|] patbot* = value` clause group(s) — the shape upstream's `parser.mly`
//! calls `recdecargpart`'s `COLON ty BAR argpatlst` alternative, needed by
//! the bundled `itemize.satyh`'s `listing-item`. The ascription is parsed and
//! otherwise ignored (no signature-enforcement pass exists yet for
//! value-level ascriptions — see `RecBinding`'s doc comment); what this
//! closes is purely the *parse* collision between `: ty` and the `|`-clause
//! sugar, so every test below also typechecks/evaluates to prove the
//! existing multi-clause machinery (`elaborate.rs`'s `rec_clause_value`) is
//! untouched by the new field.

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck, CompileError};

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

fn run(src: &str) -> Result<Value, CompileError> {
    let file = satysfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    Ok(interp.eval(&env, &program.body)?)
}

fn int(src: &str) -> i64 {
    match run(src).unwrap() {
        Value::Int(n) => n,
        other => panic!("{src:?} evaluated to {other:?}, not an int"),
    }
}

/// The acceptance example from the task: a *local* (`Expr::LetRecIn`)
/// ascribed multi-clause `let-rec` parses, typechecks, and evaluates.
#[test]
fn local_ascribed_multi_clause_let_rec() {
    let src = "in
let-rec f : int -> int
  | 0 = 0
  | n = f (n - 1)
in f 3";
    assert_eq!(int(src), 0);
}

/// The same shape at the top level (`TopBinding::LetRec`, `nxtoplevel`'s
/// alternative rather than `nxlet`'s) — the form `itemize.satyh`'s
/// `listing-item` actually uses (a module-body declaration, not a nested
/// `let .. in`). Also exercises the ascription unifying with a genuinely
/// recursive (accumulating) clause body, not just a base case passthrough.
#[test]
fn top_level_ascribed_multi_clause_let_rec() {
    let src = "let-rec sum : int -> int
  | 0 = 0
  | n = n + sum (n - 1)
in sum 3";
    assert_eq!(int(src), 6);
}

/// `itemize.satyh`'s real `listing-item` binding, verbatim (module context
/// stripped since this is a parse-only check — none of `context`/`itemize`/
/// `Item`/the various `Gr`/pervasives helpers need to actually resolve for
/// the CST to parse; only `satysfi_syntax::parse_file`, not the full
/// elaborate/typecheck/eval pipeline, runs here). Confirms the exact
/// `COLON ty` + single-`|`-clause shape (no second `EndOfList` clause in the
/// real source) parses end to end.
#[test]
fn itemize_listing_item_clause_shape_parses() {
    let src = "let-rec listing-item : context -> int -> bool -> bool -> itemize -> block-boxes
    | ctx depth is-first is-last (Item(parent, children)) =
        let ib-bullet = make-bullet ctx in
        let bullet-width = get-natural-width ib-bullet in
        let parent-indent = item-indent *' (float depth) in
        let ib-parent =
          embed-block-top ctx ((get-text-width ctx) -' parent-indent -' bullet-width) (fun ctx ->
            form-paragraph (ctx |> set-paragraph-margin item-gap item-gap)
              (read-inline ctx parent ++ inline-fil)
          )
        in
        let bb-parent =
          form-paragraph (ctx |> set-paragraph-margin item-gap item-gap)
            ((inline-skip parent-indent) ++ ib-bullet ++ ib-parent)
        in
        let bbs-children = List.map-with-ends (listing-item ctx (depth + 1)) children in
        bb-parent +++> bbs-children
in 0";
    let result = satysfi_syntax::parse_file(src);
    assert!(
        result.is_ok(),
        "itemize.satyh's listing-item clause shape failed to parse: {:?}",
        result.err()
    );
}
