//! The primitive registry. Shaped so the ~300 vminst instructions can be
//! ported one `prims!` line at a time; primitives are registered under their
//! real v0.0.6 names so later stdlib loading finds them.
//!
//! Milestone 1 used to hardcode `document`, `+p`, and `\emph` as natives.
//! Phase 4 deletes them: the real definitions now live in the `stdja-mini`
//! stdlib package (`lib-satysfi/dist/packages/stdja-mini.satyh`), loaded
//! through `satysfi-loader` and typechecked/evaluated exactly like any other
//! `.satyh` library. See that file's header comment for the small set of
//! primitives it's built from (`get-initial-context`, `page-break`,
//! `read-block`/`read-inline`, `line-break`, `++`/`inline-fil`, and the new
//! `set-font-key` below).

use crate::ast::{BText, IText};
use crate::eval::{eval_error, EvalError, Interp};
use crate::value::{DocumentValue, Env, Value};
use satysfi_backend::{
    break_into_lines, break_pages, Context, FontKey, HorzBox, HorzStringInfo, Length, PageGeometry,
    PureHorzBox, VertBox,
};
use std::rc::Rc;

/// Font keys agreed with the milestone-1 base-14 metrics provider.
pub const FONT_REGULAR: FontKey = FontKey(0);
pub const FONT_BOLD: FontKey = FontKey(1);
pub const FONT_OBLIQUE: FontKey = FontKey(2);

pub struct PrimDef {
    pub name: &'static str,
    pub arity: usize,
    pub run: fn(&mut Interp, Vec<Value>) -> Result<Value, EvalError>,
}

impl std::fmt::Debug for PrimDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PrimDef({}/{})", self.name, self.arity)
    }
}

macro_rules! prims {
    ($($name:literal ($arity:literal) => $f:path;)*) => {
        static PRIM_DEFS: &[PrimDef] = &[
            $(PrimDef { name: $name, arity: $arity, run: $f },)*
        ];
    };
}

