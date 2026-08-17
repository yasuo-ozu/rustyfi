//! Type signatures for every primitive registered in `primitives.rs`'s
//! `prims!` table (plus the `inline-fil` constant), transcribed from
//! v0.0.6's `tools/gencode/vminst.ml` `~type_:` fields (cited by line
//! number at each entry below) and from `src/frontend/primitives.cppo.ml`
//! for the handful of names vminst.ml doesn't define directly (`::`, `!`,
//! the comparison trio derived in `general_table`).
//!
//! Milestone 1 used to hardcode local signatures here for `document`, `+p`,
//! and `\emph`. Phase 4 deleted them along with their `primitives.rs`
//! bodies: the real definitions now live in the `stdja-mini` stdlib package
//! (`lib-satysfi/dist/packages/stdja-mini.satyh`), built entirely out of
//! *other* primitives below and typechecked for real, like any other
//! `.satyh` library. Our simplified `page-break` still has no vminst.ml
//! entry at all — it keeps its own local signature, documented at its entry
//! below.
//!
//! Also provides [`builtin_variants`], the seed set of variant type
//! declarations ([`VariantDecl`]) primitives.cppo.ml registers before any
//! user code runs.

use crate::types::{self, BaseType, CmdArgType, MonoType, PolyType, TyVarRef};
use std::collections::HashMap;

// ============================================================================
// Constructor helpers — read the table below like vminst.ml's own `tI`,
// `tB`, `@->`, etc.
// ============================================================================

pub fn t_unit() -> MonoType {
    MonoType::Base(BaseType::Unit)
}
pub fn t_bool() -> MonoType {
    MonoType::Base(BaseType::Bool)
}
pub fn t_int() -> MonoType {
    MonoType::Base(BaseType::Int)
}
pub fn t_float() -> MonoType {
    MonoType::Base(BaseType::Float)
}
pub fn t_length() -> MonoType {
    MonoType::Base(BaseType::Length)
}
pub fn t_string() -> MonoType {
    MonoType::Base(BaseType::String)
}
pub fn t_inline_text() -> MonoType {
    MonoType::Base(BaseType::InlineText)
}
pub fn t_block_text() -> MonoType {
    MonoType::Base(BaseType::BlockText)
}
/// `math` (vminst.ml's `tMATH`).
pub fn t_math_text() -> MonoType {
    MonoType::Base(BaseType::MathText)
}
pub fn t_inline_boxes() -> MonoType {
    MonoType::Base(BaseType::InlineBoxes)
}
pub fn t_block_boxes() -> MonoType {
    MonoType::Base(BaseType::BlockBoxes)
}
pub fn t_context() -> MonoType {
    MonoType::Base(BaseType::Context)
}
pub fn t_document() -> MonoType {
    MonoType::Base(BaseType::Document)
}
/// `color` (`primitives.cppo.ml:187-190`'s `Gray of float | RGB of
/// (float*float*float) | CMYK of (float*float*float*float)`) — a built-in
/// **variant**, not a `BaseType`: it costs a `VariantDecl` (registered by
/// [`builtin_variants`] below), no base type, no `Value` change, no backend
/// (mirrors `t_document`'s shape, but through `MonoType::Variant` since
/// there is no `BaseType::Color`). `Gray`/`RGB`/`CMYK` typecheck and
/// evaluate as ordinary `Ast::Ctor`/`Value::Ctor` values; not yet
/// *consumable* (no `set-text-color` — needs a `Context.text_color` field,
/// deferred to a later Roadmap item), but sufficient for `color.satyh` to
/// compile.
pub fn t_color() -> MonoType {
    MonoType::Variant("color".to_string(), Vec::new())
}

/// `dom -> cod` (vminst.ml's `@->`).
pub fn arrow(dom: MonoType, cod: MonoType) -> MonoType {
    MonoType::Func(Box::new(dom), Box::new(cod))
}

/// Right-folds [`arrow`] over `doms`, ending in `cod` — for chaining
/// several arguments the way vminst.ml chains `@->`, e.g.
/// `arrows(vec![t_bool(), t_bool(), t_context(), t_inline_boxes()], t_block_boxes())`
/// for `bool -> bool -> context -> inline-boxes -> block-boxes`.
pub fn arrows(doms: Vec<MonoType>, cod: MonoType) -> MonoType {
    doms.into_iter().rev().fold(cod, |acc, dom| arrow(dom, acc))
}

