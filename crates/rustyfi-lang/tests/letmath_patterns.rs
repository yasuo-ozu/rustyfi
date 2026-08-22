//! Pattern (not just plain-variable) parameters for
//! `let-math`/`let-inline`/`let-block` bindings, and the expression-level
//! `let-math \cmd param* = value in body` form (`Expr::LetMathIn`,
//! upstream's only command binding with a local `in` form, `parser.mly:688`).

use rustyfi_backend::{FontKey, FontMetrics, Length};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, CompileError};

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
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    let mono = Mono;
    let mut interp = eval::Interp::new(&mono);
    Ok(interp.eval(&env, &rustyfi_lang::ast::debrand(&program.body, &store))?)
}

fn as_length(v: Value) -> Length {
    match v {
        Value::Length(l) => l,
        other => panic!("expected a length, got {other:?}"),
    }
}

fn natural_width(v: Value) -> Length {
    match v {
        Value::Tuple(vs) if vs.len() == 3 => as_length(vs.into_iter().next().unwrap()),
        other => panic!("expected a (length * length * length) tuple, got {other:?}"),
    }
}

/// Shared dummy `let-inline` command, just so `get-initial-context` has a
/// `[math] inline-cmd` to install (unrelated to what each test actually
/// exercises).
const DUMMY_CTX_CMD: &str = "let-inline ctx \\dummy m = inline-nil\n";

#[test]
fn let_math_tuple_pattern_param() {
    // `\fst (m, _) = m`: not all-var (`(m, _)` is `PatBot::Paren`), so this
    // exercises `curry_cmd_params`'s general (per-param `Match`) path. The
    // math-mode `!(e, e)` escape (`MathArgBody::ParenEscape`) supplies the
    // tuple argument directly as a value — `elaborate.rs`'s `paren_body`
    // folds a 2+-element comma list to `Ast::Tuple` — so
    // `\fst!(${xx}, ${y})` applies `Tuple([MathText(xx), MathText(y)])`
    // against `(m, _)`, binding `m` to `${xx}`'s math value and discarding
    // `${y}`'s.
    let base = |cmd: &str| {
        let src = format!(
            "let-math \\fst (m, _) = m\n\
             {DUMMY_CTX_CMD}\
             in
             let ctx = get-initial-context 200pt (command \\dummy) in
             get-natural-metrics (embed-math ctx {cmd})"
        );
        natural_width(run(&src).unwrap())
    };
    let w_fst = base(r"${\fst!(${xx}, ${y})}");
    let w_xx = base(r"${xx}");
    assert_eq!(
        w_fst, w_xx,
        "`\\fst!(${{xx}}, ${{y}})` should bind `m` to `${{xx}}`'s math value \
         via the (m, _) tuple pattern, discarding ${{y}}"
    );
}

#[test]
fn let_math_wildcard_and_literal_patterns() {
    // `\snd (_, m) = m`: a leading wildcard discards the first tuple slot.
    // `\pick0 (0, m) = m`: a literal-int pattern in the first slot, matched
    // against a supplied literal `0`.
    let src = format!(
        "let-math \\snd (_, m) = m\n\
         let-math \\pick0 (0, m) = m\n\
         {DUMMY_CTX_CMD}\
         in
         let ctx = get-initial-context 200pt (command \\dummy) in
         let (w-snd, _, _) = get-natural-metrics (embed-math ctx ${{\\snd!(${{xx}}, ${{y}})}}) in
         let (w-pick0, _, _) = get-natural-metrics (embed-math ctx ${{\\pick0!(0, ${{y}})}}) in
         let (w-y, _, _) = get-natural-metrics (embed-math ctx ${{y}}) in
         (w-snd, w-pick0, w-y)"
    );
    match run(&src).unwrap() {
        Value::Tuple(vs) if vs.len() == 3 => {
            let mut it = vs.into_iter();
            let w_snd = as_length(it.next().unwrap());
            let w_pick0 = as_length(it.next().unwrap());
            let w_y = as_length(it.next().unwrap());
            assert_eq!(
                w_snd, w_y,
                "wildcard-first tuple pattern should bind `m` to ${{y}}"
            );
            assert_eq!(
                w_pick0, w_y,
                "literal-`0`-first tuple pattern should match and bind `m` to ${{y}}"
            );
        }
        other => panic!("expected a (length * length * length) tuple, got {other:?}"),
    }
}