prims! {
    "read-inline" (2) => prim_read_inline;
    "read-block" (2) => prim_read_block;
    // v0.0.6 (vminst.ml `BackendLineBreaking`): `bool -> bool -> context ->
    // inline-boxes -> block-boxes` — the two leading bools select whether
    // the paragraph's top/bottom edge is breakable across a page boundary.
    "line-break" (4) => prim_line_break;
    "page-break" (2) => prim_page_break;

    // ---- int arithmetic (vminst.ml: Plus/Minus/Times/Divides/Mod) --------
    "+" (2) => prim_int_add;
    "-" (2) => prim_int_sub;
    "*" (2) => prim_int_mul;
    "/" (2) => prim_int_div;
    "mod" (2) => prim_int_mod;

    // ---- int comparisons (vminst.ml: EqualTo/GreaterThan/LessThan; the
    // "<>"/">="/"<=" trio comes from primitives.cppo.ml's `general_table`,
    // defined there as `LogicalNot (EqualTo ..)` / `LogicalNot (LessThan ..)`
    // / `LogicalNot (GreaterThan ..)`, typed `int -> int -> bool`) ----------
    "==" (2) => prim_int_eq;
    "<>" (2) => prim_int_ne;
    "<" (2) => prim_int_lt;
    ">" (2) => prim_int_gt;
    "<=" (2) => prim_int_le;
    ">=" (2) => prim_int_ge;

    // ---- bool (vminst.ml: LogicalAnd/LogicalOr/LogicalNot) ----------------
    // NOTE: registered here as strict 2-arg primitives (both arguments are
    // evaluated before the call, since primitive application is call-by-
    // value). Real SATySFi short-circuits "&&"/"||" via elaboration
    // (build-in `if`); that desugaring lives in the (out-of-scope) elaborator.
    "&&" (2) => prim_bool_and;
    "||" (2) => prim_bool_or;
    "not" (1) => prim_bool_not;

    // ---- float (vminst.ml: FloatPlus/FloatMinus/FloatTimes/FloatDivides,
    // PrimitiveFloat, PrimitiveRound) ---------------------------------------
    "+." (2) => prim_float_add;
    "-." (2) => prim_float_sub;
    "*." (2) => prim_float_mul;
    "/." (2) => prim_float_div;
    "float" (1) => prim_float_of_int;
    "round" (1) => prim_round;

    // ---- length arithmetic (vminst.ml: LengthPlus/LengthMinus/LengthTimes/
    // LengthDivides/LengthLessThan/LengthGreaterThan) -----------------------
    "+'" (2) => prim_length_add;
    "-'" (2) => prim_length_sub;
    "*'" (2) => prim_length_scale;
    "/'" (2) => prim_length_div;
    "<'" (2) => prim_length_lt;
    ">'" (2) => prim_length_gt;

    // ---- string (vminst.ml: Concat, PrimitiveArabic, PrimitiveSame) -------
    "^" (2) => prim_string_concat;
    "arabic" (1) => prim_arabic;
    "string-same" (2) => prim_string_same;

    // ---- list cons ---------------------------------------------------------
    // v0.0.6 makes `::` syntax (`UTListCons`/`ListCons` in vminst.ml), not a
    // primitive. This port's elaborator flattens *every* binary operator
    // (see `elaborate.rs`'s operator-precedence fold) uniformly into
    // `Apply(Apply(Var(op_text), lhs), rhs)`, so `::` needs an env-bound
    // primitive of its own to resolve the same way as `+`/`^`/etc.
    "::" (2) => prim_list_cons;

    // ---- mutable-cell dereference (evaluator.cppo.ml `Dereference`) -------
    // v0.0.6 does not evaluate `!` as an ordinary primitive body: it's bound
    // in primitives.cppo.ml as `( "!", ptyderef, lambda1 (fun v1 ->
    // Dereference(v1)) )`, i.e. applying "!" *constructs* a `Dereference`
    // AST node that a later interpreter pass then reduces. This port has no
    // such two-step "construct-then-reduce" split for built-ins, so "!" is
    // registered as an ordinary strict primitive that performs the
    // dereference directly — a structural deviation, not a semantic one.
    "!" (1) => prim_deref;

    // ---- string, continued (vminst.ml: PrimitiveStringLength/StringSub/
    // StringExplode; low-priority additions verified against vminst.ml) ----
    "string-length" (1) => prim_string_length;
    "string-sub" (3) => prim_string_sub;
    "string-explode" (1) => prim_string_explode;

    // ---- text embedding (vminst.ml:1707 PrimitiveEmbed: string -> inline-
    // text; the interp body wraps the string as a one-element quoted text) --
    "embed-string" (1) => prim_embed_string;

    // ---- context ops (phase 4, part 1 — inventory for a future .saty
    // `document`/`+p`/`\emph`) ------------------------------------------
    //
    // vminst.ml:1434 `PrimitiveSetFontSize`: `~% (tLN @-> tCTX @-> tCTX)`.
    "set-font-size" (2) => prim_set_font_size;
    // vminst.ml:1449 `PrimitiveGetFontSize`: `~% (tCTX @-> tLN)`.
    "get-font-size" (1) => prim_get_font_size;
    // vminst.ml:1633 `PrimitiveSetLeading`: `~% (tLN @-> tCTX @-> tCTX)`,
    // sets `ctx.leading` — the baseline-to-baseline distance, which is
    // exactly our existing `Context::leading` field. (There is *also* a
    // `set-min-gap-of-lines`, vminst.ml:1291-1292, which sets a *different*
    // field, `min_gap_of_lines` — the minimum extra gap between two lines'
    // bounding boxes, on top of `leading`. We don't model that separate
    // field, so `set-leading` is the one that matches "baseline distance"
    // and an existing Context field.)
    "set-leading" (2) => prim_set_leading;
    // vminst.ml:1396 `PrimitiveSetParagraphMargin`:
    // `~% (tLN @-> tLN @-> tCTX @-> tCTX)`. Sets the new `paragraph_top`/
    // `paragraph_bottom` fields (see context.rs); not wired into any
    // box-producing primitive yet (a future `+p` would consult them).
    "set-paragraph-margin" (3) => prim_set_paragraph_margin;
    // vminst.ml:1648 `PrimitiveGetTextWidth`: `~% (tCTX @-> tLN)`.
    "get-text-width" (1) => prim_get_text_width;
    // vminst.ml:1247 `PrimitiveGetInitialContext`:
    // `~% (tLN @-> tICMD tMATH @-> tCTX)` — a paragraph width and the
    // *default math command* (the handler used for bare `${...}` math
    // embedded directly in inline text) to seed `context_main.math_command`
    // with. This port has no math typesetting at all yet (deferred to
    // phase 7), so the second argument is accepted (to keep the arity/
    // signature shape faithful to v0.0.6) and simply ignored; only the
    // `length` argument feeds `Context::initial`.
    "get-initial-context" (2) => prim_get_initial_context;
    // LOCAL, non-upstream primitive: `set-font-key : int -> context ->
    // context`, sets `Context::font` directly to `FontKey(n)`. v0.0.6 has no
    // primitive shaped like this at all — real font switching there goes
    // through `set-font : script -> (string * float * float) -> context ->
    // context` (choosing a font *by name* per script, vminst.ml's
    // `PrimitiveSetFont`), which is far richer than this milestone's
    // base-14-metrics-by-`FontKey` model can support. `set-font-key` is the
    // minimal faithful-enough stand-in the `stdja-mini` stdlib package
    // (lib-satysfi/dist/packages/stdja-mini.satyh) needs to implement
    // `\emph`/`\bold` by switching to the oblique/bold base-14 face
    // (`FONT_OBLIQUE`/`FONT_BOLD` above) without inventing a whole font-name
    // resolution layer. Out-of-range keys are accepted as-is (there is no
    // registry to validate against yet); an unknown `FontKey` simply fails
    // later, when a font metrics lookup for it comes up empty.
    "set-font-key" (2) => prim_set_font_key;

    // ---- box combinators (vminst.ml `HorzConcat`/`VertConcat`/
    // `BackendVertSkip`/`BackendFixedEmpty`/`BackendOuterEmpty`) ----------
    //
    // vminst.ml:803 `HorzConcat`: `~% (tIB @-> tIB @-> tIB)`.
    "++" (2) => prim_inline_concat;
    // vminst.ml:818 `VertConcat`: `~% (tBB @-> tBB @-> tBB)`.
    "+++" (2) => prim_block_concat;
    // vminst.ml:1757 `BackendFixedEmpty`: `~% (tLN @-> tIB)` — a fixed-width
    // box with no stretch/shrink (`PureHorzBox::FixedEmpty`, hbox.rs).
    "inline-skip" (1) => prim_inline_skip;
    // vminst.ml:1771 `BackendOuterEmpty`: `~% (tLN @-> tLN @-> tLN @-> tIB)`,
    // params `(widnat, widshrink, widstretch)` in that order — exactly the
    // (natural, shrinkable, stretchable) field order `PureHorzBox::OuterEmpty`
    // already uses, so this is a direct wrap, no new box variant needed.
    "inline-glue" (3) => prim_inline_glue;
    // vminst.ml:1171 `BackendVertSkip`: `~% (tLN @-> tBB)`, builds
    // `VertFixedBreakable(len)` — our existing `VertBox::Skip(len)`.
    "block-skip" (1) => prim_block_skip;

    // ---- frontend-completion.md §Slice 1.A: the ~18 pure primitives -------
    // (all use only already-existing `Value`/`BaseType`s — no backend work).
    // `|>` (reverse application) is NOT here: it is elaborated directly to
    // `Apply(f, x)` (see `elaborate.rs`'s `climb`), not a runtime primitive.

    // ---- float trig / log / exp / rounding (vminst.ml 2729-2880) ----------
    "sin" (1) => prim_sin;
    "asin" (1) => prim_asin;
    "cos" (1) => prim_cos;
    "acos" (1) => prim_acos;
    "tan" (1) => prim_tan;
    "atan" (1) => prim_atan;
    "atan2" (2) => prim_atan2;
    "log" (1) => prim_log;
    "exp" (1) => prim_exp;
    // vminst.ml:2865/2880 `PrimitiveCeil`/`PrimitiveFloor`: both `float ->
    // float` (NOT `int` — easy to mistype, see frontend-completion.md's
    // Risks section; contrast `round`, above, which does return `int`).
    "ceil" (1) => prim_ceil;
    "floor" (1) => prim_floor;
    // vminst.ml:2319 `PrimitiveShowFloat`: `float -> string`, OCaml's
    // `string_of_float`.
    "show-float" (1) => prim_show_float;

    // ---- byte-indexed string ops (vminst.ml 2056-2196) ---------------------
    // vminst.ml:2159 `PrimitiveStringByteLength`: counts UTF-8 BYTES, unlike
    // `string-length`'s Unicode-scalar-value count above.
    "string-byte-length" (1) => prim_string_byte_length;
    // vminst.ml:2123 `PrimitiveStringSubBytes`: byte-indexed `string-sub`.
    "string-sub-bytes" (3) => prim_string_sub_bytes;
    // vminst.ml:2196 `PrimitiveStringUnexplode`: inverse of `string-explode`.
    "string-unexplode" (1) => prim_string_unexplode;

    // ---- diagnostics (vminst.ml 2056, 3133) --------------------------------
    // vminst.ml:2056 `PrimitiveDisplayMessage`: `string -> unit`. Upstream
    // prints to stdout (`print_endline`); see `prim_display_message`'s doc
    // comment for why this port deliberately prints to stderr instead.
    "display-message" (1) => prim_display_message;
    // vminst.ml:3133 `AbortWithMessage`: `string -> 'a` — raises a dynamic
    // error carrying the message verbatim.
    "abort-with-message" (1) => prim_abort_with_message;
}