/// `tL` — list type.
pub fn list(t: MonoType) -> MonoType {
    MonoType::List(Box::new(t))
}

/// `tR` — mutable reference type.
pub fn reff(t: MonoType) -> MonoType {
    MonoType::Ref(Box::new(t))
}

/// `tPROD` — tuple type.
pub fn product(ts: Vec<MonoType>) -> MonoType {
    MonoType::Product(ts)
}

/// `[...] inline-cmd` (vminst.ml's `tICMD ty` = `HorzCommandType([Mandatory
/// ty])`: an inline command taking exactly one mandatory argument of type
/// `ty`).
pub fn inline_cmd(args: Vec<CmdArgType>) -> MonoType {
    MonoType::InlineCmd(args)
}

/// `mandatory` command-argument entry.
pub fn mandatory(ty: MonoType) -> CmdArgType {
    CmdArgType { optional: false, ty }
}

/// `optional` (`?`) command-argument entry.
pub fn optional(ty: MonoType) -> CmdArgType {
    CmdArgType { optional: true, ty }
}

/// A type scheme with no quantified variables (vminst.ml's `~%` wraps a
/// closed monomorphic body the same way; primitives that actually need
/// polymorphism, like `::`/`!`, use [`poly1`] instead).
pub fn poly0(ty: MonoType) -> PolyType {
    PolyType::mono(ty)
}

/// A type scheme quantified over exactly one fresh type variable, e.g.
/// `poly1(|a| arrow(reff(a.clone()), a))` for `!`'s `'a ref -> 'a`
/// (vminst.ml's `~@` marks such a per-scheme fresh variable; `~%` then
/// closes the whole thing into a scheme, matching `ptyderef`/`ptycons` in
/// `primitives.cppo.ml:546-547`).
pub fn poly1<F: FnOnce(MonoType) -> MonoType>(f: F) -> PolyType {
    let v = types::new_ty_var(0);
    let body = f(MonoType::Var(v.clone()));
    PolyType::from_vars(vec![v], Vec::new(), body)
}

// ============================================================================
// The primitive type table.
// ============================================================================