#[test]
fn let_math_optional_marker_composes_with_pattern_param() {
    // `\foo ?:o (a, b) = (match o with None -> a | Some(n) -> n)`: params =
    // `[Optional(o), Pat((a, b))]` — not all-var (the second param is a
    // tuple pattern), so `curry_cmd_params` takes the general path for
    // EVERY param, including the widened `Optional -> PatBot::Var(o)`. A
    // bare call supplying only the tuple argument (no `?:`/`?*` at all) must
    // still auto-pad `None` for the leading optional slot
    // (`leading_optional_count` over the ORIGINAL `Param` list, read before
    // `params_to_patbots` widens it — `walk_bindings`'s `LetMath` arm).
    let src = format!(
        "let-math \\foo ?:o (a, b) = (match o with None -> a | Some(n) -> n)\n\
         {DUMMY_CTX_CMD}\
         in
         let ctx = get-initial-context 200pt (command \\dummy) in
         let (w-bare, _, _) = get-natural-metrics (embed-math ctx ${{\\foo!(${{xx}}, ${{y}})}}) in
         let (w-supplied, _, _) = get-natural-metrics (embed-math ctx ${{\\foo?:{{www}}!(${{xx}}, ${{y}})}}) in
         let (w-xx, _, _) = get-natural-metrics (embed-math ctx ${{xx}}) in
         (w-bare, w-supplied, w-xx)"
    );
    match run(&src).unwrap() {
        Value::Tuple(vs) if vs.len() == 3 => {
            let mut it = vs.into_iter();
            let w_bare = as_length(it.next().unwrap());
            let w_supplied = as_length(it.next().unwrap());
            let w_xx = as_length(it.next().unwrap());
            assert_eq!(
                w_bare, w_xx,
                "a marker-less bare call must auto-pad `None` for the leading \
                 `?:`-marked slot and fall back to `a` (${{xx}})"
            );
            assert!(
                w_supplied > w_bare,
                "an explicitly `?:`-supplied override (`${{www}}`, 3 chars) \
                 should be wider than the auto-padded fallback (`${{xx}}`, 2 chars)"
            );
        }
        other => panic!("expected a (length * length * length) tuple, got {other:?}"),
    }
}

#[test]
fn let_inline_tuple_pattern_param_both_forms() {
    const PICK_CMD: &str =
        "let-inline ctx \\pick-ctx (a, b) = read-inline ctx (if a then { AAAA } else { B })
let-inline \\pick-light (a, b) = if a then { AAAA } else { B }
let-inline ctx \\math m = inline-nil
";
    let base = |cmd: &str| {
        let src = format!(
            "{PICK_CMD}in
             let base = get-initial-context 200pt (command \\math) in
             let (w, _, _) = get-natural-metrics (read-inline base {{ {cmd} }}) in
             w"
        );
        as_length(run(&src).unwrap())
    };
    let w_ctx_true = base(r"\pick-ctx(true, 1);");
    let w_ctx_false = base(r"\pick-ctx(false, 1);");
    let w_light_true = base(r"\pick-light(true, 1);");
    let w_light_false = base(r"\pick-light(false, 1);");
    assert!(
        w_ctx_true > w_ctx_false,
        "the `ctx`-headed form's `(a, b)` pattern param should bind `a` to \
         the supplied bool and select the 4-char branch"
    );
    assert!(
        w_light_true > w_light_false,
        "the lightweight (implicit-`%context`) form's `(a, b)` pattern param \
         should bind `a` the same way, inside the `read-inline %context ..` \
         wrapping"
    );
}