/// The base environment `document` programs start in.
pub fn base_env() -> Env {
    let env = Env::root();
    for def in PRIM_DEFS {
        env.define(
            def.name,
            Value::Prim {
                def,
                applied: Vec::new(),
            },
        );
    }
    env.define(
        "inline-fil",
        Value::InlineBoxes(vec![HorzBox::Pure(PureHorzBox::OuterFil)]),
    );
    // `inline-nil`/`block-nil`: no vminst.ml entry (like `inline-fil` above,
    // these aren't functions — v0.0.6 gets the empty inline-boxes/
    // block-boxes list for free from the literal `{}`/`<>` surface syntax,
    // which this port's syntax layer doesn't (yet) produce standalone; these
    // constants are the equivalent value bound to a name).
    env.define("inline-nil", Value::InlineBoxes(Vec::new()));
    env.define("block-nil", Value::BlockBoxes(Vec::new()));
    env
}

// ---- argument extractors ------------------------------------------------------

fn as_context(v: Value) -> Result<Context, EvalError> {
    match v {
        Value::Context(c) => Ok(*c),
        other => eval_error(format!("expected a context, got {}", other.type_name())),
    }
}

fn as_inline_text(v: Value) -> Result<(Rc<Vec<IText>>, Env), EvalError> {
    match v {
        Value::InlineText { elems, env } => Ok((elems, env)),
        other => eval_error(format!("expected inline-text, got {}", other.type_name())),
    }
}

fn as_block_text(v: Value) -> Result<(Rc<Vec<BText>>, Env), EvalError> {
    match v {
        Value::BlockText { elems, env } => Ok((elems, env)),
        other => eval_error(format!("expected block-text, got {}", other.type_name())),
    }
}

fn as_inline_boxes(v: Value) -> Result<Vec<HorzBox>, EvalError> {
    match v {
        Value::InlineBoxes(b) => Ok(b),
        other => eval_error(format!(
            "expected inline-boxes, got {}",
            other.type_name()
        )),
    }
}

fn as_block_boxes(v: Value) -> Result<Vec<VertBox>, EvalError> {
    match v {
        Value::BlockBoxes(b) => Ok(b),
        other => eval_error(format!("expected block-boxes, got {}", other.type_name())),
    }
}

fn as_int(v: Value) -> Result<i64, EvalError> {
    match v {
        Value::Int(n) => Ok(n),
        other => eval_error(format!("expected int, got {}", other.type_name())),
    }
}

fn as_float(v: Value) -> Result<f64, EvalError> {
    match v {
        Value::Float(x) => Ok(x),
        other => eval_error(format!("expected float, got {}", other.type_name())),
    }
}

fn as_bool(v: Value) -> Result<bool, EvalError> {
    match v {
        Value::Bool(b) => Ok(b),
        other => eval_error(format!("expected bool, got {}", other.type_name())),
    }
}