/// Look up the type scheme of a primitive registered in
/// `primitives.rs`'s `prims!` table (or the separately-defined
/// `inline-fil` constant), by its *source* name (sigil included, e.g.
/// `"\\emph"`, `"+'"`, `"::"`).
pub fn primitive_type(name: &str) -> Option<PolyType> {
    Some(match name {
        // ---- milestone-1 natives (no vminst.ml entry — local signatures) ----
        //
        // `read-inline : context -> inline-text -> inline-boxes`
        // (vminst.ml:834, `HorzLex`: `~% (tCTX @-> tIT @-> tIB)`).
        "read-inline" => poly0(arrow(t_context(), arrow(t_inline_text(), t_inline_boxes()))),
        // `read-block : context -> block-text -> block-boxes`
        // (vminst.ml:857, `VertLex`: `~% (tCTX @-> tBT @-> tBB)`).
        "read-block" => poly0(arrow(t_context(), arrow(t_block_text(), t_block_boxes()))),
        // `line-break : bool -> bool -> context -> inline-boxes -> block-boxes`
        // (vminst.ml:1003, `BackendLineBreaking`:
        // `~% (tB @-> tB @-> tCTX @-> tIB @-> tBB)`).
        "line-break" => poly0(arrows(
            vec![t_bool(), t_bool(), t_context(), t_inline_boxes()],
            t_block_boxes(),
        )),
        // `page-break : context -> block-boxes -> document` — a LOCAL,
        // simplified signature. Upstream's `BackendPageBreaking`
        // (vminst.ml:1024) is `page -> pagecontf -> pagepartsf -> block-
        // boxes -> document` (a page size plus two per-page-number
        // continuation callbacks for headers/footers); this milestone's
        // `prim_page_break` (primitives.rs) always uses
        // `PageGeometry::default()` and has no header/footer callbacks, so
        // it's typed as exactly what it does at runtime.
        "page-break" => poly0(arrow(t_context(), arrow(t_block_boxes(), t_document()))),
        // `document`, `+p`, and `\emph` used to have LOCAL signatures here
        // (milestone-1 natives); phase 4 deleted them along with their
        // `primitives.rs` bodies — see this module's doc comment. They are
        // now ordinary `stdja-mini` bindings (a plain `let`, a
        // `let-block`, and a `let-inline` respectively), typechecked by the
        // normal rules from the primitives below, with no bespoke entry
        // needed in this table at all.

        // ---- int arithmetic ----
        // vminst.ml:2537 `Plus`: `~% (tI @-> tI @-> tI)`.
        "+" => poly0(arrows(vec![t_int(), t_int()], t_int())),
        // vminst.ml:2553 `Minus`.
        "-" => poly0(arrows(vec![t_int(), t_int()], t_int())),
        // vminst.ml:2487 `Times`.
        "*" => poly0(arrows(vec![t_int(), t_int()], t_int())),
        // vminst.ml:2503 `Divides`.
        "/" => poly0(arrows(vec![t_int(), t_int()], t_int())),
        // vminst.ml:2520 `Mod`.
        "mod" => poly0(arrows(vec![t_int(), t_int()], t_int())),

        // ---- int comparisons ----
        // vminst.ml:2569 `EqualTo`: `~% (tI @-> tI @-> tB)`.
        "==" => poly0(arrows(vec![t_int(), t_int()], t_bool())),
        // LOCAL (`<>` is `LogicalNot (EqualTo ..)` in primitives.cppo.ml's
        // `general_table`, not its own vminst.ml instruction) — same type
        // as `==`.
        "<>" => poly0(arrows(vec![t_int(), t_int()], t_bool())),
        // vminst.ml:2601 `LessThan`.
        "<" => poly0(arrows(vec![t_int(), t_int()], t_bool())),
        // vminst.ml:2585 `GreaterThan`.
        ">" => poly0(arrows(vec![t_int(), t_int()], t_bool())),
        // LOCAL (`LogicalNot (GreaterThan ..)`) — same type as `>`/`<`.
        "<=" => poly0(arrows(vec![t_int(), t_int()], t_bool())),
        // LOCAL (`LogicalNot (LessThan ..)`).
        ">=" => poly0(arrows(vec![t_int(), t_int()], t_bool())),

        // ---- bool ----
        // vminst.ml:2617 `LogicalAnd`: `~% (tB @-> tB @-> tB)`.
        "&&" => poly0(arrows(vec![t_bool(), t_bool()], t_bool())),
        // vminst.ml:2633 `LogicalOr`.
        "||" => poly0(arrows(vec![t_bool(), t_bool()], t_bool())),
        // vminst.ml:2649 `LogicalNot`: `~% (tB @-> tB)`.
        "not" => poly0(arrow(t_bool(), t_bool())),

        // ---- float ----
        // vminst.ml:2664 `FloatPlus`: `~% (tFL @-> tFL @-> tFL)`.
        "+." => poly0(arrows(vec![t_float(), t_float()], t_float())),
        // vminst.ml:2680 `FloatMinus`.
        "-." => poly0(arrows(vec![t_float(), t_float()], t_float())),
        // vminst.ml:2696 `FloatTimes`.
        "*." => poly0(arrows(vec![t_float(), t_float()], t_float())),
        // vminst.ml:2712 `FloatDivides`.
        "/." => poly0(arrows(vec![t_float(), t_float()], t_float())),
        // vminst.ml:2333 `PrimitiveFloat`: `~% (tI @-> tFL)`.
        "float" => poly0(arrow(t_int(), t_float())),
        // vminst.ml:2348 `PrimitiveRound`: `~% (tFL @-> tI)` — despite the
        // name this truncates toward zero (see primitives.rs's
        // `prim_round`), but the *type* is unaffected by that.
        "round" => poly0(arrow(t_float(), t_int())),

        // ---- length ----
        // vminst.ml:2894 `LengthPlus`: `~% (tLN @-> tLN @-> tLN)`.
        "+'" => poly0(arrows(vec![t_length(), t_length()], t_length())),
        // vminst.ml:2910 `LengthMinus`.
        "-'" => poly0(arrows(vec![t_length(), t_length()], t_length())),
        // vminst.ml:2926 `LengthTimes`: `~% (tLN @-> tFL @-> tLN)` — note
        // the second argument is `float`, not `length`.
        "*'" => poly0(arrows(vec![t_length(), t_float()], t_length())),
        // vminst.ml:2942 `LengthDivides`: `~% (tLN @-> tLN @-> tFL)` —
        // note the result is `float`, not `length`.
        "/'" => poly0(arrows(vec![t_length(), t_length()], t_float())),
        // vminst.ml:2958 `LengthLessThan`.
        "<'" => poly0(arrows(vec![t_length(), t_length()], t_bool())),
        // vminst.ml:2974 `LengthGreaterThan`.
        ">'" => poly0(arrows(vec![t_length(), t_length()], t_bool())),

        // ---- string ----
        // vminst.ml:22 `Concat`: `~% (tS @-> tS @-> tS)`.
        "^" => poly0(arrows(vec![t_string(), t_string()], t_string())),
        // vminst.ml:2303 `PrimitiveArabic`: `~% (tI @-> tS)`.
        "arabic" => poly0(arrow(t_int(), t_string())),
        // vminst.ml:2085 `PrimitiveSame`: `~% (tS @-> tS @-> tB)`.
        "string-same" => poly0(arrows(vec![t_string(), t_string()], t_bool())),
        // vminst.ml:2143 `PrimitiveStringLength`: `~% (tS @-> tI)`.
        "string-length" => poly0(arrow(t_string(), t_int())),
        // vminst.ml:2101 `PrimitiveStringSub`: `~% (tS @-> tI @-> tI @-> tS)`.
        "string-sub" => poly0(arrows(vec![t_string(), t_int(), t_int()], t_string())),
        // vminst.ml:2212 `PrimitiveStringExplode`: `~% (tS @-> (tL tI))`.
        "string-explode" => poly0(arrow(t_string(), list(t_int()))),

        // ---- list cons ----
        // primitives.cppo.ml:547: `ptycons = ~% ((~@ tv2) @-> (tL (~@ tv2))
        // @-> (tL (~@ tv2)))`, i.e. `'a -> 'a list -> 'a list`. Not its own
        // vminst.ml instruction upstream (`::` desugars to `ListCons` at
        // parse time there); here it's a first-class primitive (see the
        // `prims!` table's comment on `"::"` in primitives.rs).
        "::" => poly1(|a| arrow(a.clone(), arrow(list(a.clone()), list(a)))),

        // ---- mutable-cell dereference ----
        // primitives.cppo.ml:546: `ptyderef = ~% ((tR (~@ tv1)) @-> (~@
        // tv1))`, i.e. `'a ref -> 'a`.
        "!" => poly1(|a| arrow(reff(a.clone()), a)),

        // ---- text embedding ----
        // vminst.ml:1706 `PrimitiveEmbed`: `~% (tS @-> tIT)`.
        "embed-string" => poly0(arrow(t_string(), t_inline_text())),

        // ---- context ops (phase 4, part 1) ----
        // vminst.ml:1434 `PrimitiveSetFontSize`: `~% (tLN @-> tCTX @-> tCTX)`.
        "set-font-size" => poly0(arrow(t_length(), arrow(t_context(), t_context()))),
        // vminst.ml:1449 `PrimitiveGetFontSize`: `~% (tCTX @-> tLN)`.
        "get-font-size" => poly0(arrow(t_context(), t_length())),
        // vminst.ml:1633 `PrimitiveSetLeading`: `~% (tLN @-> tCTX @-> tCTX)`
        // (see primitives.rs's `prims!` table comment on why this, and not
        // `set-min-gap-of-lines`, is the baseline-distance setter).
        "set-leading" => poly0(arrow(t_length(), arrow(t_context(), t_context()))),
        // vminst.ml:1396 `PrimitiveSetParagraphMargin`:
        // `~% (tLN @-> tLN @-> tCTX @-> tCTX)`.
        "set-paragraph-margin" => poly0(arrows(
            vec![t_length(), t_length(), t_context()],
            t_context(),
        )),
        // vminst.ml:1648 `PrimitiveGetTextWidth`: `~% (tCTX @-> tLN)`.
        "get-text-width" => poly0(arrow(t_context(), t_length())),
        // vminst.ml:1247 `PrimitiveGetInitialContext`:
        // `~% (tLN @-> tICMD tMATH @-> tCTX)`. Faithfully, `tICMD tMATH`
        // would be `HorzCommandType` (our `MonoType::InlineCmd`) with
        // exactly one mandatory `math` (`BaseType::MathText`) argument — NOT
        // `MathCmd` (that former is `MathCommandType`, a *different*
        // v0.0.6 type used for math-mode commands like `\sqrt`, unrelated
        // to this argument).
        //
        // DEVIATION (phase 4): that faithful second-argument type is a REAL
        // dead end for the `stdja-mini` stdlib package (dist/packages/
        // stdja-mini.satyh), which now calls this primitive directly from
        // ordinary `.satyh` source to build `document`'s initial context.
        // This port's surface grammar has no way to *construct* a value of
        // type `[math] inline-cmd` in expression position at all: command
        // names (`\cmd`) only ever appear as `IText::Cmd`/`BText::Cmd`/
        // `MathElem::Cmd` heads inside quoted text (see `cst.rs`'s
        // `Atomic`), never as a plain referenceable value, and there is no
        // `let-math`-style top-level form to define a math-mode command
        // binding either — so nothing in-language can ever type-check as
        // this argument. Since math typesetting doesn't exist yet in this
        // port (the runtime, `prim_get_initial_context` in primitives.rs,
        // already ignores the value completely), the second argument's
        // *declared* type is relaxed here from `[math] inline-cmd` to
        // `unit`: the simplest type an in-language value can actually
        // inhabit (the literal `()`), keeping the primitive's arity/shape
        // faithful to v0.0.6 while making it callable. `stdja-mini.satyh`
        // passes `()` for this argument.
        "get-initial-context" => poly0(arrow(t_length(), arrow(t_unit(), t_context()))),

        // ---- context ops, continued (phase 4, part 2 — a LOCAL,
        // non-upstream primitive; see primitives.rs's `prims!` table
        // comment on `"set-font-key"` for why it exists) --------------------
        "set-font-key" => poly0(arrow(t_int(), arrow(t_context(), t_context()))),

        // ---- box combinators ----
        // vminst.ml:803 `HorzConcat`: `~% (tIB @-> tIB @-> tIB)`.
        "++" => poly0(arrows(vec![t_inline_boxes(), t_inline_boxes()], t_inline_boxes())),
        // vminst.ml:818 `VertConcat`: `~% (tBB @-> tBB @-> tBB)`.
        "+++" => poly0(arrows(vec![t_block_boxes(), t_block_boxes()], t_block_boxes())),
        // No vminst.ml entry — see `base_env`'s comment on `inline-nil`/
        // `block-nil` in primitives.rs (the empty-boxes value that v0.0.6
        // gets for free from `{}`/`<>` literal syntax).
        "inline-nil" => poly0(t_inline_boxes()),
        "block-nil" => poly0(t_block_boxes()),
        // vminst.ml:1757 `BackendFixedEmpty`: `~% (tLN @-> tIB)`.
        "inline-skip" => poly0(arrow(t_length(), t_inline_boxes())),
        // vminst.ml:1771 `BackendOuterEmpty`:
        // `~% (tLN @-> tLN @-> tLN @-> tIB)`.
        "inline-glue" => poly0(arrows(
            vec![t_length(), t_length(), t_length()],
            t_inline_boxes(),
        )),
        // vminst.ml:1171 `BackendVertSkip`: `~% (tLN @-> tBB)`.
        "block-skip" => poly0(arrow(t_length(), t_block_boxes())),

        // ---- inline-fil ----
        // Not a primitive *function* at all (`base_env` binds it directly
        // to a constant `Value::InlineBoxes`, primitives.rs), so there is
        // no vminst.ml `~type_:` to cite; its type is simply that of the
        // value it names.
        "inline-fil" => poly0(t_inline_boxes()),

        // ---- frontend-completion.md §Slice 1.A: the ~18 pure primitives ----
        // (`|>` is excluded here on purpose — it is elaborated directly to
        // ordinary `Apply`, never a `scope`/env-bound name, so it has no
        // primitive type scheme at all; see `elaborate.rs`'s `climb`.)

        // vminst.ml:2729/2744/2759/2774/2789/2804 `FloatSine`/`FloatArcSine`/
        // `FloatCosine`/`FloatArcCosine`/`FloatTangent`/`FloatArcTangent`:
        // all `~% (tFL @-> tFL)`.
        "sin" => poly0(arrow(t_float(), t_float())),
        "asin" => poly0(arrow(t_float(), t_float())),
        "cos" => poly0(arrow(t_float(), t_float())),
        "acos" => poly0(arrow(t_float(), t_float())),
        "tan" => poly0(arrow(t_float(), t_float())),
        "atan" => poly0(arrow(t_float(), t_float())),
        // vminst.ml:2819 `FloatArcTangent2`: `~% (tFL @-> tFL @-> tFL)`,
        // params `(flt1, flt2)` in that order, so `flt1.atan2(flt2)`.
        "atan2" => poly0(arrows(vec![t_float(), t_float()], t_float())),
        // vminst.ml:2835 `FloatLogarithm`: natural log, not log10.
        "log" => poly0(arrow(t_float(), t_float())),
        // vminst.ml:2850 `FloatExponential`.
        "exp" => poly0(arrow(t_float(), t_float())),
        // vminst.ml:2865/2880 `PrimitiveCeil`/`PrimitiveFloor`: both
        // `~% (tFL @-> tFL)` — the RESULT is `float`, not `int` (contrast
        // `round`, above).
        "ceil" => poly0(arrow(t_float(), t_float())),
        "floor" => poly0(arrow(t_float(), t_float())),
        // vminst.ml:2319 `PrimitiveShowFloat`: `~% (tFL @-> tS)`.
        "show-float" => poly0(arrow(t_float(), t_string())),

        // vminst.ml:2159 `PrimitiveStringByteLength`: `~% (tS @-> tI)`.
        "string-byte-length" => poly0(arrow(t_string(), t_int())),
        // vminst.ml:2123 `PrimitiveStringSubBytes`:
        // `~% (tS @-> tI @-> tI @-> tS)`.
        "string-sub-bytes" => poly0(arrows(vec![t_string(), t_int(), t_int()], t_string())),
        // vminst.ml:2196 `PrimitiveStringUnexplode`: `~% ((tL tI) @-> tS)`.
        "string-unexplode" => poly0(arrow(list(t_int()), t_string())),

        // vminst.ml:2056 `PrimitiveDisplayMessage`: `~% (tS @-> tU)`.
        "display-message" => poly0(arrow(t_string(), t_unit())),
        // vminst.ml:3133 `AbortWithMessage`: `~% (tS @-> (~@ tv))` — a
        // fresh-per-instantiation type variable (see `!`/`::`'s `poly1`
        // above for the same pattern).
        "abort-with-message" => poly1(|a| arrow(t_string(), a)),

        _ => return None,
    })
}