#[test]
fn let_math_refutable_pattern_param_fails_at_apply_time() {
    // `\lit (0, m) = m`, called with a first tuple component of `1`, not
    // `0` — the per-param `Match` (not `rec_clause_value`'s tuple match)
    // `curry_cmd_params`'s general path lowers to has no matching arm, so
    // evaluation (not parsing or typechecking) must fail.
    let src = format!(
        "let-math \\lit (0, m) = m\n\
         {DUMMY_CTX_CMD}\
         in
         let ctx = get-initial-context 200pt (command \\dummy) in
         let (w, _, _) = get-natural-metrics (embed-math ctx ${{\\lit!(1, ${{xx}})}}) in
         w"
    );
    match run(&src) {
        Err(_) => {}
        Ok(v) => panic!("expected a runtime match failure, got {v:?}"),
    }
}

// Expression-level `let-math \cmd param* = value in body` (`Expr::LetMathIn`).

#[test]
fn expression_level_let_math_in() {
    // `\g m = ${#m#m}` splices `m`'s math value twice, so applying it to
    // `${x}` (1 char) should render as wide as `${xx}` (2 chars) under this
    // harness's per-char-fixed-advance `Mono` font.
    //
    // `\math` sits BEFORE the file's own top `in` (a genuine top-level
    // prelude binding — `let-inline` has no expression-level `in` form);
    // `let-math \g .. in ..` only then appears in the file's BODY
    // expression, proving it parses as `Expr::LetMathIn` there rather than
    // being swallowed as a `TopBinding::LetMath` prelude item with the
    // following `in` reinterpreted as the file's own.
    let src = "let-inline ctx \\math m2 = inline-nil
in
let-math \\g m = ${#m#m} in
let ctx = get-initial-context 200pt (command \\math) in
let (w-g, _, _) = get-natural-metrics (embed-math ctx ${\\g{x}}) in
let (w-xx, _, _) = get-natural-metrics (embed-math ctx ${xx}) in
(w-g, w-xx)";
    match run(src).unwrap() {
        Value::Tuple(vs) if vs.len() == 2 => {
            let mut it = vs.into_iter();
            let w_g = as_length(it.next().unwrap());
            let w_xx = as_length(it.next().unwrap());
            assert_eq!(
                w_g, w_xx,
                "\\g{{x}} (double-splicing a 1-char math value) should be as \
                 wide as ${{xx}} (a literal 2-char run)"
            );
        }
        other => panic!("expected a (length * length) tuple, got {other:?}"),
    }
}

#[test]
fn expression_level_let_math_in_nested_under_a_plain_let() {
    // The same `let-math .. in` form, but nested one level deeper under an
    // ordinary `let .. in` — proving it really is usable at arbitrary
    // expression position, not merely "first form after the file's own
    // prelude" (which could be confused with a disguised top-level form).
    let src = "let-inline ctx \\math m = inline-nil in
let result =
  let-math \\g m = ${#m#m} in
  let ctx = get-initial-context 200pt (command \\math) in
  get-natural-metrics (embed-math ctx ${\\g{x}})
in
let (w-g, _, _) = result in
let (w-xx, _, _) =
  let ctx = get-initial-context 200pt (command \\math) in
  get-natural-metrics (embed-math ctx ${xx})
in
(w-g, w-xx)";
    match run(src).unwrap() {
        Value::Tuple(vs) if vs.len() == 2 => {
            let mut it = vs.into_iter();
            let w_g = as_length(it.next().unwrap());
            let w_xx = as_length(it.next().unwrap());
            assert_eq!(
                w_g, w_xx,
                "let-math .. in nested under a plain let .. in should still work"
            );
        }
        other => panic!("expected a (length * length) tuple, got {other:?}"),
    }
}

// An expression-level `let-math` binding whose value isn't `math` must fail
// typechecking (`math_command_scheme`, reached from `Expr::LetMathIn` too).

#[test]
fn expression_level_let_math_in_non_math_value_fails_typecheck() {
    let src = "let-math \\f = 3 in 0";
    match run(src) {
        Err(_) => {}
        Ok(v) => {
            panic!("expected a typecheck error (a math command's value must be `math`), got {v:?}")
        }
    }
}