fn as_str(v: Value) -> Result<String, EvalError> {
    match v {
        Value::Str(s) => Ok(s),
        other => eval_error(format!("expected string, got {}", other.type_name())),
    }
}

fn as_length(v: Value) -> Result<Length, EvalError> {
    match v {
        Value::Length(l) => Ok(l),
        other => eval_error(format!("expected length, got {}", other.type_name())),
    }
}

/// Used by `string-unexplode` (the only new Slice-1 primitive that needs a
/// bare list, as opposed to a per-element extractor applied inside a loop).
fn as_list(v: Value) -> Result<Vec<Value>, EvalError> {
    match v {
        Value::List(items) => Ok(items),
        other => eval_error(format!("expected list, got {}", other.type_name())),
    }
}

// ---- primitive-body macros ----------------------------------------------------
//
// The arithmetic, comparison, boolean, and unary-conversion primitives all
// share one strict-call shape: the (already-evaluated) operands are popped
// right-to-left through a type extractor, then the result is re-wrapped as a
// `Value`. These macros capture that shape so each primitive is a single line.
// The vminst.ml citations for each stay on the `prims!` registration table
// above; per-primitive notes ride along on the invocations below.

/// A strict binary primitive. Pops `b` then `a` (i.e. rightmost argument
/// first, matching application order) through the given extractor(s) and wraps
/// `body` as `Value::$ctor`. Accepts either one extractor for both operands or
/// a `(as_a, as_b)` pair when the operands have different types.
macro_rules! binop_prim {
    ($name:ident, ($as_a:path, $as_b:path), $ctor:ident, |$a:ident, $b:ident| $body:expr) => {
        fn $name(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
            let $b = $as_b(args.pop().unwrap())?;
            let $a = $as_a(args.pop().unwrap())?;
            Ok(Value::$ctor($body))
        }
    };
    ($name:ident, $as:path, $ctor:ident, |$a:ident, $b:ident| $body:expr) => {
        binop_prim!($name, ($as, $as), $ctor, |$a, $b| $body);
    };
}

/// A strict binary comparison: like `binop_prim!` but always wraps as
/// `Value::Bool`.
macro_rules! cmp_prim {
    ($name:ident, $as:path, |$a:ident, $b:ident| $body:expr) => {
        binop_prim!($name, ($as, $as), Bool, |$a, $b| $body);
    };
}

/// A strict unary primitive: pops one operand through `as` and wraps `body`
/// as `Value::$ctor`.
macro_rules! unop_prim {
    ($name:ident, $as:path, $ctor:ident, |$a:ident| $body:expr) => {
        fn $name(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
            let $a = $as(args.pop().unwrap())?;
            Ok(Value::$ctor($body))
        }
    };
}

/// A strict binary primitive with a fallible body: `body` is the function's
/// tail expression and must itself yield `Result<Value, EvalError>`, so it can
/// guard cases like division by zero.
macro_rules! binop_prim_try {
    ($name:ident, $as:path, |$a:ident, $b:ident| $body:expr) => {
        fn $name(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
            let $b = $as(args.pop().unwrap())?;
            let $a = $as(args.pop().unwrap())?;
            $body
        }
    };
}

// ---- text conversion ----------------------------------------------------------

/// Convert quoted inline text to boxes under `ctx` (the core of
/// `read-inline`): words become measured `InnerString`s, whitespace becomes
/// glue, embedded commands are applied to `ctx` and their arguments.
pub fn read_inline(
    interp: &mut Interp,
    ctx: &Context,
    elems: &[IText],
    env: &Env,
) -> Result<Vec<HorzBox>, EvalError> {
    let mut out = Vec::new();
    for elem in elems {
        match elem {
            IText::Text(text) => text_to_boxes(interp, ctx, text, &mut out)?,
            IText::Cmd { name, span, args } => {
                let cmd = env.lookup(name).ok_or_else(|| EvalError {
                    span: Some(*span),
                    msg: format!("unbound inline command '{name}' at run time"),
                })?;
                let mut v = interp.apply(cmd, Value::Context(Box::new(ctx.clone())))?;
                for arg in args {
                    let arg_v = interp.eval_arg(env, arg)?;
                    v = interp.apply(v, arg_v)?;
                }
                out.extend(as_inline_boxes(v)?);
            }
            IText::Embed { expr, span } => {
                let v = interp.eval_arg(env, expr)?;
                match v {
                    Value::InlineText {
                        elems: sub_elems,
                        env: cap_env,
                    } => {
                        out.extend(read_inline(interp, ctx, &sub_elems, &cap_env)?);
                    }
                    other => {
                        return Err(EvalError {
                            span: Some(*span),
                            msg: format!(
                                "expected inline-text in '#…;' embed, got {}",
                                other.type_name()
                            ),
                        });
                    }
                }
            }
            IText::EmbedMath { span, .. } => {
                return Err(EvalError {
                    span: Some(*span),
                    msg: "math typesetting is not implemented yet (phase 7)".into(),
                });
            }
        }
    }
    Ok(out)
}