// ============================================================================
// Variant type declarations (primitives.cppo.ml's `~% ( ... )` registration
// block, lines ~150-170).
// ============================================================================

/// A user (or built-in) variant type declaration.
///
/// `param_vars` names the declaration's type parameters as concrete
/// placeholder [`TyVarRef`]s that appear (via `MonoType::Var`) inside
/// `ctors`' payload types; they are never meant to be unified against
/// anything directly. [`VariantDecl::instantiate_ctor`] is the only
/// sanctioned way to use a declaration: given concrete argument types for
/// the variant's `params` type parameters, it substitutes them for the
/// placeholders (matched by pointer identity, the same mechanism
/// `types::instantiate` uses for quantified variables — see
/// `types::substitute`) throughout the chosen constructor's payload type,
/// and returns both that payload type and the resulting
/// `MonoType::Variant(name, args)`.
///
/// This mirrors v0.0.6's `Typeenv.Raw.register_type`/`add_constructor`
/// (`primitives.cppo.ml:154-217`), which pairs a `TypeID.t` with a
/// `Typeenv.Data(arity)` and each constructor with a `Poly(...)` scheme
/// quantified over the same `bid`/`typaram` placeholders declared once for
/// the whole type — the same shape, just spelled with this port's
/// `TyVarRef`/`substitute` machinery instead of v0.0.6's `BoundID`/
/// `PolyBound`.
#[derive(Clone, Debug)]
pub struct VariantDecl {
    pub name: String,
    pub params: usize,
    pub ctors: Vec<(String, Option<MonoType>)>,
    pub param_vars: Vec<TyVarRef>,
}

