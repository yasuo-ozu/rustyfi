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
/// `math` — alias of [`t_math_text`], named to match `docs/plans/math-
/// engine.md`'s own `t_math()` naming (both name the same
/// `BaseType::MathText`; kept as a thin alias rather than renaming the
/// original, which `IText`/`Ast::MathText`'s existing call sites already use).
pub fn t_math() -> MonoType {
    t_math_text()
}
/// `math-class` (`docs/plans/math-engine.md` §A item 2;
/// `primitives.cppo.ml:162-170`'s `MathOrd | MathBin | MathRel | MathOp |
/// MathPunct | MathOpen | MathClose | MathPrefix | MathInner`) — a built-in
/// **variant** (same shape as `t_color()`/`t_script()` above), registered by
/// [`builtin_variants`]. `math-char`/`math-group`/… all take this as an
/// argument. **Distinct** from `t_math_char_class()` below (phase F's
/// `MathItalic`/… styling variant) — do not conflate the two.
pub fn t_math_class() -> MonoType {
    MonoType::Variant("math-class".to_string(), Vec::new())
}
/// `math-char-class` (`docs/plans/math-engine.md` §F; `horzBox.ml:147`'s
/// `MathItalic | MathBoldItalic | MathRoman | MathBoldRoman | MathScript |
/// MathBoldScript | MathFraktur | MathBoldFraktur | MathDoubleStruck`) — a
/// built-in variant, registered by [`builtin_variants`]. Needed for
/// `math.satyh`'s `sig` (`\math-style : [math-char-class; math] math-cmd`)
/// and its `\mathrm`/`\mathbf`/`\mathcal`/… definitions to type-check —
/// gap 5 (`docs/plans/math-mode-language-gaps.md`) resolves the actual
/// Unicode-math-block restyling this variant names (`satysfi_backend::
/// MathCharClass`/`resolve_variant_char`) once evaluation reaches a value
/// of this type.
pub fn t_math_char_class() -> MonoType {
    MonoType::Variant("math-char-class".to_string(), Vec::new())
}
/// `paren` (`pervasives.satyh`'s `type paren = length -> length -> length ->
/// length -> color -> inline-boxes * (length -> length)`) — structural, like
/// `t_point()`/`t_dash()`/`t_deco()` above: directly the expanded function
/// type, not a nominal reference. `math-paren`'s first two arguments
/// (`math.satyh`'s `paren-left`/`paren-right`, `\paren`/`\brace`/`\abs`/…)
/// are typed against this shape directly; this port's type-synonym
/// expansion resolves a `paren`-named annotation (e.g. `val paren-left :
/// paren` in a `sig`) to the same shape, but — see `typecheck.rs`'s
/// `lower_sig_item` doc comment — no sig-enforcement pass consults that yet,
/// so what actually matters is that this matches `paren-left`/`paren-right`'s
/// own INFERRED type, which it does by construction.
pub fn t_paren() -> MonoType {
    arrows(
        vec![t_length(), t_length(), t_length(), t_length(), t_color()],
        product(vec![t_inline_boxes(), arrow(t_length(), t_length())]),
    )
}
/// A math-char kern function (`math.satyh`'s `\int`: `let kernfR fontsize
/// ypos = fontsize *' 0.45 in ...`) — `length -> length -> length`
/// (fontsize, y-position -> kern amount). `math-char-with-kern`/
/// `math-big-char-with-kern`'s 3rd/4th arguments.
pub fn t_math_kern_func() -> MonoType {
    arrows(vec![t_length(), t_length()], t_length())
}
/// `math-variant-char`'s 9-field per-style codepoint record
/// (`docs/plans/math-engine.md` §F; `value.rs`'s `MathVariantStyle`) — a
/// closed record row, structural like `t_pbinfo()`/`t_page_content_scheme()`
/// above. Field order doesn't matter (records are structural), only the
/// label set; matches `math.satyh`'s `greek-lowercase`/`greek-uppercase`
/// record literals field-for-field.
pub fn t_math_variant_style() -> MonoType {
    const LABELS: [&str; 9] = [
        "italic",
        "bold-italic",
        "roman",
        "bold-roman",
        "script",
        "bold-script",
        "fraktur",
        "bold-fraktur",
        "double-struck",
    ];
    let mut row = types::Row::Empty;
    for label in LABELS.iter().rev() {
        row = types::Row::Cons(label.to_string(), Box::new(t_string()), Box::new(row));
    }
    MonoType::Record(row)
}
/// `image` (vminst.ml's `tIMG`) — `load-image`'s result;
/// `docs/plans/math-images.md` §Slice 1.
pub fn t_image() -> MonoType {
    MonoType::Base(BaseType::Image)
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
/// compile. `fill`/`stroke` (graphics Slice 1) are its first consumers.
pub fn t_color() -> MonoType {
    MonoType::Variant("color".to_string(), Vec::new())
}
/// `pre-path` (vminst.ml's `tPRP`; v0.0.6 `PrePathType`).
pub fn t_prepath() -> MonoType {
    MonoType::Base(BaseType::PrePath)
}
/// `path` (vminst.ml's `tPATH`; v0.0.6 `PathType`).
pub fn t_path() -> MonoType {
    MonoType::Base(BaseType::Path)
}
/// `graphics` (vminst.ml's `tGR`; v0.0.6 `GraphicsType`).
pub fn t_graphics() -> MonoType {
    MonoType::Base(BaseType::Graphics)
}
/// `point = length * length` (vminst.ml's `tPT = tPROD[tLN;tLN]`) —
/// structural, not a `BaseType`: a point is just a 2-tuple of lengths,
/// matching the runtime representation (`Value::Tuple([Length, Length])`,
/// see `primitives.rs`'s `as_point`/`make_point_value`).
pub fn t_point() -> MonoType {
    product(vec![t_length(), t_length()])
}
/// `page-break-info` (vminst.ml's `tPBINFO`) — the closed record row `{|
/// page-number : int |}` a `hook-page-break` closure's first argument
/// receives. The port has first-class row-typed records
/// (`types::MonoType::Record`) and `#field` access, so this type-checks
/// structurally with no nominal `tPBINFO` variant needed
/// (`docs/plans/hooks-annotations-crossref.md` §Slice 1 point 5). Runtime:
/// `Value::Record` with the single `"page-number"` key, built by
/// `fire_hooks` (`lib.rs`).
pub fn t_pbinfo() -> MonoType {
    MonoType::Record(types::Row::Cons(
        "page-number".to_string(),
        Box::new(t_int()),
        Box::new(types::Row::Empty),
    ))
}
/// `page` (vminst.ml's `tPG`) — the one new nominal variant this plan adds:
/// `A0Paper | A1Paper | ... | A5Paper | USLetter | USLegal |
/// UserDefinedPaper of (length * length)` (`primitives.cppo.ml:203-212`),
/// registered by [`builtin_variants`]. `page-break`'s first argument
/// selects the whole document's paper size.
pub fn t_page() -> MonoType {
    MonoType::Variant("page".to_string(), Vec::new())
}
/// `page-content-scheme` (vminst.ml's `tPAGECONT`) — the closed record row
/// `{| text-origin : point; text-height : length |}` a `page-break`
/// content-scheme closure returns, applied once per page with that page's
/// `pbinfo` (`docs/plans/document-page-model.md` §"The scheme model").
/// Structural, like `t_pbinfo` above — no nominal type needed.
pub fn t_page_content_scheme() -> MonoType {
    MonoType::Record(types::Row::Cons(
        "text-origin".to_string(),
        Box::new(t_point()),
        Box::new(types::Row::Cons(
            "text-height".to_string(),
            Box::new(t_length()),
            Box::new(types::Row::Empty),
        )),
    ))
}
/// `page-parts` (vminst.ml's `tPAGEPARTS`) — the closed record row `{|
/// header-origin : point; header-content : block-boxes; footer-origin :
/// point; footer-content : block-boxes |}` a `page-break` parts-scheme
/// closure returns, applied once per page with that page's `pbinfo`.
pub fn t_page_parts() -> MonoType {
    MonoType::Record(types::Row::Cons(
        "header-origin".to_string(),
        Box::new(t_point()),
        Box::new(types::Row::Cons(
            "header-content".to_string(),
            Box::new(t_block_boxes()),
            Box::new(types::Row::Cons(
                "footer-origin".to_string(),
                Box::new(t_point()),
                Box::new(types::Row::Cons(
                    "footer-content".to_string(),
                    Box::new(t_block_boxes()),
                    Box::new(types::Row::Empty),
                )),
            )),
        )),
    ))
}
/// `'a option` (vminst.ml's `tOPT`) — the built-in `option` variant
/// (`builtin_variants`'s `option_decl`) applied to `ty`.
pub fn t_option(ty: MonoType) -> MonoType {
    MonoType::Variant("option".to_string(), vec![ty])
}
/// `script` (vminst.ml's `tSCR`) — a built-in **variant** (same shape as
/// `t_color()` above): `HanIdeographic | Kana | Latin | OtherScript`
/// (upstream's real surface constructor set, `primitives.cppo.ml:192-196`),
/// registered by [`builtin_variants`]. `script-guard` (pervasives.satyh's
/// `\SATySFi`/`\LaTeX`/`\TeX`) is its first consumer.
pub fn t_script() -> MonoType {
    MonoType::Variant("script".to_string(), Vec::new())
}
/// `language` (vminst.ml's `tLANG`; `charBasis.ml`'s `language_system =
/// Japanese | English | NoLanguageSystem`) — a nullary built-in variant,
/// same shape as [`t_script`]. `set-language`'s 2nd argument
/// (`stdja.satyh`'s `set-language Kana Japanese`).
pub fn t_language() -> MonoType {
    MonoType::Variant("language".to_string(), Vec::new())
}
/// `text-info` (vminst.ml's `tTCTX`; v0.0.6 `TextInfoType`) — the text-mode
/// context. §G sliver: only the three pure prims below produce/consume it.
pub fn t_text_info() -> MonoType {
    MonoType::Base(BaseType::TextInfo)
}
/// `paddings` (vminst.ml's `tPADS = tPROD [tLN;tLN;tLN;tLN]`) — a plain
/// 4-tuple `(paddingL, paddingR, paddingT, paddingB)`, matching the runtime
/// shape `primitives.rs`'s `as_paddings` reads (mirrors `evalUtil.ml`'s
/// `get_paddings` field order).
pub fn t_paddings() -> MonoType {
    product(vec![t_length(), t_length(), t_length(), t_length()])
}
/// `cell` (`primitives.cppo.ml:214-217`'s `NormalCell of (paddings *
/// inline-boxes) | EmptyCell | MultiCell of (int * int * paddings *
/// inline-boxes)`) — a built-in **variant** (same shape as `t_color()`
/// above), registered by [`builtin_variants`]. `tabular`'s first argument is
/// `(cell list) list`; docs/plans/table-subsystem.md §Slice 1.
pub fn t_cell() -> MonoType {
    MonoType::Variant("cell".to_string(), Vec::new())
}
/// `dash` (`graphicD.ml`'s `type dash = length * length * length`) —
/// `dashed-stroke`'s 2nd argument, `(d1, d2, d0)` = on-length, off-length,
/// phase.
pub fn t_dash() -> MonoType {
    product(vec![t_length(), t_length(), t_length()])
}
/// `deco` (vminst.ml's `tDECO_raw = tPT @-> tLN @-> tLN @-> tLN @-> (tL
/// tGR)`) — a callback `point -> length -> length -> length -> graphics
/// list`, invoked (once placed) with its own position and resolved
/// width/height/depth. `inline-frame-outer`'s stand-in body
/// (`primitives.rs`) never actually calls it (see that primitive's doc
/// comment), but it is typed faithfully so callers still type-check exactly
/// as they would upstream.
pub fn t_deco() -> MonoType {
    arrows(
        vec![t_point(), t_length(), t_length(), t_length()],
        list(t_graphics()),
    )
}
/// `deco-set = deco * deco * deco * deco` (vminst.ml's `tDECOSET`) —
/// `block-frame-breakable`'s third argument (the four edge/corner
/// decoration closures a frame would fire at placement time). STAND-IN
/// body (`primitives.rs`'s `prim_block_frame_breakable`) pops and drops it
/// entirely (like `t_deco()`'s own callers above), but it is typed
/// faithfully here so callers still type-check exactly as they would
/// upstream. docs/plans/context-box-prims.md §4.
pub fn t_decoset() -> MonoType {
    product(vec![t_deco(); 4])
}
/// `font = string * float * float` (vminst.ml's `tFONT`) — an
/// `(abbrev, size_ratio, rising_ratio)` triple; `set-font`'s second
/// argument. docs/plans/context-box-prims.md §6.
pub fn t_font() -> MonoType {
    product(vec![t_string(), t_float(), t_float()])
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
        // `page-break : page -> (pbinfo -> page-content-scheme) -> (pbinfo
        // -> page-parts) -> block-boxes -> document` (vminst.ml:1024,
        // `BackendPageBreaking`: `~% (tPG @-> tPAGECONTF @-> tPAGEPARTSF @->
        // tBB @-> tDOC)`) — the real 4-arg primitive
        // (docs/plans/document-page-model.md Slice 1), up from the old
        // LOCAL 2-arg `context -> block-boxes -> document`.
        "page-break" => poly0(arrows(
            vec![
                t_page(),
                arrow(t_pbinfo(), t_page_content_scheme()),
                arrow(t_pbinfo(), t_page_parts()),
                t_block_boxes(),
            ],
            t_document(),
        )),
        // `page-break-multicolumn : page -> length list -> (unit ->
        // block-boxes) -> (unit -> block-boxes) -> (pbinfo ->
        // page-content-scheme) -> (pbinfo -> page-parts) -> block-boxes ->
        // document` (vminst.ml:1065 `BackendPageBreakingMultiColumn`:
        // `~% (tPG @-> tL tLN @-> (tU @-> tBB) @-> (tU @-> tBB) @->
        // tPAGECONTF @-> tPAGEPARTSF @-> tBB @-> tDOC)`) — FAITHFUL, see
        // `primitives.rs`'s `prim_page_break_multicolumn` / `page_break_core`.
        "page-break-multicolumn" => poly0(arrows(
            vec![
                t_page(),
                list(t_length()),
                arrow(t_unit(), t_block_boxes()),
                arrow(t_unit(), t_block_boxes()),
                arrow(t_pbinfo(), t_page_content_scheme()),
                arrow(t_pbinfo(), t_page_parts()),
                t_block_boxes(),
            ],
            t_document(),
        )),
        // `page-break-two-column : page -> length -> (unit -> block-boxes)
        // -> (pbinfo -> page-content-scheme) -> (pbinfo -> page-parts) ->
        // block-boxes -> document` (vminst.ml:1041
        // `BackendPageBreakingTwoColumn`: `~% (tPG @-> tLN @-> (tU @-> tBB)
        // @-> tPAGECONTF @-> tPAGEPARTSF @-> tBB @-> tDOC)`).
        "page-break-two-column" => poly0(arrows(
            vec![
                t_page(),
                t_length(),
                arrow(t_unit(), t_block_boxes()),
                arrow(t_pbinfo(), t_page_content_scheme()),
                arrow(t_pbinfo(), t_page_parts()),
                t_block_boxes(),
            ],
            t_document(),
        )),
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
        // `~% (tLN @-> tICMD tMATH @-> tCTX)` — faithfully, a `[math]
        // inline-cmd` (our `MonoType::InlineCmd` with exactly one mandatory
        // `math` argument), NOT `MathCmd` (that's `MathCommandType`, the
        // different v0.0.6 type used for math-mode commands like `\sqrt`).
        // RESTORED (`docs/plans/math-engine.md` §G): `(command \math)`
        // (`class-signature-lang-gaps.md` gap 1) constructs the first-class
        // command reference this needs; every call site now passes
        // `(command \math)` (or a local stub command) instead of `()`.
        // FAITHFUL: `prim_get_initial_context` (primitives.rs) interns the
        // command via `Interp::register_math_command` and installs it as
        // `Context::math_command`, consulted by `read_inline`'s `EmbedMath`
        // arm for bare `${…}` in prose.
        "get-initial-context" => poly0(arrow(
            t_length(),
            arrow(inline_cmd(vec![mandatory(t_math_text())]), t_context()),
        )),

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

        // ---- images (Slice 1: raster images; docs/plans/math-images.md,
        // mirroring v0.0.6 vminstdef.yaml:540/:554; `load-pdf-image`,
        // vminstdef.yaml:525, is deferred) ----
        // `load-image : string -> image`.
        "load-image" => poly0(arrow(t_string(), t_image())),
        // `use-image-by-width : image -> length -> inline-boxes`.
        "use-image-by-width" => poly0(arrows(
            vec![t_image(), t_length()],
            t_inline_boxes(),
        )),

        // ---- inline-fil ----
        // Not a primitive *function* at all (`base_env` binds it directly
        // to a constant `Value::InlineBoxes`, primitives.rs), so there is
        // no vminst.ml `~type_:` to cite; its type is simply that of the
        // value it names.
        "inline-fil" => poly0(t_inline_boxes()),
        // `omit-skip-after` (`primitives.cppo.ml:567`) — same shape as
        // `inline-fil` above (a bare constant, STAND-IN body; see
        // `primitives.rs`'s `base_env` comment).
        "omit-skip-after" => poly0(t_inline_boxes()),
        // `clear-page : block-boxes` (`primitives.cppo.ml:569`: `("clear-
        // page", ~% tBB, ..)`) — same shape as `inline-fil`/`omit-skip-
        // after` above: a bare constant (`base_env` binds it to
        // `Value::BlockBoxes(vec![VertBox::ClearPage])`), FAITHFUL —
        // `mitou-report.satyh`'s `document` unblocker.
        "clear-page" => poly0(t_block_boxes()),

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

        // ==== Slice 1 graphics primitives (docs/plans/graphics-subsystem.md
        // §2) — paths, fill/stroke, and the `inline-graphics` on-page sink.
        // Argument order transcribed from `tools/gencode/vminst.ml`:
        // `start-path` :713, `line-to` :727, `terminate-path` :759,
        // `close-with-line` :773, `fill` :2398, `stroke` :2381,
        // `inline-graphics` :1872. ====================================
        //
        // `start-path : point -> pre-path`.
        "start-path" => poly0(arrow(t_point(), t_prepath())),
        // `line-to : point -> pre-path -> pre-path` (point first).
        "line-to" => poly0(arrows(vec![t_point(), t_prepath()], t_prepath())),
        // `terminate-path : pre-path -> path` — finishes an OPEN subpath.
        "terminate-path" => poly0(arrow(t_prepath(), t_path())),
        // `close-with-line : pre-path -> path` — closes with a straight
        // segment back to the subpath's start.
        "close-with-line" => poly0(arrow(t_prepath(), t_path())),
        // `fill : color -> path -> graphics` — even-odd filled region.
        "fill" => poly0(arrows(vec![t_color(), t_path()], t_graphics())),
        // `stroke : length -> color -> path -> graphics` — width first.
        "stroke" => poly0(arrows(
            vec![t_length(), t_color(), t_path()],
            t_graphics(),
        )),
        // `inline-graphics : length -> length -> length -> (point ->
        // graphics list) -> inline-boxes` — a box of size (w, h, d) carrying
        // the callback's graphics, the minimal on-page sink for `graphics`
        // values (see that primitive's body, `primitives.rs`, for the
        // eager-callback-at-origin caveat this signature doesn't capture).
        "inline-graphics" => poly0(arrows(
            vec![
                t_length(),
                t_length(),
                t_length(),
                arrow(t_point(), list(t_graphics())),
            ],
            t_inline_boxes(),
        )),

        // `tabular : (cell list) list -> (length list -> length list ->
        // graphics list) -> inline-boxes` (vminst.ml:539, `tRULESF = (tL
        // tLN) @-> (tL tLN) @-> (tL tGR)` at primitives.cppo.ml:141) — the
        // ruled-grid primitive; docs/plans/table-subsystem.md §Slice 1.
        "tabular" => poly0(arrows(
            vec![
                list(list(t_cell())),
                arrows(vec![list(t_length()), list(t_length())], list(t_graphics())),
            ],
            t_inline_boxes(),
        )),

        // vminst.ml:1891 `BackendInlineGraphicsOuter`: `~% (tLN @-> tLN @->
        // tIGRO @-> tIB)` — tIGRO = `length -> point -> graphics list` (the
        // resolved width, then the placed point).
        "inline-graphics-outer" => poly0(arrows(
            vec![
                t_length(),
                t_length(),
                arrows(vec![t_length(), t_point()], list(t_graphics())),
            ],
            t_inline_boxes(),
        )),

        // ==== gr.satyh roadmap prims (docs/plans/graphics-subsystem.md
        // §Full roadmap A/B/C/D) — signatures from `tools/gencode/vminst.ml`:
        // `bezier-to` :742, `close-with-bezier` :787, `shift-path` :663,
        // `linear-transform-path` :678, `shift-graphics` :2451,
        // `linear-transform-graphics` :2432, `get-graphics-bbox` :2466,
        // `dashed-stroke` :2414, `draw-text` :2363. ====================
        //
        // `bezier-to : point -> point -> point -> pre-path -> pre-path`.
        "bezier-to" => poly0(arrows(
            vec![t_point(), t_point(), t_point(), t_prepath()],
            t_prepath(),
        )),
        // `close-with-bezier : point -> point -> pre-path -> path`.
        "close-with-bezier" => poly0(arrows(
            vec![t_point(), t_point(), t_prepath()],
            t_path(),
        )),
        // `shift-path : point -> path -> path`.
        "shift-path" => poly0(arrows(vec![t_point(), t_path()], t_path())),
        // `linear-transform-path : float -> float -> float -> float -> path
        // -> path`.
        "linear-transform-path" => poly0(arrows(
            vec![t_float(), t_float(), t_float(), t_float(), t_path()],
            t_path(),
        )),
        // `shift-graphics : point -> graphics -> graphics`.
        "shift-graphics" => poly0(arrows(vec![t_point(), t_graphics()], t_graphics())),
        // `linear-transform-graphics : float -> float -> float -> float ->
        // graphics -> graphics` (see `primitives.rs`'s body for the
        // eager-vs-upstream's-lazy-`cm` stroke-width caveat this signature
        // doesn't capture).
        "linear-transform-graphics" => poly0(arrows(
            vec![t_float(), t_float(), t_float(), t_float(), t_graphics()],
            t_graphics(),
        )),
        // `get-graphics-bbox : graphics -> point * point`.
        "get-graphics-bbox" => poly0(arrow(t_graphics(), product(vec![t_point(), t_point()]))),
        // `get-path-bbox : path -> point * point` (vminst.ml:696
        // `PathGetBoundingBox`).
        "get-path-bbox" => poly0(arrow(t_path(), product(vec![t_point(), t_point()]))),
        // `dashed-stroke : length -> dash -> color -> path -> graphics`
        // (width first, like `stroke`, with the dash pattern inserted next).
        "dashed-stroke" => poly0(arrows(
            vec![t_length(), t_dash(), t_color(), t_path()],
            t_graphics(),
        )),
        // `draw-text : point -> inline-boxes -> graphics` — FAITHFUL (see
        // `primitives.rs`'s `prim_draw_text`).
        "draw-text" => poly0(arrows(vec![t_point(), t_inline_boxes()], t_graphics())),

        // ==== pervasives.satyh unblockers (docs/plans/stdlib-port.md) ====
        //
        // vminst.ml:2020 `PrimitiveGetNaturalMetrics`:
        // `~% (tIB @-> tPROD [tLN; tLN; tLN])`.
        "get-natural-metrics" => poly0(arrow(
            t_inline_boxes(),
            product(vec![t_length(), t_length(), t_length()]),
        )),
        // vminst.ml:1787 `BackendOuterFrame`: `~% (tPADS @-> tDECO @-> tIB
        // @-> tIB)`. STAND-IN body — see primitives.rs's
        // `prim_inline_frame_outer` doc comment.
        "inline-frame-outer" => poly0(arrows(
            vec![t_paddings(), t_deco(), t_inline_boxes()],
            t_inline_boxes(),
        )),
        // vminst.ml:1807 `BackendInnerFrame`: `~% (tPADS @-> tDECO @-> tIB
        // @-> tIB)` — same signature as `inline-frame-outer` above.
        "inline-frame-inner" => poly0(arrows(
            vec![t_paddings(), t_deco(), t_inline_boxes()],
            t_inline_boxes(),
        )),
        // vminst.ml:1661 `PrimitiveSetManualRising`: `~% (tLN @-> tCTX @->
        // tCTX)`.
        "set-manual-rising" => poly0(arrow(t_length(), arrow(t_context(), t_context()))),
        // vminst.ml:1908 `BackendScriptGuard`: `~% (tSCR @-> tIB @-> tIB)`.
        // STAND-IN body (identity) — see primitives.rs's `prim_script_guard`
        // doc comment.
        "script-guard" => poly0(arrows(vec![t_script(), t_inline_boxes()], t_inline_boxes())),
        // vminst.ml:1969 `BackendDiscretionary`: `~% (tI @-> tIB @-> tIB
        // @-> tIB @-> tIB)`, params `(pb, hblst0, hblst1, hblst2)`.
        "discretionary" => poly0(arrows(
            vec![
                t_int(),
                t_inline_boxes(),
                t_inline_boxes(),
                t_inline_boxes(),
            ],
            t_inline_boxes(),
        )),

        // ==== Tier-2 decoration/graphics packages ====
        //
        // vminst.ml:1739 `PrimitiveGetAxisHeight`: `~% (tCTX @-> tLN)`.
        // STAND-IN body — see `primitives.rs`'s `prim_get_axis_height` doc
        // comment.
        "get-axis-height" => poly0(arrow(t_context(), t_length())),

        // ==== docs/plans/hooks-annotations-crossref.md §Slice 1 ====
        //
        // vminstdef.yaml:576 `~% ((tPBINFO @-> tPT @-> tU) @-> tIB)`.
        "hook-page-break" => poly0(arrow(
            arrows(vec![t_pbinfo(), t_point()], t_unit()),
            t_inline_boxes(),
        )),
        // vminst.ml:632 `BackendHookPageBreakBlock`:
        // `~% ((tPBINFO @-> tPT @-> tU) @-> tBB)` — the block-level analog
        // of `hook-page-break` above, FAITHFUL: see `primitives.rs`'s
        // `prim_hook_page_break_block`. `stdjareport.satyh`'s `document`
        // unblocker.
        "hook-page-break-block" => poly0(arrow(
            arrows(vec![t_pbinfo(), t_point()], t_unit()),
            t_block_boxes(),
        )),
        // vminstdef.yaml:1793 `~% (tS @-> tS @-> tU)`.
        "register-cross-reference" => {
            poly0(arrows(vec![t_string(), t_string()], t_unit()))
        }
        // vminstdef.yaml:1808 `~% (tS @-> tOPT tS)`.
        "get-cross-reference" => poly0(arrow(t_string(), t_option(t_string()))),
        // vminst.ml:3043 `BackendProbeCrossReference`: `~% (tS @-> tOPT tS)`
        // — `get-cross-reference` without the recorded miss. FAITHFUL.
        "probe-cross-reference" => poly0(arrow(t_string(), t_option(t_string()))),

        // ==== docs/plans/hooks-annotations-crossref.md §B/§D: annot.satyh's
        // prim surface. STAND-IN bodies — see primitives.rs's
        // `prim_get_leftmost_script`/`prim_inline_frame_breakable`/
        // `prim_register_destination` doc comments. ====
        //
        // vminstdef.yaml:1754/1767 `~% (tIB @-> tOPT tSCR)`.
        "get-leftmost-script" => poly0(arrow(t_inline_boxes(), t_option(t_script()))),
        "get-rightmost-script" => poly0(arrow(t_inline_boxes(), t_option(t_script()))),
        // vminstdef.yaml:1672 `~% (tPADS @-> tDECOSET @-> tIB @-> tIB)`.
        "inline-frame-breakable" => poly0(arrows(
            vec![t_paddings(), t_decoset(), t_inline_boxes()],
            t_inline_boxes(),
        )),
        // vminstdef.yaml:2738 `~% (tS @-> tPT @-> tU)`.
        "register-destination" => poly0(arrows(vec![t_string(), t_point()], t_unit())),
        // vminstdef.yaml:2753/2773 `~% (tS @-> tPT @-> tLN @-> tLN @-> tLN
        // @-> (tOPT (tPROD [tLN; tCLR])) @-> tU)`.
        "register-link-to-uri" => poly0(arrows(
            vec![
                t_string(),
                t_point(),
                t_length(),
                t_length(),
                t_length(),
                t_option(product(vec![t_length(), t_color()])),
            ],
            t_unit(),
        )),
        "register-link-to-location" => poly0(arrows(
            vec![
                t_string(),
                t_point(),
                t_length(),
                t_length(),
                t_length(),
                t_option(product(vec![t_length(), t_color()])),
            ],
            t_unit(),
        )),

        // ==== docs/plans/math-engine.md §A + §G: the faithful `Value::Math`
        // primitive layer `math.satyh` is built out of. Signatures
        // transcribed from `tools/gencode/vminst.ml` (cited per entry); the
        // "Phase" column of the plan's primitive-inventory table follows —
        // A unless noted. ====
        //
        // vminst.ml:388 `BackendMathChar`: `~% (tMATHCLS @-> tS @-> tMATH)`.
        "math-char" => poly0(arrows(vec![t_math_class(), t_string()], t_math())),
        // vminst.ml:405 `BackendMathBigChar` — same shape, large-operator
        // size class (roadmap D upscales it; Slice 1 renders it the same
        // size as `math-char`).
        "math-big-char" => poly0(arrows(vec![t_math_class(), t_string()], t_math())),
        // vminst.ml:422 `BackendMathCharWithKern`:
        // `~% (tMATHCLS @-> tS @-> mckf @-> mckf @-> tMATH)`.
        "math-char-with-kern" => poly0(arrows(
            vec![t_math_class(), t_string(), t_math_kern_func(), t_math_kern_func()],
            t_math(),
        )),
        // vminst.ml:445 `BackendMathBigCharWithKern` — same shape.
        "math-big-char-with-kern" => poly0(arrows(
            vec![t_math_class(), t_string(), t_math_kern_func(), t_math_kern_func()],
            t_math(),
        )),
        // vminst.ml:193 `BackendMathConcat`: `~% (tMATH @-> tMATH @-> tMATH)`.
        "math-concat" => poly0(arrows(vec![t_math(), t_math()], t_math())),
        // vminst.ml:209 `BackendMathGroup`:
        // `~% (tMATHCLS @-> tMATHCLS @-> tMATH @-> tMATH)`.
        "math-group" => poly0(arrows(
            vec![t_math_class(), t_math_class(), t_math()],
            t_math(),
        )),
        // vminst.ml:226 `BackendMathSuperscript`.
        "math-sup" => poly0(arrows(vec![t_math(), t_math()], t_math())),
        // vminst.ml:242 `BackendMathSubscript`.
        "math-sub" => poly0(arrows(vec![t_math(), t_math()], t_math())),
        // vminst.ml:258 `BackendMathFraction`. Phase C.
        "math-frac" => poly0(arrows(vec![t_math(), t_math()], t_math())),
        // vminst.ml:274 `BackendMathRadical`:
        // `~% (tOPT tMATH @-> tMATH @-> tMATH)`. Phase C.
        "math-radical" => poly0(arrows(vec![t_option(t_math()), t_math()], t_math())),
        // vminst.ml:352 `BackendMathLowerLimit`. Phase D.
        "math-lower" => poly0(arrows(vec![t_math(), t_math()], t_math())),
        // vminst.ml:336 `BackendMathUpperLimit`. Phase D.
        "math-upper" => poly0(arrows(vec![t_math(), t_math()], t_math())),
        // vminst.ml:368 `BackendMathPullInScripts`:
        // `~% (tMATHCLS @-> tMATHCLS @-> (tOPT tMATH @-> tOPT tMATH @-> tMATH)
        // @-> tMATH)`. Phase D.
        "math-pull-in-scripts" => poly0(arrows(
            vec![
                t_math_class(),
                t_math_class(),
                arrows(vec![t_option(t_math()), t_option(t_math())], t_math()),
            ],
            t_math(),
        )),
        // vminst.ml:488 `BackendMathColor`: `~% (tCLR @-> tMATH @-> tMATH)`.
        "math-color" => poly0(arrows(vec![t_color(), t_math()], t_math())),
        // vminst.ml:504 `BackendMathCharClass`:
        // `~% (tMCCLS @-> tMATH @-> tMATH)`. Phase F.
        "math-char-class" => poly0(arrows(vec![t_math_char_class(), t_math()], t_math())),
        // vminst.ml:111 `BackendMathVariantCharDirect`:
        // `~% (tMATHCLS @-> tMCSTY @-> tMATH)`. Phase F.
        "math-variant-char" => poly0(arrows(
            vec![t_math_class(), t_math_variant_style()],
            t_math(),
        )),
        // gap 7 (`docs/plans/math-mode-language-gaps.md`) — no bundled
        // `.satyh` consumer, but v0.0.6-shaped: `math-char-class -> int ->
        // int -> context -> context` (`set-math-variant-char`); `context ->
        // math -> math-class option` (the boundary-class introspection
        // pair).
        "set-math-variant-char" => poly0(arrows(
            vec![t_math_char_class(), t_int(), t_int(), t_context()],
            t_context(),
        )),
        "get-left-math-class" => poly0(arrows(
            vec![t_context(), t_math()],
            t_option(t_math_class()),
        )),
        "get-right-math-class" => poly0(arrows(
            vec![t_context(), t_math()],
            t_option(t_math_class()),
        )),
        // vminst.ml:294 `BackendMathParen`:
        // `~% (tPAREN @-> tPAREN @-> tMATH @-> tMATH)`. Phase D.
        "math-paren" => poly0(arrows(vec![t_paren(), t_paren(), t_math()], t_math())),
        // vminst.ml:314 `BackendMathParenWithMiddle`:
        // `~% (tPAREN @-> tPAREN @-> tPAREN @-> tL tMATH @-> tMATH)`. Phase D.
        "math-paren-with-middle" => poly0(arrows(
            vec![t_paren(), t_paren(), t_paren(), list(t_math())],
            t_math(),
        )),
        // vminst.ml:468 `BackendMathText` (named `text-in-math`):
        // `~% (tMATHCLS @-> (tCTX @-> tIB) @-> tMATH)`. Phase E.
        "text-in-math" => poly0(arrows(
            vec![t_math_class(), arrow(t_context(), t_inline_boxes())],
            t_math(),
        )),
        // vminst.ml:61 `PrimitiveConvertStringForMath`:
        // `~% (tCTX @-> tMCCLS @-> tS @-> tS)`. Phase F — STAND-IN body (see
        // `primitives.rs`'s `prim_convert_string_for_math`).
        "convert-string-for-math" => poly0(arrows(
            vec![t_context(), t_math_char_class(), t_string()],
            t_string(),
        )),
        // vminst.ml:520 `BackendEmbeddedMath` (named `embed-math`):
        // `~% (tCTX @-> tMATH @-> tIB)` — the bridge to the page; `\math`
        // (math.satyh:439) wraps this.
        "embed-math" => poly0(arrows(vec![t_context(), t_math()], t_inline_boxes())),
        // vminst.ml:77 `PrimitiveSetMathCommand`:
        // `~% (tICMD tMATH @-> tCTX @-> tCTX)` — installs the default
        // command a bare `${…}`-in-text dispatches to. FAITHFUL: interned
        // via `Interp::register_math_command` and installed as
        // `Context::math_command`, consulted by `read_inline`'s `EmbedMath`
        // arm.
        "set-math-command" => poly0(arrow(
            inline_cmd(vec![mandatory(t_math())]),
            arrow(t_context(), t_context()),
        )),
        // vminst.ml:1495 `PrimitiveSetMathFont`: `~% (tS @-> tCTX @-> tCTX)`.
        // Phase B — STAND-IN body (no `MathFontStore` yet; not called by
        // `math.satyh` itself, registered for signature parity).
        "set-math-font" => poly0(arrow(t_string(), arrow(t_context(), t_context()))),
        // vminst.ml:173 `BackendSpaceBetweenMaths`:
        // `~% (tCTX @-> tMATH @-> tMATH @-> tOPT tIB)`. Phase E — STAND-IN
        // body (the full `space_between_math_kinds` table is phase A.4,
        // roadmap); used by `math.satyh`'s `+align`.
        "space-between-maths" => poly0(arrows(
            vec![t_context(), t_math(), t_math()],
            t_option(t_inline_boxes()),
        )),
        // vminst.ml:1677 `PrimitiveRaiseInline` (name inferred from usage;
        // not independently confirmed against a `~name:` line):
        // `~% (tLN @-> tIB @-> tIB)`. STAND-IN body — see `primitives.rs`'s
        // `prim_raise_inline` doc comment (no per-box vertical-offset
        // wrapper in the line model yet outside `PureHorzBox::Math`).
        "raise-inline" => poly0(arrows(vec![t_length(), t_inline_boxes()], t_inline_boxes())),
        // vminst.ml:973 `PrimitiveEmbeddedVertBreakable` (named
        // `embed-block-breakable`): `~% (tCTX @-> tBB @-> tIB)`. STAND-IN
        // body — no nested page-breakable block-in-inline box yet (roadmap
        // E; see `primitives.rs`'s `prim_embed_block_breakable`).
        "embed-block-breakable" => poly0(arrows(
            vec![t_context(), t_block_boxes()],
            t_inline_boxes(),
        )),
        // `unite-path : path -> path -> path` (`gr.satyh`-adjacent path
        // combinator; `math.satyh`'s `\norm` unions two vertical bars into
        // one path). FAITHFUL: a real path union (concatenation of
        // subpaths — see `primitives.rs`'s `prim_unite_path`).
        "unite-path" => poly0(arrows(vec![t_path(), t_path()], t_path())),
        // vminst.ml:1291 `PrimitiveSetMinGapOfLines`:
        // `~% (tLN @-> tCTX @-> tCTX)` — a *different* context field than
        // `set-leading` (see that primitive's own comment); `math.satyh`'s
        // `+math-list` calls this. STAND-IN body: no separate
        // `min_gap_of_lines` field on `Context` yet, so this is a same-
        // shape passthrough (see `primitives.rs`'s
        // `prim_set_min_gap_of_lines`).
        "set-min-gap-of-lines" => poly0(arrow(t_length(), arrow(t_context(), t_context()))),

        // ==== docs/plans/context-box-prims.md §Slice 1 (rows 1-10): the
        // context-setter + box-combinator prims `code.satyh`/`itemize.satyh`
        // need. Signatures transcribed from `tools/gencode/vminst.ml` (cited
        // per entry). ====
        //
        // vminst.ml:1603 `PrimitiveSetTextColor`: `~% (tCLR @-> tCTX @-> tCTX)`.
        // FAITHFUL store (`primitives.rs`'s `prim_set_text_color`) — glyph-
        // color *rendering* is a later follow-on, see that primitive's doc
        // comment.
        "set-text-color" => poly0(arrow(t_color(), arrow(t_context(), t_context()))),
        // vminst.ml:1618 `PrimitiveGetTextColor`: `~% (tCTX @-> tCLR)`.
        // FAITHFUL — `itemize.satyh` feeds this straight into `fill`, see
        // `primitives.rs`'s `prim_get_text_color`/`make_color_value`.
        "get-text-color" => poly0(arrow(t_context(), t_color())),
        // vminst.ml:1692 `PrimitiveSetHyphenPenalty`: `~% (tI @-> tCTX @-> tCTX)`.
        // FAITHFUL store; no consumer yet (no hyphenation).
        "set-hyphen-penalty" => poly0(arrow(t_int(), arrow(t_context(), t_context()))),
        // vminst.ml:1309 `PrimitiveSetSpaceRatio`:
        // `~% (tFL @-> tFL @-> tFL @-> tCTX @-> tCTX)`, params
        // `(natural, shrink, stretch)`. FAITHFUL store; consumption by the
        // line breaker is a `docs/plans/text-rendering.md` follow-on.
        "set-space-ratio" => poly0(arrows(
            vec![t_float(), t_float(), t_float(), t_context()],
            t_context(),
        )),
        // vminst.ml:2269 `PrimitiveSplitIntoLines`:
        // `~% (tS @-> tL (tPROD [tI; tS]))`. FAITHFUL — pure string op, see
        // `primitives.rs`'s `prim_split_into_lines`.
        "split-into-lines" => poly0(arrow(
            t_string(),
            list(product(vec![t_int(), t_string()])),
        )),
        // vminst.ml:1090 `PrimitiveBlockFrameBreakable`:
        // `~% (tCTX @-> tPADS @-> tDECOSET @-> (tCTX @-> tBB) @-> tBB)`.
        // STAND-IN: reduced-width + left-indent inner block, `deco-set`
        // dropped — see `primitives.rs`'s `prim_block_frame_breakable`.
        "block-frame-breakable" => poly0(arrows(
            vec![
                t_context(),
                t_paddings(),
                t_decoset(),
                arrow(t_context(), t_block_boxes()),
            ],
            t_block_boxes(),
        )),
        // vminst.ml:1145 `PrimitiveEmbeddedVertTop` (named `embed-block-top`):
        // `~% (tCTX @-> tLN @-> (tCTX @-> tBB) @-> tIB)`. STAND-IN:
        // top-aligned `PureHorzBox::EmbeddedBlock` — see `primitives.rs`'s
        // `prim_embed_block_top`.
        "embed-block-top" => poly0(arrows(
            vec![t_context(), t_length(), arrow(t_context(), t_block_boxes())],
            t_inline_boxes(),
        )),
        // vminst.ml:1185 `PrimitiveEmbeddedVertBottom` (named
        // `embed-block-bottom`): `~% (tCTX @-> tLN @-> (tCTX @-> tBB) @-> tIB)`.
        // Same STAND-IN shape as `embed-block-top` above — see
        // `primitives.rs`'s `prim_embed_block_bottom`.
        "embed-block-bottom" => poly0(arrows(
            vec![t_context(), t_length(), arrow(t_context(), t_block_boxes())],
            t_inline_boxes(),
        )),
        // vminst.ml:1229 `PrimitiveLineStackBottom` (named
        // `line-stack-bottom`): `~% ((tL tIB) @-> tIB)`. FAITHFUL — see
        // `primitives.rs`'s `prim_line_stack_bottom`.
        "line-stack-bottom" => poly0(arrow(list(t_inline_boxes()), t_inline_boxes())),
        // vminst.ml:1130 PrimitiveAddFootnote: ~% (tBB @-> tIB). FAITHFUL —
        // see primitives.rs's prim_add_footnote (footnote float
        // accumulator, docs/plans/document-page-model.md §C).
        "add-footnote" => poly0(arrow(t_block_boxes(), t_inline_boxes())),
        // vminst.ml:1463 `PrimitiveSetFont`: `~% (tSCR @-> tFONT @-> tCTX @-> tCTX)`.
        // STAND-IN: single `FontKey` slot, script ignored — see
        // `primitives.rs`'s `prim_set_font` (real per-script wiring is
        // `docs/plans/text-rendering.md`'s Slice 1).
        "set-font" => poly0(arrows(
            vec![t_script(), t_font(), t_context()],
            t_context(),
        )),
        // `set-code-text-command : [string] inline-cmd -> context -> context`
        // (`stdja:116`; orphan #4 of `docs/plans/build-order-to-stdja.md`,
        // not in any vminst.ml table this port has transcribed — no
        // upstream line cited). STAND-IN: `(command \cmd)` (`docs/plans/
        // class-signature-lang-gaps.md` gap 1, already landed) means real
        // programs CAN construct a `[string] inline-cmd` value to pass here
        // now — but `Context` (satysfi-backend) still can't hold an
        // arbitrary lang-side `Value` without an illegal reverse crate
        // dependency, and the one legal indirection this codebase uses for
        // that (`Interp::hooks`'s ID-table seam) lives in `eval.rs`, outside
        // this slice's file boundary — so, like `set-math-command`/
        // `set-math-font` above, the command is accepted and dropped (see
        // `primitives.rs`'s `prim_set_code_text_command`).
        "set-code-text-command" => poly0(arrow(
            inline_cmd(vec![mandatory(t_string())]),
            arrow(t_context(), t_context()),
        )),
        // vminst.ml:2040 `PrimitiveGetNaturalLength`: `~% (tBB @-> tLN)` —
        // `get-natural-width`'s block sibling. FAITHFUL: block height+depth
        // summed to one length via `measure_block` (satysfi-backend) — see
        // `primitives.rs`'s `prim_get_natural_length`.
        "get-natural-length" => poly0(arrow(t_block_boxes(), t_length())),

        // ==== `docs/plans/build-order-to-stdja.md` step 8/9 orphans: the
        // remaining stdja.satyh primitives with no prior slice.
        // `set-dominant-wide-script`/`set-dominant-narrow-script`/
        // `set-language` (rows 15/17/18) are now FAITHFUL stores
        // (context-box-prims.md §C landed, group E2) with real getter
        // round-trips just below; `set-every-word-break`/`register-outline`
        // remain STAND-INs (accepted, dropped) — see their `primitives.rs`
        // doc comments. ====
        //
        // vminst.ml:1511 `PrimitiveSetDominantWideScript`:
        // `~% (tSCR @-> tCTX @-> tCTX)`.
        "set-dominant-wide-script" => poly0(arrow(t_script(), arrow(t_context(), t_context()))),
        // vminst.ml:1539 `PrimitiveSetDominantNarrowScript`: same shape.
        "set-dominant-narrow-script" => poly0(arrow(t_script(), arrow(t_context(), t_context()))),
        // vminst.ml:1568 `PrimitiveSetLangSys`:
        // `~% (tSCR @-> tLANG @-> tCTX @-> tCTX)`.
        "set-language" => poly0(arrows(
            vec![t_script(), t_language(), t_context()],
            t_context(),
        )),
        // vminst.ml:1526/1555 `PrimitiveGetDominantWideScript`/
        // `...NarrowScript`: `~% (tCTX @-> tSCR)`. FAITHFUL.
        "get-dominant-wide-script" => poly0(arrow(t_context(), t_script())),
        "get-dominant-narrow-script" => poly0(arrow(t_context(), t_script())),
        // vminst.ml:1587 `PrimitiveGetLangSys`: `~% (tSCR @-> tCTX @-> tLANG)`.
        "get-language" => poly0(arrows(vec![t_script(), t_context()], t_language())),
        // vminst.ml:3007 `PrimitiveSetEveryWordBreak`:
        // `~% (tIB @-> tIB @-> tCTX @-> tCTX)`.
        "set-every-word-break" => poly0(arrows(
            vec![t_inline_boxes(), t_inline_boxes(), t_context()],
            t_context(),
        )),
        // vminstdef.yaml:2794 `BackendRegisterOutline`:
        // `~% ((tL(tPROD [tI; tS; tS; tB])) @-> tU)` — a list of `(depth,
        // title, label, is-frozen)` PDF-outline entries. STAND-IN: this
        // port has no PDF `/Outlines` writer yet (unlike `register-
        // destination`/`register-link-to-*`, which DO reach the PDF via
        // `Annotation`), so the list is accepted and discarded.
        "register-outline" => poly0(arrow(
            list(product(vec![t_int(), t_string(), t_string(), t_bool()])),
            t_unit(),
        )),
        // vminstdef.yaml:1565 `PrimitiveExtract`: `~% (tIB @-> tS)` —
        // FAITHFUL: concatenates every `InnerString` run's own text,
        // recursing into `Discretionary`'s `no_break` slot (mirrors
        // `horzBox.ml`'s `extract_string`; this port's box vocabulary has no
        // separate Rising/Frame wrapper box, since `inline-frame-breakable`
        // et al. already flatten into the same `Vec<HorzBox>` — see
        // `primitives.rs`'s `prim_extract_string`).
        "extract-string" => poly0(arrow(t_inline_boxes(), t_string())),

        // ==== docs/plans/context-box-prims.md §G (text-mode-context
        // sliver): the three PURE text-info prims. The text/html backends
        // (`stringify-inline`/`stringify-block`, `.satyh-text` loading) are
        // deliberately out of scope for this PDF port — see
        // `primitives.rs`'s section comment. ====
        //
        // vminst.ml:953 `TextGetInitialTextModeContext`: `~% (tU @-> tTCTX)`.
        "get-initial-text-info" => poly0(arrow(t_unit(), t_text_info())),
        // vminst.ml:921 `TextDeepenIndent`: `~% (tI @-> tTCTX @-> tTCTX)`.
        "deepen-indent" => poly0(arrows(vec![t_int(), t_text_info()], t_text_info())),
        // vminst.ml:935 `TextBreak`: `~% (tTCTX @-> tS)`.
        "break" => poly0(arrow(t_text_info(), t_string())),

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
    // constructor wrappers; `fill`/`stroke` (graphics Slice 1) also consume it.
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

    // `script` (pervasives.satyh's `\SATySFi`/`\LaTeX`/`\TeX`, via
    // `script-guard`) — nullary variant, upstream's real surface constructor
    // set (`primitives.cppo.ml:192-196`). `script-guard`'s stand-in body
    // (primitives.rs) never inspects which constructor it got; the full set
    // is registered anyway so the TYPE is faithful even though the behavior
    // isn't yet.
    let script_decl = VariantDecl {
        name: "script".to_string(),
        params: 0,
        ctors: vec![
            ("HanIdeographic".to_string(), None),
            ("Kana".to_string(), None),
            ("Latin".to_string(), None),
            ("OtherScript".to_string(), None),
        ],
        param_vars: Vec::new(),
    };

    // `language` (`charBasis.ml`'s `language_system`) — nullary variant,
    // `set-language`'s 2nd argument (`stdja.satyh`'s `set-language Kana
    // Japanese`). `set-language` itself (primitives.rs) never inspects
    // which constructor it got (same "type faithful, behavior not yet"
    // stand-in as `script` above).
    let language_decl = VariantDecl {
        name: "language".to_string(),
        params: 0,
        ctors: vec![
            ("Japanese".to_string(), None),
            ("English".to_string(), None),
            ("NoLanguageSystem".to_string(), None),
        ],
        param_vars: Vec::new(),
    };

    // `page` (docs/plans/document-page-model.md §Slice 1) — nullary variant,
    // the exact constructor set at `primitives.cppo.ml:204-212`: 8 nullary
    // paper-size constants plus `UserDefinedPaper` carrying a `(length *
    // length)` payload. `page-break`'s first argument; `as_page`
    // (`primitives.rs`) maps each ctor to a backend `PaperSize`.
    let page_decl = VariantDecl {
        name: "page".to_string(),
        params: 0,
        ctors: vec![
            ("A0Paper".to_string(), None),
            ("A1Paper".to_string(), None),
            ("A2Paper".to_string(), None),
            ("A3Paper".to_string(), None),
            ("A4Paper".to_string(), None),
            ("A5Paper".to_string(), None),
            ("USLetter".to_string(), None),
            ("USLegal".to_string(), None),
            (
                "UserDefinedPaper".to_string(),
                Some(product(vec![t_length(), t_length()])),
            ),
        ],
        param_vars: Vec::new(),
    };

    // `cell` (docs/plans/table-subsystem.md §Slice 1) — nullary variant,
    // transcribed from `primitives.cppo.ml:214-217`: `NormalCell of
    // (paddings * inline-boxes) | EmptyCell | MultiCell of (int * int *
    // paddings * inline-boxes)`. `EmptyCell`'s payload is `None` (this
    // port's nullary-constructor spelling, see this fn's doc comment),
    // matching upstream's `Poly(tU)` "no real payload" case.
    let cell_decl = VariantDecl {
        name: "cell".to_string(),
        params: 0,
        ctors: vec![
            (
                "NormalCell".to_string(),
                Some(product(vec![t_paddings(), t_inline_boxes()])),
            ),
            ("EmptyCell".to_string(), None),
            (
                "MultiCell".to_string(),
                Some(product(vec![
                    t_int(),
                    t_int(),
                    t_paddings(),
                    t_inline_boxes(),
                ])),
            ),
        ],
        param_vars: Vec::new(),
    };

    // `math-class` (docs/plans/math-engine.md §A item 2) — nullary variant,
    // transcribed from `primitives.cppo.ml:162-170`. **Distinct** from
    // `math-char-class` below (phase F's styling variant) — do not conflate.
    let math_class_decl = VariantDecl {
        name: "math-class".to_string(),
        params: 0,
        ctors: vec![
            ("MathOrd".to_string(), None),
            ("MathBin".to_string(), None),
            ("MathRel".to_string(), None),
            ("MathOp".to_string(), None),
            ("MathPunct".to_string(), None),
            ("MathOpen".to_string(), None),
            ("MathClose".to_string(), None),
            ("MathPrefix".to_string(), None),
            ("MathInner".to_string(), None),
        ],
        param_vars: Vec::new(),
    };

    // `math-char-class` (docs/plans/math-engine.md §F) — nullary variant,
    // `horzBox.ml:147`'s exact constructor set. Needed for `math.satyh`'s
    // `\mathrm`/`\mathbf`/`\mathcal`/`\mathfrak`/`\mathbb`/`\bm` to
    // type-check (each applies `math-char-class` to one of these).
    let math_char_class_decl = VariantDecl {
        name: "math-char-class".to_string(),
        params: 0,
        ctors: vec![
            ("MathItalic".to_string(), None),
            ("MathBoldItalic".to_string(), None),
            ("MathRoman".to_string(), None),
            ("MathBoldRoman".to_string(), None),
            ("MathScript".to_string(), None),
            ("MathBoldScript".to_string(), None),
            ("MathFraktur".to_string(), None),
            ("MathBoldFraktur".to_string(), None),
            ("MathDoubleStruck".to_string(), None),
        ],
        param_vars: Vec::new(),
    };

    vec![
        option_decl,
        itemize_decl,
        color_decl,
        script_decl,
        language_decl,
        page_decl,
        cell_decl,
        math_class_decl,
        math_char_class_decl,
    ]
}