/// Convert quoted block text to vertical boxes (the core of `read-block`).
pub fn read_block(
    interp: &mut Interp,
    ctx: &Context,
    elems: &[BText],
    env: &Env,
) -> Result<Vec<VertBox>, EvalError> {
    let mut out = Vec::new();
    for elem in elems {
        match elem {
            BText::Cmd { name, span, args } => {
                let cmd = env.lookup(name).ok_or_else(|| EvalError {
                    span: Some(*span),
                    msg: format!("unbound block command '{name}' at run time"),
                })?;
                let mut v = interp.apply(cmd, Value::Context(Box::new(ctx.clone())))?;
                for arg in args {
                    let arg_v = interp.eval_arg(env, arg)?;
                    v = interp.apply(v, arg_v)?;
                }
                out.extend(as_block_boxes(v)?);
            }
            BText::Embed { expr, span } => {
                let v = interp.eval_arg(env, expr)?;
                match v {
                    Value::BlockText {
                        elems: sub_elems,
                        env: cap_env,
                    } => {
                        out.extend(read_block(interp, ctx, &sub_elems, &cap_env)?);
                    }
                    other => {
                        return Err(EvalError {
                            span: Some(*span),
                            msg: format!(
                                "expected block-text in '#…;' embed, got {}",
                                other.type_name()
                            ),
                        });
                    }
                }
            }
        }
    }
    Ok(out)
}

fn text_to_boxes(
    interp: &mut Interp,
    ctx: &Context,
    text: &str,
    out: &mut Vec<HorzBox>,
) -> Result<(), EvalError> {
    let space_width = interp
        .metrics
        .advance(ctx.font, ' ', ctx.font_size)
        .unwrap_or(ctx.font_size * 0.33);
    let mut word = String::new();
    let flush_word = |word: &mut String, out: &mut Vec<HorzBox>| -> Result<(), EvalError> {
        if word.is_empty() {
            return Ok(());
        }
        let width = interp
            .metrics
            .text_width(ctx.font, word, ctx.font_size)
            .ok_or_else(|| EvalError {
                span: None,
                msg: format!(
                    "text '{word}' contains characters not available in the \
                     milestone-1 base fonts (WinAnsi only)"
                ),
            })?;
        out.push(HorzBox::Pure(PureHorzBox::InnerString {
            info: HorzStringInfo {
                font: ctx.font,
                size: ctx.font_size,
            },
            text: std::mem::take(word),
            width,
            height: interp.metrics.ascender(ctx.font, ctx.font_size),
            depth: interp.metrics.descender(ctx.font, ctx.font_size),
        }));
        Ok(())
    };
    for c in text.chars() {
        if c == ' ' || c == '\n' {
            flush_word(&mut word, out)?;
            // Avoid piling up doubled glue at text-run boundaries.
            if !matches!(
                out.last(),
                Some(HorzBox::Pure(PureHorzBox::OuterEmpty { .. }))
            ) {
                out.push(HorzBox::Pure(PureHorzBox::OuterEmpty {
                    natural: space_width,
                    shrinkable: space_width * 0.25,
                    stretchable: space_width * 0.5,
                }));
            }
        } else {
            word.push(c);
        }
    }
    flush_word(&mut word, out)
}

// ---- primitive bodies ----------------------------------------------------------

fn prim_read_inline(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let it = args.pop().unwrap();
    let ctx = as_context(args.pop().unwrap())?;
    let (elems, env) = as_inline_text(it)?;
    Ok(Value::InlineBoxes(read_inline(interp, &ctx, &elems, &env)?))
}

fn prim_read_block(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let bt = args.pop().unwrap();
    let ctx = as_context(args.pop().unwrap())?;
    let (elems, env) = as_block_text(bt)?;
    Ok(Value::BlockBoxes(read_block(interp, &ctx, &elems, &env)?))
}