impl VariantDecl {
    /// Instantiate `ctor` at a fresh application `Name(args[0], args[1],
    /// ...)`. Returns `None` if `ctor` isn't one of this declaration's
    /// constructors or `args.len() != self.params`. On success, returns
    /// `(payload_type, result_type)`, where `payload_type` is `None` for a
    /// nullary constructor (like `None`).
    pub fn instantiate_ctor(&self, ctor: &str, args: &[MonoType]) -> Option<(Option<MonoType>, MonoType)> {
        if args.len() != self.params {
            return None;
        }
        let (_, payload_tpl) = self.ctors.iter().find(|(n, _)| n == ctor)?;
        let mut var_map: HashMap<usize, MonoType> = HashMap::new();
        for (pv, arg) in self.param_vars.iter().zip(args.iter()) {
            var_map.insert(types::ptr_key(pv), arg.clone());
        }
        let row_map = HashMap::new();
        let payload = payload_tpl
            .as_ref()
            .map(|t| types::substitute(t, &var_map, &row_map));
        Some((payload, MonoType::Variant(self.name.clone(), args.to_vec())))
    }
}

/// The variant declarations `crate::eval`'s pattern matching and (later)
/// the inferencer need before any user `.saty` source runs, transcribed
/// from `primitives.cppo.ml:154-159`:
///
/// ```ocaml
/// |> Typeenv.Raw.register_type "option" tyid_option (Typeenv.Data(1))
/// |> Typeenv.Raw.add_constructor "None" ([bid], Poly(tU)) tyid_option
/// |> Typeenv.Raw.add_constructor "Some" ([bid], Poly(typaram)) tyid_option
/// |> Typeenv.Raw.register_type "itemize" tyid_itemize (Typeenv.Data(0))
/// |> Typeenv.Raw.add_constructor "Item" ([], Poly(tPROD [tIT; tL (tITMZ ())])) tyid_itemize
/// ```
///
/// Note v0.0.6 gives *every* constructor a payload type, using `tU` (unit)
/// for `None`'s "no real payload" case; this port's `Ast::Ctor`/
/// `Pattern::Ctor` (ast.rs) instead represent a nullary constructor as
/// `None` (the Rust `Option`, not the SATySFi one!) directly, so `None`'s
/// declared payload here is `Option::None`, not `Some(unit)`.
pub fn builtin_variants() -> Vec<VariantDecl> {
    let option_param = types::new_ty_var(0);
    let option_decl = VariantDecl {
        name: "option".to_string(),
        params: 1,
        ctors: vec![
            ("None".to_string(), None),
            ("Some".to_string(), Some(MonoType::Var(option_param.clone()))),
        ],
        param_vars: vec![option_param],
    };

    let itemize_decl = VariantDecl {
        name: "itemize".to_string(),
        params: 0,
        ctors: vec![(
            "Item".to_string(),
            Some(product(vec![
                t_inline_text(),
                list(MonoType::Variant("itemize".to_string(), Vec::new())),
            ])),
        )],
        param_vars: Vec::new(),
    };

    // `color` (frontend-completion.md §Slice1-B) — nullary variant (no type
    // parameters, so `param_vars` is empty, same as `itemize` above):
    // `Gray of float | RGB of (float*float*float) | CMYK of
    // (float*float*float*float)` (`primitives.cppo.ml:187-190`). Unblocks
    // **[stdlib]** `color.satyh`'s `Color.rgb`/`Color.gray`/`Color.cmyk`
    // constructor wrappers.
    let color_decl = VariantDecl {
        name: "color".to_string(),
        params: 0,
        ctors: vec![
            ("Gray".to_string(), Some(t_float())),
            ("RGB".to_string(), Some(product(vec![t_float(), t_float(), t_float()]))),
            (
                "CMYK".to_string(),
                Some(product(vec![t_float(), t_float(), t_float(), t_float()])),
            ),
        ],
        param_vars: Vec::new(),
    };

    vec![option_decl, itemize_decl, color_decl]
}