/// `line-break : bool -> bool -> context -> inline-boxes -> block-boxes`
/// (vminst.ml `BackendLineBreaking`). The two leading bools tell the real
/// line breaker whether the paragraph's top/bottom edge may break across a
/// page; milestone-1's `break_into_lines` does not yet model breakability
/// at all, so both are accepted (to keep the arity/signature faithful to
/// v0.0.6) and ignored for now.
fn prim_line_break(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ib = as_inline_boxes(args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    let _is_breakable_bottom = as_bool(args.pop().unwrap())?;
    let _is_breakable_top = as_bool(args.pop().unwrap())?;
    Ok(Value::BlockBoxes(break_into_lines(&ctx, ib)))
}

fn prim_page_break(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let bb = as_block_boxes(args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    let geometry = PageGeometry::default();
    let pages = break_pages(&geometry, ctx.leading, bb);
    Ok(Value::Document(Rc::new(DocumentValue { geometry, pages })))
}

// ---- int arithmetic -------------------------------------------------------

binop_prim!(prim_int_add, as_int, Int, |a, b| a + b);
binop_prim!(prim_int_sub, as_int, Int, |a, b| a - b);
binop_prim!(prim_int_mul, as_int, Int, |a, b| a * b);

// OCaml catches `Division_by_zero` and reports `"division by zero"`; `mod`
// (see `Mod` in vminst.ml) shares that behavior.
binop_prim_try!(prim_int_div, as_int, |a, b| if b == 0 {
    eval_error("division by zero")
} else {
    Ok(Value::Int(a / b))
});
binop_prim_try!(prim_int_mod, as_int, |a, b| if b == 0 {
    eval_error("division by zero")
} else {
    Ok(Value::Int(a % b))
});

// ---- int comparisons -------------------------------------------------------

cmp_prim!(prim_int_eq, as_int, |a, b| a == b);
cmp_prim!(prim_int_ne, as_int, |a, b| a != b);
cmp_prim!(prim_int_lt, as_int, |a, b| a < b);
cmp_prim!(prim_int_gt, as_int, |a, b| a > b);
cmp_prim!(prim_int_le, as_int, |a, b| a <= b);
cmp_prim!(prim_int_ge, as_int, |a, b| a >= b);

// ---- bool -------------------------------------------------------------------

// Strict (both arguments already evaluated by the caller before these natives
// run): real SATySFi source-level `&&`/`||` short-circuit via elaboration into
// `if`, which is out of scope here.
binop_prim!(prim_bool_and, as_bool, Bool, |a, b| a && b);
binop_prim!(prim_bool_or, as_bool, Bool, |a, b| a || b);
unop_prim!(prim_bool_not, as_bool, Bool, |a| !a);

// ---- float --------------------------------------------------------------------

binop_prim!(prim_float_add, as_float, Float, |a, b| a + b);
binop_prim!(prim_float_sub, as_float, Float, |a, b| a - b);
binop_prim!(prim_float_mul, as_float, Float, |a, b| a * b);
binop_prim!(prim_float_div, as_float, Float, |a, b| a / b);
unop_prim!(prim_float_of_int, as_int, Float, |n| n as f64);

// `PrimitiveRound` in vminst.ml is, despite the name, `int_of_float`
// (truncation toward zero), not rounding to nearest.
unop_prim!(prim_round, as_float, Int, |x| x as i64);

// ---- length ---------------------------------------------------------------------

binop_prim!(prim_length_add, as_length, Length, |a, b| a + b);
binop_prim!(prim_length_sub, as_length, Length, |a, b| a - b);
binop_prim!(prim_length_scale, (as_length, as_float), Length, |a, b| a * b);
binop_prim!(prim_length_div, as_length, Float, |a, b| a / b);
cmp_prim!(prim_length_lt, as_length, |a, b| a < b);

// `LengthGreaterThan` in vminst.ml is implemented as `len2 <% len1`, i.e.
// `a >' b` iff `b <' a` — the same ordering, just flipped operands.
cmp_prim!(prim_length_gt, as_length, |a, b| b < a);

// ---- string -----------------------------------------------------------------------

binop_prim!(prim_string_concat, as_str, Str, |a, b| a + &b);
unop_prim!(prim_arabic, as_int, Str, |n| n.to_string());
cmp_prim!(prim_string_same, as_str, |a, b| a == b);

// ---- list -----------------------------------------------------------------

/// `x :: xs` — prepend `x` onto the list `xs`.
fn prim_list_cons(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let tail = args.pop().unwrap();
    let head = args.pop().unwrap();
    let mut list = match tail {
        Value::List(v) => v,
        other => return eval_error(format!("expected list, got {}", other.type_name())),
    };
    list.insert(0, head);
    Ok(Value::List(list))
}

// ---- mutable-cell dereference ----------------------------------------------

/// `!` — read the current contents of a mutable cell (see the `prims!`
/// registration above for how this differs structurally, not semantically,
/// from v0.0.6's `Dereference`/`Location` handling).
fn prim_deref(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let v = args.pop().unwrap();
    match v {
        Value::Ref(cell) => Ok(cell.borrow().clone()),
        other => eval_error(format!(
            "expected a mutable cell for '!', got {}",
            other.type_name()
        )),
    }
}

// ---- string, continued -----------------------------------------------------

// `string-length : string -> int` (vminst.ml `PrimitiveStringLength`) —
// counts Unicode scalar values (`BatUTF8.length`), not UTF-8 bytes.
unop_prim!(prim_string_length, as_str, Int, |s| s.chars().count() as i64);

/// `string-sub : string -> int -> int -> string` (vminst.ml
/// `PrimitiveStringSub`) — a substring addressed by Unicode-scalar-value
/// offset/width (`BatUTF8.sub`), not byte offset. Upstream raises a dynamic
/// error ("illegal index for string-sub") on an out-of-range index; we do
/// the same.
fn prim_string_sub(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let wid = as_int(args.pop().unwrap())?;
    let pos = as_int(args.pop().unwrap())?;
    let s = as_str(args.pop().unwrap())?;
    if wid < 0 || pos < 0 {
        return eval_error("illegal index for string-sub");
    }
    let chars: Vec<char> = s.chars().collect();
    let pos = pos as usize;
    let wid = wid as usize;
    match pos.checked_add(wid) {
        Some(end) if end <= chars.len() => Ok(Value::Str(chars[pos..end].iter().collect())),
        _ => eval_error("illegal index for string-sub"),
    }
}

// `string-explode : string -> int list` (vminst.ml `PrimitiveStringExplode`)
// — the string's Unicode scalar values (code points) in order, not its bytes.
unop_prim!(prim_string_explode, as_str, List, |s| s
    .chars()
    .map(|c| Value::Int(c as i64))
    .collect());

fn prim_embed_string(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let s = as_str(args.pop().unwrap())?;
    Ok(Value::InlineText {
        elems: Rc::new(vec![IText::Text(s)]),
        env: Env::root(),
    })
}

// ---- context ops ------------------------------------------------------------

/// `set-font-size : length -> context -> context` (vminst.ml
/// `PrimitiveSetFontSize`).
fn prim_set_font_size(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let size = as_length(args.pop().unwrap())?;
    Ok(Value::Context(Box::new(Context {
        font_size: size,
        ..ctx
    })))
}

/// `get-font-size : context -> length` (vminst.ml `PrimitiveGetFontSize`).
fn prim_get_font_size(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    Ok(Value::Length(ctx.font_size))
}

/// `set-leading : length -> context -> context` (vminst.ml
/// `PrimitiveSetLeading`; see the `prims!` table comment for why this is
/// the baseline-distance setter and not `set-min-gap-of-lines`).
fn prim_set_leading(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let leading = as_length(args.pop().unwrap())?;
    Ok(Value::Context(Box::new(Context { leading, ..ctx })))
}

/// `set-paragraph-margin : length -> length -> context -> context`
/// (vminst.ml `PrimitiveSetParagraphMargin`).
fn prim_set_paragraph_margin(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let bottom = as_length(args.pop().unwrap())?;
    let top = as_length(args.pop().unwrap())?;
    Ok(Value::Context(Box::new(Context {
        paragraph_top: top,
        paragraph_bottom: bottom,
        ..ctx
    })))
}

/// `get-text-width : context -> length` (vminst.ml `PrimitiveGetTextWidth`).
fn prim_get_text_width(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    Ok(Value::Length(ctx.paragraph_width))
}

/// `get-initial-context : length -> <second argument, ignored> -> context`
/// (vminst.ml `PrimitiveGetInitialContext`) — see the `prims!` table
/// comment: the second argument (v0.0.6's default math command) is
/// accepted but not used, since this port has no math typesetting yet. Its
/// *value* is simply discarded at runtime regardless of what
/// `prim_types.rs` declares its type to be (see that module's comment on
/// this same primitive for why the declared type was relaxed to `unit`).
fn prim_get_initial_context(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let _ignored = args.pop().unwrap();
    let width = as_length(args.pop().unwrap())?;
    Ok(Value::Context(Box::new(Context::initial(width))))
}

/// `set-font-key : int -> context -> context` — LOCAL, non-upstream
/// primitive; see the `prims!` table comment on `"set-font-key"` for why it
/// exists. Sets `Context::font` directly to `FontKey(n)`.
fn prim_set_font_key(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let key = as_int(args.pop().unwrap())?;
    if key < 0 || key > i64::from(u16::MAX) {
        return eval_error(format!("set-font-key: font key {key} is out of range"));
    }
    Ok(Value::Context(Box::new(Context {
        font: FontKey(key as u16),
        ..ctx
    })))
}

// ---- box combinators ---------------------------------------------------------

/// `++ : inline-boxes -> inline-boxes -> inline-boxes` (vminst.ml
/// `HorzConcat`).
fn prim_inline_concat(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let mut b = as_inline_boxes(args.pop().unwrap())?;
    let mut a = as_inline_boxes(args.pop().unwrap())?;
    a.append(&mut b);
    Ok(Value::InlineBoxes(a))
}

/// `+++ : block-boxes -> block-boxes -> block-boxes` (vminst.ml
/// `VertConcat`).
fn prim_block_concat(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let mut b = as_block_boxes(args.pop().unwrap())?;
    let mut a = as_block_boxes(args.pop().unwrap())?;
    a.append(&mut b);
    Ok(Value::BlockBoxes(a))
}

/// `inline-skip : length -> inline-boxes` (vminst.ml `BackendFixedEmpty`).
fn prim_inline_skip(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let width = as_length(args.pop().unwrap())?;
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::FixedEmpty { width },
    )]))
}

/// `inline-glue : length -> length -> length -> inline-boxes` (vminst.ml
/// `BackendOuterEmpty`; params `(widnat, widshrink, widstretch)`, i.e.
/// natural, then shrink, then stretch — the same order `OuterEmpty`'s
/// fields are already declared in).
fn prim_inline_glue(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let stretchable = as_length(args.pop().unwrap())?;
    let shrinkable = as_length(args.pop().unwrap())?;
    let natural = as_length(args.pop().unwrap())?;
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::OuterEmpty {
            natural,
            shrinkable,
            stretchable,
        },
    )]))
}

/// `block-skip : length -> block-boxes` (vminst.ml `BackendVertSkip`).
fn prim_block_skip(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let len = as_length(args.pop().unwrap())?;
    Ok(Value::BlockBoxes(vec![VertBox::Skip(len)]))
}

// ---- frontend-completion.md §Slice 1.A: the ~18 pure primitives -----------
//
// All bodies below are `float -> float` (or `float -> float -> float`)
// straight wraps of the matching `f64` method — vminst.ml's OCaml bodies
// (`make_float (sin flt1)`, etc.) are themselves direct wraps of the same
// IEEE-754 libm functions, so there is no behavioral daylight here.

binop_prim!(prim_atan2, as_float, Float, |a, b| a.atan2(b));
unop_prim!(prim_sin, as_float, Float, |x| x.sin());
unop_prim!(prim_asin, as_float, Float, |x| x.asin());
unop_prim!(prim_cos, as_float, Float, |x| x.cos());
unop_prim!(prim_acos, as_float, Float, |x| x.acos());
unop_prim!(prim_tan, as_float, Float, |x| x.tan());
unop_prim!(prim_atan, as_float, Float, |x| x.atan());
// vminst.ml:2834 `FloatLogarithm`: OCaml's `log` is the NATURAL logarithm
// (`ln`), not `log10`.
unop_prim!(prim_log, as_float, Float, |x| x.ln());
unop_prim!(prim_exp, as_float, Float, |x| x.exp());
// `ceil`/`floor` return `float`, unlike `round` (above), which returns
// `int` — see this file's `prims!` table comment on `"ceil"`/`"floor"`.
unop_prim!(prim_ceil, as_float, Float, |x| x.ceil());
unop_prim!(prim_floor, as_float, Float, |x| x.floor());

// `show-float : float -> string` (vminst.ml:2319 `PrimitiveShowFloat`) —
// OCaml's `string_of_float`. See `ocaml_show_float`'s doc comment (below)
// for the emulation and its known fidelity limits.
unop_prim!(prim_show_float, as_float, Str, |x| ocaml_show_float(x));

/// A from-scratch emulation of OCaml's `Stdlib.string_of_float`: format via
/// a C `%.12g` equivalent (12 significant digits; fixed-point when the
/// decimal exponent falls in `-4..12`, scientific otherwise; trailing
/// fractional zeros trimmed), then apply `valid_float_lexem`'s post-pass —
/// append a trailing `.` when the result would otherwise print as a bare
/// integer (`"1."`, never `"1"`, so a `float`'s printed form always reads
/// as a float, not an `int`). Known limitation: this is a Rust
/// reimplementation of the same specification (OCaml itself defers to the
/// platform C library's `%.12g`), so it may disagree with OCaml in obscure
/// corner cases, though it agrees on ordinary values (verified by hand
/// against real OCaml output for `0.`, `-0.`, `1.`, `100.`, `0.0025`,
/// `1e+20`, `1e-05`).
fn ocaml_show_float(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-infinity" } else { "infinity" }.to_string();
    }
    const PREC: i32 = 12;
    // Style-E rendering at precision PREC-1 recovers the correctly-rounded
    // decimal exponent (a naive `log10().floor()` can be off by one right
    // at a power of ten, because of binary/decimal rounding).
    let sci = format!("{:.*e}", (PREC - 1) as usize, x);
    let epos = sci.find('e').expect("scientific formatting always emits 'e'");
    let exp: i32 = sci[epos + 1..].parse().expect("well-formed exponent");
    let body = if exp < -4 || exp >= PREC {
        let mantissa = trim_trailing_fractional_zeros(&sci[..epos]);
        format!(
            "{mantissa}e{}{:02}",
            if exp < 0 { "-" } else { "+" },
            exp.abs()
        )
    } else {
        let decimals = (PREC - 1 - exp).max(0) as usize;
        trim_trailing_fractional_zeros(&format!("{:.*}", decimals, x)).to_string()
    };
    if body.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        format!("{body}.")
    } else {
        body
    }
}

/// Strip trailing zeros after a decimal point, then the point itself if
/// nothing remains after it (`"3.140" -> "3.14"`, `"5.000" -> "5"`);
/// already-integer-shaped strings (no `.`) pass through unchanged.
fn trim_trailing_fractional_zeros(s: &str) -> &str {
    if !s.contains('.') {
        return s;
    }
    s.trim_end_matches('0').trim_end_matches('.')
}

// `string-byte-length : string -> int` (vminst.ml:2159
// `PrimitiveStringByteLength`) — UTF-8 BYTE count (`String.length` in
// OCaml, whose native strings are raw byte sequences), unlike
// `string-length`'s Unicode-scalar-value count.
unop_prim!(prim_string_byte_length, as_str, Int, |s| s.len() as i64);

/// `string-sub-bytes : string -> int -> int -> string` (vminst.ml:2123
/// `PrimitiveStringSubBytes`) — byte-indexed substring (OCaml's
/// `String.sub`), unlike `string-sub`'s Unicode-scalar-value indexing.
/// Guards an out-of-range span exactly like `prim_string_sub`'s "illegal
/// index" dynamic error, AND a split landing inside a multi-byte UTF-8
/// sequence — impossible for OCaml's byte-oriented strings, but a
/// `Value::Str` here is a Rust `String`, which must stay valid UTF-8.
fn prim_string_sub_bytes(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let wid = as_int(args.pop().unwrap())?;
    let pos = as_int(args.pop().unwrap())?;
    let s = as_str(args.pop().unwrap())?;
    if wid < 0 || pos < 0 {
        return eval_error("illegal index for string-sub-bytes");
    }
    let (pos, wid) = (pos as usize, wid as usize);
    match pos.checked_add(wid) {
        Some(end) if end <= s.len() && s.is_char_boundary(pos) && s.is_char_boundary(end) => {
            Ok(Value::Str(s[pos..end].to_string()))
        }
        _ => eval_error("illegal index for string-sub-bytes"),
    }
}

/// `string-unexplode : int list -> string` (vminst.ml:2196
/// `PrimitiveStringUnexplode`) — the inverse of `string-explode` (above):
/// each int is a Unicode scalar value (code point), concatenated into one
/// UTF-8 string. Upstream's `Uchar.of_int` raises on an int that isn't a
/// valid Unicode scalar value (a surrogate, or out of range); reported here
/// as the same kind of dynamic error rather than panicking.
fn prim_string_unexplode(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let items = as_list(args.pop().unwrap())?;
    let mut s = String::new();
    for v in items {
        let n = as_int(v)?;
        match u32::try_from(n).ok().and_then(char::from_u32) {
            Some(c) => s.push(c),
            None => {
                return eval_error(format!(
                    "string-unexplode: {n} is not a valid Unicode scalar value"
                ))
            }
        }
    }
    Ok(Value::Str(s))
}

/// `display-message : string -> unit` (vminst.ml:2056
/// `PrimitiveDisplayMessage`) — upstream prints via `print_endline`
/// (STDOUT); this port deliberately prints to STDERR instead (`eprintln!`),
/// keeping stdout reserved for actual document output. This matches the
/// existing house convention: the CLI's own "output written" status line
/// (`satysfi-cli`'s `main.rs`) is likewise stderr-only, never stdout — a
/// documented deviation, not an oversight.
fn prim_display_message(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let msg = as_str(args.pop().unwrap())?;
    eprintln!("{msg}");
    Ok(Value::Unit)
}

/// `abort-with-message : string -> 'a` (vminst.ml:3133 `AbortWithMessage`)
/// — raises a dynamic error carrying `msg` verbatim. The polymorphic result
/// type (`prim_types.rs`'s `poly1`) is vacuously satisfiable: this always
/// evaluates to `Err`, never actually producing a value of whatever type
/// the call site expected.
fn prim_abort_with_message(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let msg = as_str(args.pop().unwrap())?;
    eval_error(msg)
}
