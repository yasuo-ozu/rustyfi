//! Type signatures for every primitive registered in `primitives.rs`'s
//! `prims!` table (plus the `inline-fil` constant), transcribed from
//! v0.0.6's `tools/gencode/vminst.ml` `~type_:` fields (cited by line
//! number at each entry below) and from `src/frontend/primitives.cppo.ml`
//! for the handful of names vminst.ml doesn't define directly (`::`, `!`,
//! the comparison trio derived in `general_table`).
//!
//! `document`, `+p` and `\emph` have no entry here: they are ordinary
//! `stdja-mini` stdlib bindings
//! (`lib-rustyfi/dist/packages/stdja-mini.satyh`), built out of *other*
//! primitives below and typechecked like any other `.satyh` library.
//!
//! Also provides `builtin_variants_with_version`, the seed set of variant
//! type declarations ([`VariantDecl`]) primitives.cppo.ml registers before
//! any user code runs.

use crate::types::{self, BaseType, CmdArgType, MonoType, PolyType, TyVarRef};
use rustyfi_syntax::RustyfiVersion;
use std::collections::HashMap;

// ============================================================================
// Constructor helpers — read the table below like vminst.ml's own `tI`,
// `tB`, `@->`, etc.
// ============================================================================

pub(crate) fn t_unit() -> MonoType {
    MonoType::Base(BaseType::Unit)
}
pub fn t_bool() -> MonoType {
    MonoType::Base(BaseType::Bool)
}
pub fn t_int() -> MonoType {
    MonoType::Base(BaseType::Int)
}
pub(crate) fn t_float() -> MonoType {
    MonoType::Base(BaseType::Float)
}
pub(crate) fn t_length() -> MonoType {
    MonoType::Base(BaseType::Length)
}
pub fn t_string() -> MonoType {
    MonoType::Base(BaseType::String)
}
pub(crate) fn t_inline_text() -> MonoType {
    MonoType::Base(BaseType::InlineText)
}
pub(crate) fn t_block_text() -> MonoType {
    MonoType::Base(BaseType::BlockText)
}
/// `math` (v0.0.6, vminst.ml's `tMATH`) / `math-text` (V0_1, upstream's
/// literal rename of 0.0.6's `math`, dev-0-1-0 `tMT`) — same `MonoType`
/// (`BaseType::MathText`) under both versions; only the surface NAME differs
/// (`typecheck.rs`'s `name_to_mono`, version-gated). `${…}`'s unparsed
/// source, in both generations.
pub(crate) fn t_math_text() -> MonoType {
    MonoType::Base(BaseType::MathText)
}
/// `math` — v0.0.6-only alias of [`t_math_text`] (both name the same
/// `BaseType::MathText`). V0_1 call sites should read `t_math_text()`
/// instead; this alias exists only for 0.0.6 signatures written against it.
fn t_math() -> MonoType {
    t_math_text()
}
/// `math-boxes` (V0_1 only; dev-0-1-0 vminst.ml's `tMB`) — the evaluated
/// math tree `read-math` produces. 0.0.6 has no name
/// for this type (its `math` conflates both halves).
pub(crate) fn t_math_boxes() -> MonoType {
    MonoType::Base(BaseType::MathBoxes)
}
/// `context -> math-boxes` (V0_1 only) — the script-callback type
/// `math-sup`/`math-sub`/`math-upper`/`math-lower` take in 0.1
/// (vminst.ml:208-353), evaluated under `enter_script`.
fn t_math_script_fn() -> MonoType {
    arrow(t_context(), t_math_boxes())
}
/// `math-class` (`primitives.cppo.ml:162-170`) — a built-in
/// **variant** (same shape as `t_color()`/`t_script()` above), registered by
/// `builtin_variants_with_version`. `math-char`/`math-group`/… all take this as an
/// argument. **Distinct** from `t_math_char_class()` below (the
/// `MathItalic`/… styling variant) — do not conflate the two.
fn t_math_class() -> MonoType {
    MonoType::Variant("math-class".to_string(), Vec::new())
}
/// `math-char-class` (`horzBox.ml:147`) — a built-in variant, registered by
/// `builtin_variants_with_version`. Needed for `math.satyh`'s `sig` (`\math-style :
/// [math-char-class; math] math-cmd`) and its
/// `\mathrm`/`\mathbf`/`\mathcal`/… definitions to type-check —
/// the actual Unicode-math-block restyling this variant names is resolved
/// (`rustyfi_backend:: MathCharClass`/`resolve_variant_char`) once
/// evaluation reaches a value of this type. This TYPE itself
/// (`math-char-class`, nominal, no parameters) is version-blind; its
/// CONSTRUCTOR SET is not — see [`builtin_variants_with_version`]'s
/// `math_char_class_decl` (V0_1 registers 14
/// ctors, V0_0 exactly 9).
fn t_math_char_class() -> MonoType {
    MonoType::Variant("math-char-class".to_string(), Vec::new())
}
/// `paren` — version-forked, the same shape as
/// `t_deco(version)`/`t_graphics_output(version)`/`t_decoset(version)`:
/// - `V0_0` (`pervasives.satyh`'s `type paren`) — structural, like
///   `t_point()`/`t_dash()`/`t_deco()` above: directly the expanded function
///   type, not a nominal reference.
/// - `V0_1` (`primitives.cppo.ml:91`'s `tPAREN`) — args are (inner height,
///   inner depth SIGNED, the context) → (the delimiter boxes, the
///   script-kern function). The 0.0.6→0.1 delta: the closure now extracts
///   fontsize / axis-ratio (via `get-math-axis-height-ratio`) / color FROM
///   the context instead of receiving them as separate explicit arguments —
///   see `make_paren_run` (`primitives.rs`) for the corresponding runtime
///   protocol fork.
///
/// `math-paren`'s first two arguments (`math.satyh`'s
/// `paren-left`/`paren-right`, `\paren`/`\brace`/`\abs`/…) are typed against
/// this shape directly; this port's type-synonym expansion resolves a
/// `paren`-named annotation to the same shape (V0_0: pervasives synonym
/// expansion; V0_1: the `name_to_mono("paren", …)` nominal case,
/// `typecheck.rs`), so what must match is `paren-left`/`paren-right`'s own
/// INFERRED type, which it does by construction. Gated on `math_is_split()`,
/// the same predicate that forks `math-paren` itself, so type-env and
/// runtime stay keyed on one capability.
pub(crate) fn t_paren(version: RustyfiVersion) -> MonoType {
    if version.math_is_split() {
        arrows(
            vec![t_length(), t_length(), t_context()],
            product(vec![t_inline_boxes(), arrow(t_length(), t_length())]),
        )
    } else {
        arrows(
            vec![t_length(), t_length(), t_length(), t_length(), t_color()],
            product(vec![t_inline_boxes(), arrow(t_length(), t_length())]),
        )
    }
}
/// A math-char kern function (fontsize, y-position -> kern amount).
/// `math-char-with-kern`/`math-big-char-with-kern`'s 3rd/4th arguments.
fn t_math_kern_func() -> MonoType {
    arrows(vec![t_length(), t_length()], t_length())
}
/// `math-variant-char`'s 9-field per-style codepoint record (`value.rs`'s
/// `MathVariantStyle`) — a closed record row, structural like
/// `t_pbinfo()`/`t_page_content_scheme()` above. Field order doesn't matter
/// (records are structural), only the label set; matches `math.satyh`'s
/// `greek-lowercase`/`greek-uppercase` record literals field-for-field.
fn t_math_variant_style() -> MonoType {
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
pub(crate) fn t_image() -> MonoType {
    MonoType::Base(BaseType::Image)
}
pub(crate) fn t_inline_boxes() -> MonoType {
    MonoType::Base(BaseType::InlineBoxes)
}
pub(crate) fn t_block_boxes() -> MonoType {
    MonoType::Base(BaseType::BlockBoxes)
}
pub(crate) fn t_context() -> MonoType {
    MonoType::Base(BaseType::Context)
}
pub(crate) fn t_document() -> MonoType {
    MonoType::Base(BaseType::Document)
}
/// `color` (`primitives.cppo.ml:187-190`'s `Gray of float | RGB of
/// (float*float*float) | CMYK of (float*float*float*float)`) — a built-in
/// **variant**, not a `BaseType`: it costs a `VariantDecl` (registered by
/// `builtin_variants_with_version` below) and no `BaseType::Color`. `Gray`/`RGB`/`CMYK`
/// typecheck and evaluate as ordinary `Ast::Ctor`/`Value::Ctor` values;
/// `fill`/`stroke` are its first consumers.
fn t_color() -> MonoType {
    MonoType::Variant("color".to_string(), Vec::new())
}
/// `hyphenation` (dev-0-1-0 `types.cppo.ml`'s opaque `HyphenationType`) —
/// stand-in: NOT a declared `BaseType` or
/// `VariantDecl`, just the nominal-`Variant` fallthrough `name_to_mono`
/// already gives any unrecognized sig type name (`typecheck.rs:500`), the
/// same shape the already-vendored `color.satyh` seals through.
/// `load-hyphenation-dictionary`'s return type and
/// `set-hyphenation-dictionary`'s domain both spell this helper, so sealing
/// subsumption unifies them structurally — no `types.rs` touch needed.
fn t_hyphenation() -> MonoType {
    MonoType::Variant("hyphenation".to_string(), Vec::new())
}
/// `unicode-char-database` (dev-0-1-0 `types.cppo.ml`'s opaque
/// `UnidataType`) — a stand-in, same nominal-`Variant` shape as
/// [`t_hyphenation`] just above.
fn t_unicode_char_database() -> MonoType {
    MonoType::Variant("unicode-char-database".to_string(), Vec::new())
}
/// `pre-path` (vminst.ml's `tPRP`; v0.0.6 `PrePathType`).
pub(crate) fn t_prepath() -> MonoType {
    MonoType::Base(BaseType::PrePath)
}
/// `path` (vminst.ml's `tPATH`; v0.0.6 `PathType`).
pub(crate) fn t_path() -> MonoType {
    MonoType::Base(BaseType::Path)
}
/// `graphics` (vminst.ml's `tGR`; v0.0.6 `GraphicsType`).
pub(crate) fn t_graphics() -> MonoType {
    MonoType::Base(BaseType::Graphics)
}
/// `point = length * length` (vminst.ml's `tPT = tPROD[tLN;tLN]`) —
/// structural, not a `BaseType`: a point is just a 2-tuple of lengths,
/// matching the runtime representation (`Value::Tuple([Length, Length])`,
/// see `primitives.rs`'s `as_point`/`make_point_value`).
fn t_point() -> MonoType {
    product(vec![t_length(), t_length()])
}
/// `page-break-info` (vminst.ml's `tPBINFO`) — a `hook-page-break` closure's
/// first argument. The port has first-class row-typed records
/// (`types::MonoType::Record`) and `#field` access, so this type-checks
/// structurally with no nominal `tPBINFO` variant needed.
/// Runtime: `Value::Record`, built by `fire_hooks` (`lib.rs`).
fn t_pbinfo() -> MonoType {
    MonoType::Record(types::Row::Cons(
        "page-number".to_string(),
        Box::new(t_int()),
        Box::new(types::Row::Empty),
    ))
}
/// `tDOCINFODIC` (dev-0-1-0 `src/frontend/primitives.cppo.ml:98-107`):
/// `register-document-information`'s argument, upstream's named record type
/// `document-information-dictionary`. Structural here, the same `t_pbinfo`
/// precedent above: upstream registers a `SynonymType` name for the
/// identical closed row, which this port deliberately doesn't mirror
/// nominally (cosmetic deviation — revisit only if a 0.1 package names the
/// type in a signature).
fn t_doc_info_dictionary() -> MonoType {
    MonoType::Record(types::Row::Cons(
        "title".to_string(),
        Box::new(t_option(t_string())),
        Box::new(types::Row::Cons(
            "subject".to_string(),
            Box::new(t_option(t_string())),
            Box::new(types::Row::Cons(
                "author".to_string(),
                Box::new(t_option(t_string())),
                Box::new(types::Row::Cons(
                    "keywords".to_string(),
                    Box::new(list(t_string())),
                    Box::new(types::Row::Empty),
                )),
            )),
        )),
    ))
}
/// `page` (vminst.ml's `tPG`) — a nominal variant, `primitives.cppo.ml:203-212`,
/// registered by `builtin_variants_with_version`. `page-break`'s first argument
/// selects the whole document's paper size.
fn t_page() -> MonoType {
    MonoType::Variant("page".to_string(), Vec::new())
}
/// `page-break`/`page-break-multicolumn`/`page-break-two-column`'s
/// first-argument type, forked: `t_page()` (the v0.0.6 9-ctor ADT)
/// under `has_page_adt()`, a plain `length * length` tuple otherwise.
/// `RustyfiVersion::has_page_adt()` is the single source of truth for
/// which shape a given version's `page-break*` family admits —
/// `builtin_variants_with_version` below gates the ADT's own *registration* on the
/// exact same method, so the two can never disagree (a `V0_1` program can
/// never see a type that calls `t_page()` while `page`'s `VariantDecl` is
/// absent from its `builtin_variants_with_version` result).
fn t_page_or_geometry(version: RustyfiVersion) -> MonoType {
    if version.has_page_adt() {
        t_page()
    } else {
        product(vec![t_length(), t_length()])
    }
}
/// `page-content-scheme` (vminst.ml's `tPAGECONT`) — what a `page-break`
/// content-scheme closure returns, applied once per page with that page's
/// `pbinfo`. Structural, like `t_pbinfo` above — no nominal type needed.
fn t_page_content_scheme() -> MonoType {
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
/// `page-parts` (vminst.ml's `tPAGEPARTS`) — what a `page-break`
/// parts-scheme closure returns, applied once per page with that page's
/// `pbinfo`.
fn t_page_parts() -> MonoType {
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
/// (`builtin_variants_with_version`'s `option_decl`) applied to `ty`.
pub(crate) fn t_option(ty: MonoType) -> MonoType {
    MonoType::Variant("option".to_string(), vec![ty])
}
/// `script` (vminst.ml's `tSCR`) — a built-in **variant** (same shape as
/// `t_color()` above): `HanIdeographic | Kana | Latin | OtherScript`
/// (upstream's real surface constructor set, `primitives.cppo.ml:192-196`),
/// registered by `builtin_variants_with_version`. `script-guard` (pervasives.satyh's
/// `\SATySFi`/`\LaTeX`/`\TeX`) is its first consumer.
fn t_script() -> MonoType {
    MonoType::Variant("script".to_string(), Vec::new())
}
/// `language` (vminst.ml's `tLANG`; `charBasis.ml`'s `language_system =
/// Japanese | English | NoLanguageSystem`) — a nullary built-in variant,
/// same shape as [`t_script`]. `set-language`'s 2nd argument
/// (`stdja.satyh`'s `set-language Kana Japanese`).
fn t_language() -> MonoType {
    MonoType::Variant("language".to_string(), Vec::new())
}
/// `text-info` (vminst.ml's `tTCTX`; v0.0.6 `TextInfoType`) — the text-mode
/// context: only the three pure prims below produce/consume it.
fn t_text_info() -> MonoType {
    MonoType::Base(BaseType::TextInfo)
}
/// `paddings` (vminst.ml's `tPADS = tPROD [tLN;tLN;tLN;tLN]`) — a plain
/// 4-tuple `(paddingL, paddingR, paddingT, paddingB)`, matching the runtime
/// shape `primitives.rs`'s `as_paddings` reads (mirrors `evalUtil.ml`'s
/// `get_paddings` field order).
fn t_paddings() -> MonoType {
    product(vec![t_length(), t_length(), t_length(), t_length()])
}
/// `cell` (`primitives.cppo.ml:214-217`) — a built-in **variant** (same
/// shape as `t_color()` above), registered by `builtin_variants_with_version`.
fn t_cell() -> MonoType {
    MonoType::Variant("cell".to_string(), Vec::new())
}
/// `dash` (`graphicD.ml`'s `type dash = length * length * length`) —
/// `dashed-stroke`'s 2nd argument, `(d1, d2, d0)` = on-length, off-length,
/// phase.
fn t_dash() -> MonoType {
    product(vec![t_length(), t_length(), t_length()])
}
/// The result type of a graphics-producing callback (`inline-graphics`'s
/// `tIGR`, `inline-graphics-outer`'s `tIGRO`, `tabular`'s `tRULESF`,
/// `t_deco`'s own result): `list graphics` (`tL tGR`) under `V0_0`, one
/// `graphics` collection (`tGR`) under `V0_1` — the hidden alias-redefinition
/// retype surfaces across every carrier primitive that returns this shape. Runtime counterpart: `coerce_graphics_result`
/// (`primitives.rs`), keyed on the same
/// `RustyfiVersion::graphics_is_collection()` capability so the env and
/// type-env agree by construction.
fn t_graphics_output(version: RustyfiVersion) -> MonoType {
    if version.graphics_is_collection() {
        t_graphics()
    } else {
        list(t_graphics())
    }
}
/// `deco` (vminst.ml's `tDECO_raw` under `V0_0`; dev-0-1-0 redefines the
/// same alias with a bare `tGR` result), invoked (once placed) with
/// its own position and resolved width/height/depth. `inline-frame-outer`'s
/// stand-in body (`primitives.rs`) never actually calls it (see that
/// primitive's doc comment), but it is typed faithfully so callers still
/// type-check exactly as they would upstream.
pub(crate) fn t_deco(version: RustyfiVersion) -> MonoType {
    arrows(
        vec![t_point(), t_length(), t_length(), t_length()],
        t_graphics_output(version),
    )
}
/// `deco-set = deco * deco * deco * deco` (vminst.ml's `tDECOSET`) —
/// `block-frame-breakable`'s third argument (the four edge/corner
/// decoration closures a frame would fire at placement time). STAND-IN
/// body (`primitives.rs`'s `prim_block_frame_breakable`) pops and drops it
/// entirely (like `t_deco()`'s own callers above), but it is typed
/// faithfully here so callers still type-check exactly as they would
/// upstream.
pub(crate) fn t_decoset(version: RustyfiVersion) -> MonoType {
    product(vec![t_deco(version); 4])
}
/// `font` — the V0_1-only OPAQUE face handle (upstream `saphe-split`
/// `primitives.cppo.ml:45`'s `tFONTKEY = (~! "font", BaseType(FontType))`).
/// Value: [`Value::Font`](crate::value::Value::Font), a resolved
/// `rustyfi_backend::FontKey`, exactly upstream's `BCFontKey of FontKey.t`.
///
/// There is deliberately no `V0_0` counterpart: upstream 0.0.6 registers no
/// `font` base type and declares no `type font` in its bundled library, so
/// under `V0_0` the name falls through to the nominal `Variant("font", [])`
/// — see [`BaseType::Font`] for the citations.
pub(crate) fn t_font_key() -> MonoType {
    MonoType::Base(BaseType::Font)
}

/// `font-with-ratio`, i.e. what `set-font`'s second argument actually is,
/// **version-forked at the head component**:
///
/// - `V0_0` — `string * float * float`, upstream `v0.0.6
///   primitives.cppo.ml:69`'s `tFONT = tPROD [tS; tFL; tFL]`. The head is a
///   font ABBREV naming a row of `dist/hash/fonts.satysfi-hash`.
/// - `V0_1` — `font * float * float`, upstream `saphe-split
///   primitives.cppo.ml:74`'s `tFONTWR = tPROD [tFONTKEY; tFL; tFL]`. The
///   head is the opaque [`t_font_key`] handle; the 0.0.6 name is GONE from
///   the surface, and no primitive converts between the two.
///
/// The trailing `(size_ratio, rising_ratio)` pair is identical in both.
fn t_font_with_ratio(version: RustyfiVersion) -> MonoType {
    let head = match version {
        RustyfiVersion::V0_1 => t_font_key(),
        _ => t_string(),
    };
    product(vec![head, t_float(), t_float()])
}

/// `dom -> cod` (vminst.ml's `@->`) — a function taking no labeled optional
/// arguments (`Row::Empty`), so every 0.0.6 primitive/inference site
/// building a `Func` produces the empty-row shape by construction.
pub fn arrow(dom: MonoType, cod: MonoType) -> MonoType {
    MonoType::Func(
        Box::new(crate::types::Row::Empty),
        Box::new(dom),
        Box::new(cod),
    )
}

/// Right-folds [`arrow`] over `doms`, ending in `cod` — for chaining
/// several arguments the way vminst.ml chains `@->`, e.g.
/// `arrows(vec![t_bool(), t_bool(), t_context(), t_inline_boxes()], t_block_boxes())`
/// for `bool -> bool -> context -> inline-boxes -> block-boxes`.
fn arrows(doms: Vec<MonoType>, cod: MonoType) -> MonoType {
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
fn inline_cmd(args: Vec<CmdArgType>) -> MonoType {
    MonoType::InlineCmd(args)
}

/// `mandatory` command-argument entry (0.0.6 positional model; also the
/// V0_1 shape for a slot with no `?(…)` bundle at all — `opt_labels` empty
/// either way).
pub(crate) fn mandatory(ty: MonoType) -> CmdArgType {
    CmdArgType {
        optional: false,
        opt_labels: Vec::new(),
        ty,
    }
}

/// `optional` (`?`) command-argument entry (0.0.6 positional model only —
/// `opt_labels` stays empty; V0_1 never sets `optional: true`, see
/// [`labeled`]).
pub(crate) fn optional(ty: MonoType) -> CmdArgType {
    CmdArgType {
        optional: true,
        opt_labels: Vec::new(),
        ty,
    }
}

/// A SATySFi 0.1 labeled command-argument entry (upstream
/// `CommandArgType(LabelMap.t, typ)`, `types.cppo.ml:214-215`):
/// `ty` is the slot's mandatory argument, `opt_labels` its CLOSED `?(l:τ,…)`
/// bundle (kept sorted by label by the caller — `command_scheme`'s harvest,
/// `lower_type_atom`'s sig lowering). `optional` is always `false`: V0_1 has
/// no whole-slot `ty?` positional-optional model at all.
pub(crate) fn labeled(opt_labels: Vec<(String, MonoType)>, ty: MonoType) -> CmdArgType {
    CmdArgType {
        optional: false,
        opt_labels,
        ty,
    }
}

/// A type scheme with no quantified variables (vminst.ml's `~%` wraps a
/// closed monomorphic body the same way; primitives that actually need
/// polymorphism, like `::`/`!`, use [`poly1`] instead).
fn poly0(ty: MonoType) -> PolyType {
    PolyType::mono(ty)
}

/// A type scheme quantified over exactly one fresh type variable, e.g.
/// `poly1(|a| arrow(reff(a.clone()), a))` for `!`'s `'a ref -> 'a`
/// (vminst.ml's `~@` marks such a per-scheme fresh variable; `~%` then
/// closes the whole thing into a scheme, matching `ptyderef`/`ptycons` in
/// `primitives.cppo.ml:546-547`).
fn poly1<F: FnOnce(MonoType) -> MonoType>(f: F) -> PolyType {
    let v = types::new_ty_var(0);
    let body = f(MonoType::Var(v.clone()));
    PolyType::from_vars(vec![v], Vec::new(), body)
}

// ============================================================================
// The primitive type table.
// ============================================================================

/// Look up the type scheme of a v0.0.6 primitive registered in
/// `primitives.rs`'s `prims!` table (or the separately-defined
/// `inline-fil` constant), by its *source* name (sigil included, e.g.
/// `"\\emph"`, `"+'"`, `"::"`). Back-compat wrapper — see
/// `primitive_type_with_version`'s doc comment.
pub fn primitive_type(name: &str) -> Option<PolyType> {
    primitive_type_with_version(name, RustyfiVersion::V0_0)
}

/// Look up the type scheme of a primitive registered in `primitives.rs`'s
/// `prims!` table, for a given target `version`. Mirrors
/// `primitives::base_env`/`base_env_with_version`'s split (the
/// `lex`/`lex_with_version` idiom).
pub fn primitive_type_with_version(name: &str, version: RustyfiVersion) -> Option<PolyType> {
    Some(match name {
        // ==== removed in 0.1 — guard these OUT of the type table under V0_1
        // before falling through to their ordinary (0.0.6-only-meaningful)
        // arms further below. Runtime availability is the `prims!` table's
        // `v006` tag on the same six names; this guard keeps the two
        // mechanisms in agreement. ====
        "get-axis-height"
        | "math-pull-in-scripts"
        | "math-color"
        | "math-char-class"
        | "math-variant-char"
        | "text-in-math"
            if version.math_is_split() =>
        {
            return None
        }

        // ==== added in 0.1 — unbound under V0_0
        // (falls through this guard to the catch-all `_ => return None`
        // below, since none of these names have a v0.0.6 arm at all). ====
        //
        // dev-0-1-0 vminst.ml:790-793 — REAL, see `primitives.rs`'s `prim_read_math`.
        "read-math" if version.math_is_split() => {
            poly0(arrows(vec![t_context(), t_math_text()], t_math_boxes()))
        }
        // vminst.ml:858 — STAND-IN (out-of-scope text backend, same scoping
        // note as `primitives.rs`'s `prim_convert_string_for_math`);
        // registered so 0.1 packages typecheck.
        "stringify-math" if version.math_is_split() => {
            poly0(arrows(vec![t_text_info(), t_math_text()], t_string()))
        }
        // vminst.ml:59 — REAL: inserts into `Context::math_class_map`.
        "set-math-char" if version.math_is_split() => poly0(arrows(
            vec![t_int(), t_int(), t_math_class(), t_context()],
            t_context(),
        )),
        // vminst.ml:445 — REAL: sets `Context::math_char_class`.
        "set-math-char-class" if version.math_is_split() => {
            poly0(arrows(vec![t_math_char_class(), t_context()], t_context()))
        }
        // vminst.ml:459 — REAL: inverse of `as_math_char_class`.
        "get-math-char-class" if version.math_is_split() => {
            poly0(arrow(t_context(), t_math_char_class()))
        }
        // vminst.ml:432 — REAL data, stand-in render (`MathElement::EmbeddedBoxes`).
        "embed-inline-to-math" if version.math_is_split() => poly0(arrows(
            vec![t_math_class(), t_inline_boxes()],
            t_math_boxes(),
        )),
        // vminst.ml:1305 — REAL: the axis-height ratio `MathC` already scales by.
        "get-math-axis-height-ratio" if version.math_is_split() => {
            poly0(arrow(t_context(), t_float()))
        }
        // `%math-attach-scripts` — hidden (unlexable name, `%` starts a
        // comment), the synthesized script-attacher `val math` commands
        // without `with sub sup` lower to.
        "%math-attach-scripts" if version.math_is_split() => poly0(arrows(
            vec![
                t_context(),
                t_math_boxes(),
                t_option(t_math_text()),
                t_option(t_math_text()),
            ],
            t_math_boxes(),
        )),

        // ==== hyphenation/unidata loader
        // + setter stand-ins, and the `here` lex-time-constant stand-in —
        // all V0_1-only (genuinely absent from 0.0.6 upstream, so these
        // fall through to the catch-all `_ => return None` under V0_0).
        // Types are FAITHFUL to upstream (`vminst.ml`/`primitives.cppo.ml`);
        // bodies (`primitives.rs`) are ACCEPT-AND-RETURN stand-ins, not
        // hard errors like `stringify-math` above, because std-ja
        // *evaluates* `load-unicode-char-database`/`load-hyphenation-
        // dictionary` at module load time. ====
        //
        // `vminst.ml`'s `LoadHyphenationDictionary`.
        "load-hyphenation-dictionary" if version == RustyfiVersion::V0_1 => {
            poly0(arrow(t_string(), t_hyphenation()))
        }
        // `vminst.ml`'s `LoadUnicodeCharDatabase` — args are
        // Scripts.txt/EastAsianWidth.txt/LineBreak.txt paths.
        "load-unicode-char-database" if version == RustyfiVersion::V0_1 => poly0(arrows(
            vec![t_string(), t_string(), t_string()],
            t_unicode_char_database(),
        )),
        // STAND-IN no-op (see `primitives.rs`'s
        // `prim_set_hyphenation_dictionary`); closes a scout-identified gap.
        "set-hyphenation-dictionary" if version == RustyfiVersion::V0_1 => {
            poly0(arrows(vec![t_hyphenation(), t_context()], t_context()))
        }
        // STAND-IN no-op (see `primitives.rs`'s
        // `prim_set_unicode_char_database`); closes a scout-identified gap.
        "set-unicode-char-database" if version == RustyfiVersion::V0_1 => poly0(arrows(
            vec![t_unicode_char_database(), t_context()],
            t_context(),
        )),
        // upstream is a lex-time constant (the source file's directory);
        // the port models it as a V0_1-only nullary constant bound to
        // `Value::Str(String::new())` (`primitives.rs`'s `base_env_with_version`).
        "here" if version == RustyfiVersion::V0_1 => poly0(t_string()),

        // ---- this port's own natives (no vminst.ml entry — local signatures) ----
        //
        // vminst.ml:834, `HorzLex`.
        "read-inline" => poly0(arrow(t_context(), arrow(t_inline_text(), t_inline_boxes()))),
        // vminst.ml:857, `VertLex`.
        "read-block" => poly0(arrow(t_context(), arrow(t_block_text(), t_block_boxes()))),
        // vminst.ml:1003, `BackendLineBreaking`.
        "line-break" => poly0(arrows(
            vec![t_bool(), t_bool(), t_context(), t_inline_boxes()],
            t_block_boxes(),
        )),
        // vminst.ml:1024, `BackendPageBreaking` — the real 4-arg primitive.
        "page-break" => poly0(arrows(
            vec![
                t_page_or_geometry(version),
                arrow(t_pbinfo(), t_page_content_scheme()),
                arrow(t_pbinfo(), t_page_parts()),
                t_block_boxes(),
            ],
            t_document(),
        )),
        // vminst.ml:1065 `BackendPageBreakingMultiColumn` — FAITHFUL, see
        // `primitives.rs`'s `prim_page_break_multicolumn` / `page_break_core`.
        "page-break-multicolumn" => poly0(arrows(
            vec![
                t_page_or_geometry(version),
                list(t_length()),
                arrow(t_unit(), t_block_boxes()),
                arrow(t_unit(), t_block_boxes()),
                arrow(t_pbinfo(), t_page_content_scheme()),
                arrow(t_pbinfo(), t_page_parts()),
                t_block_boxes(),
            ],
            t_document(),
        )),
        // vminst.ml:1041 `BackendPageBreakingTwoColumn`.
        "page-break-two-column" => poly0(arrows(
            vec![
                t_page_or_geometry(version),
                t_length(),
                arrow(t_unit(), t_block_boxes()),
                arrow(t_pbinfo(), t_page_content_scheme()),
                arrow(t_pbinfo(), t_page_parts()),
                t_block_boxes(),
            ],
            t_document(),
        )),
        // ---- int arithmetic ----
        // vminst.ml:2537 `Plus`.
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
        // vminst.ml:2569 `EqualTo`.
        "==" => poly0(arrows(vec![t_int(), t_int()], t_bool())),
        // LOCAL (primitives.cppo.ml's `general_table` derives this as
        // `LogicalNot (EqualTo ..)`, not its own vminst.ml instruction).
        "<>" => poly0(arrows(vec![t_int(), t_int()], t_bool())),
        // vminst.ml:2601 `LessThan`.
        "<" => poly0(arrows(vec![t_int(), t_int()], t_bool())),
        // vminst.ml:2585 `GreaterThan`.
        ">" => poly0(arrows(vec![t_int(), t_int()], t_bool())),
        // LOCAL (derives as `LogicalNot (GreaterThan ..)`).
        "<=" => poly0(arrows(vec![t_int(), t_int()], t_bool())),
        // LOCAL (derives as `LogicalNot (LessThan ..)`).
        ">=" => poly0(arrows(vec![t_int(), t_int()], t_bool())),

        // ---- bool ----
        // vminst.ml:2617 `LogicalAnd`.
        "&&" => poly0(arrows(vec![t_bool(), t_bool()], t_bool())),
        // vminst.ml:2633 `LogicalOr`.
        "||" => poly0(arrows(vec![t_bool(), t_bool()], t_bool())),
        // vminst.ml:2649 `LogicalNot`.
        "not" => poly0(arrow(t_bool(), t_bool())),

        // ---- float ----
        // vminst.ml:2664 `FloatPlus`.
        "+." => poly0(arrows(vec![t_float(), t_float()], t_float())),
        // vminst.ml:2680 `FloatMinus`.
        "-." => poly0(arrows(vec![t_float(), t_float()], t_float())),
        // vminst.ml:2696 `FloatTimes`.
        "*." => poly0(arrows(vec![t_float(), t_float()], t_float())),
        // vminst.ml:2712 `FloatDivides`.
        "/." => poly0(arrows(vec![t_float(), t_float()], t_float())),
        // vminst.ml:2333 `PrimitiveFloat`.
        "float" => poly0(arrow(t_int(), t_float())),
        // vminst.ml:2348 `PrimitiveRound` — despite the name this truncates
        // toward zero (see primitives.rs's `prim_round`).
        "round" => poly0(arrow(t_float(), t_int())),

        // ---- length ----
        // vminst.ml:2894 `LengthPlus`.
        "+'" => poly0(arrows(vec![t_length(), t_length()], t_length())),
        // vminst.ml:2910 `LengthMinus`.
        "-'" => poly0(arrows(vec![t_length(), t_length()], t_length())),
        // vminst.ml:2926 `LengthTimes`.
        "*'" => poly0(arrows(vec![t_length(), t_float()], t_length())),
        // vminst.ml:2942 `LengthDivides`.
        "/'" => poly0(arrows(vec![t_length(), t_length()], t_float())),
        // vminst.ml:2958 `LengthLessThan`.
        "<'" => poly0(arrows(vec![t_length(), t_length()], t_bool())),
        // vminst.ml:2974 `LengthGreaterThan`.
        ">'" => poly0(arrows(vec![t_length(), t_length()], t_bool())),

        // ---- string ----
        // vminst.ml:22 `Concat`.
        "^" => poly0(arrows(vec![t_string(), t_string()], t_string())),
        // vminst.ml:2303 `PrimitiveArabic`.
        "arabic" => poly0(arrow(t_int(), t_string())),
        // vminst.ml:2085 `PrimitiveSame`.
        "string-same" => poly0(arrows(vec![t_string(), t_string()], t_bool())),
        // vminst.ml:2143 `PrimitiveStringLength`.
        "string-length" => poly0(arrow(t_string(), t_int())),
        // vminst.ml:2101 `PrimitiveStringSub`.
        "string-sub" => poly0(arrows(vec![t_string(), t_int(), t_int()], t_string())),
        // vminst.ml:2212 `PrimitiveStringExplode`.
        "string-explode" => poly0(arrow(t_string(), list(t_int()))),
        // vminst.ml `PrimitiveRegExpOfString`/`PrimitiveStringMatch`. The
        // port models `regexp` as its underlying pattern `string` (see
        // `primitives.rs`); `satysfi-base`'s `char.satyg` only ever builds
        // character-class patterns (`[0-9]`, `[A-Za-z]`, …).
        "regexp-of-string" => poly0(arrow(t_string(), t_string())),
        "string-match" => poly0(arrows(vec![t_string(), t_string()], t_bool())),
        // vminstdef.yaml:1961 `PrimitiveStringScan`:
        // `~% (tRE @-> tS @-> tOPT (tPROD [tS; tS]))` — the matched prefix and
        // the remainder. `satysfi-code-printer`'s lexer is built on it.
        "string-scan" => poly0(arrows(
            vec![t_string(), t_string()],
            t_option(product(vec![t_string(), t_string()])),
        )),
        // vminst.ml `PrimitiveSplitOnRegExp` — split points paired with the
        // segment between them (`satysfi-base`/figbox split a path on `\.`).
        "split-on-regexp" => poly0(arrows(
            vec![t_string(), t_string()],
            list(product(vec![t_int(), t_string()])),
        )),

        // ---- list cons ----
        // primitives.cppo.ml:547's `ptycons`. Not its own vminst.ml
        // instruction upstream (`::` desugars to `ListCons` at parse time
        // there); here it's a first-class primitive (see the `prims!`
        // table's comment on `"::"` in primitives.rs).
        "::" => poly1(|a| arrow(a.clone(), arrow(list(a.clone()), list(a)))),

        // ---- mutable-cell dereference ----
        // primitives.cppo.ml:546's `ptyderef`.
        "!" => poly1(|a| arrow(reff(a.clone()), a)),

        // ---- text embedding ----
        // vminst.ml:1706 `PrimitiveEmbed`.
        "embed-string" => poly0(arrow(t_string(), t_inline_text())),

        // ---- context ops ----
        // vminst.ml:1434 `PrimitiveSetFontSize`.
        "set-font-size" => poly0(arrow(t_length(), arrow(t_context(), t_context()))),
        // vminst.ml:1449 `PrimitiveGetFontSize`.
        "get-font-size" => poly0(arrow(t_context(), t_length())),
        // vminst.ml:1633 `PrimitiveSetLeading` (see primitives.rs's `prims!`
        // table comment on why this, and not `set-min-gap-of-lines`, is the
        // baseline-distance setter).
        "set-leading" => poly0(arrow(t_length(), arrow(t_context(), t_context()))),
        // vminst.ml:1396 `PrimitiveSetParagraphMargin`.
        "set-paragraph-margin" => poly0(arrows(
            vec![t_length(), t_length(), t_context()],
            t_context(),
        )),
        // vminst.ml:1648 `PrimitiveGetTextWidth`.
        "get-text-width" => poly0(arrow(t_context(), t_length())),
        // vminst.ml:1247 `PrimitiveGetInitialContext` — the second argument
        // is a `[math] inline-cmd` (`MonoType::InlineCmd` with one mandatory
        // `math` argument), NOT `MathCmd` (`MathCommandType`, the different
        // v0.0.6 type used for math-mode commands like `\sqrt`). Call sites
        // pass `(command \math)` — or a local stub command — to build the
        // first-class command reference this needs; the runtime
        // side is FAITHFUL, see `primitives.rs`'s
        // `prim_get_initial_context`.
        "get-initial-context" => poly0(arrow(
            t_length(),
            arrow(inline_cmd(vec![mandatory(t_math_text())]), t_context()),
        )),

        // ---- context ops, continued (a LOCAL, non-upstream primitive; see primitives.rs's `prims!` table
        // comment on `"set-font-key"` for why it exists) --------------------
        "set-font-key" => poly0(arrow(t_int(), arrow(t_context(), t_context()))),

        // ---- box combinators ----
        // vminst.ml:803 `HorzConcat`.
        "++" => poly0(arrows(
            vec![t_inline_boxes(), t_inline_boxes()],
            t_inline_boxes(),
        )),
        // vminst.ml:818 `VertConcat`.
        "+++" => poly0(arrows(
            vec![t_block_boxes(), t_block_boxes()],
            t_block_boxes(),
        )),
        // No vminst.ml entry — see `base_env`'s comment on `inline-nil`/
        // `block-nil` in primitives.rs (the empty-boxes value that v0.0.6
        // gets for free from `{}`/`<>` literal syntax).
        "inline-nil" => poly0(t_inline_boxes()),
        "block-nil" => poly0(t_block_boxes()),
        // vminst.ml:1757 `BackendFixedEmpty`.
        "inline-skip" => poly0(arrow(t_length(), t_inline_boxes())),
        // vminst.ml:1771 `BackendOuterEmpty`.
        "inline-glue" => poly0(arrows(
            vec![t_length(), t_length(), t_length()],
            t_inline_boxes(),
        )),
        // vminst.ml:1171 `BackendVertSkip`.
        "block-skip" => poly0(arrow(t_length(), t_block_boxes())),

        // ---- the reflow
        // marker-box constructors — no vminst.ml entry (NEW, not a port);
        // see `primitives.rs`'s `prim_list_mark`/`prim_inline_mark` doc
        // comments for the `int` tag encoding. Both produce an INERT marker
        // box, stripped with zero contribution before PDF/faithful-HTML
        // placement — read only by the reflow HTML walker. ----
        "list-mark" => poly0(arrow(t_int(), t_block_boxes())),
        "inline-mark" => poly0(arrow(t_int(), t_inline_boxes())),

        // ---- images (raster images, mirroring v0.0.6 vminstdef.yaml:540/:554) ----
        "load-image" => poly0(arrow(t_string(), t_image())),
        "use-image-by-width" => poly0(arrows(vec![t_image(), t_length()], t_inline_boxes())),
        // v0.0.6 vminstdef.yaml:525 — path + 1-based page number.
        "load-pdf-image" => poly0(arrows(vec![t_string(), t_int()], t_image())),

        // ---- inline-fil ----
        // Not a primitive *function* at all (`base_env` binds it directly
        // to a constant `Value::InlineBoxes`, primitives.rs), so there is
        // no vminst.ml `~type_:` to cite; its type is simply that of the
        // value it names.
        "inline-fil" => poly0(t_inline_boxes()),
        // primitives.cppo.ml:567 — same shape as `inline-fil` above (a bare
        // constant, STAND-IN body; see `primitives.rs`'s `base_env` comment).
        "omit-skip-after" => poly0(t_inline_boxes()),
        // primitives.cppo.ml:569 — same shape as `inline-fil`/`omit-skip-
        // after` above: a bare constant (`base_env` binds it to
        // `Value::BlockBoxes(vec![VertBox::ClearPage])`), FAITHFUL —
        // `mitou-report.satyh`'s `document` unblocker.
        "clear-page" => poly0(t_block_boxes()),

        // ---- the ~18 pure primitives ----------------------------
        // (`|>` is excluded here on purpose — it is elaborated directly to
        // ordinary `Apply`, never a `scope`/env-bound name, so it has no
        // primitive type scheme at all; see `elaborate.rs`'s `climb`.)

        // vminst.ml:2729/2744/2759/2774/2789/2804 `FloatSine`/`FloatArcSine`/
        // `FloatCosine`/`FloatArcCosine`/`FloatTangent`/`FloatArcTangent`.
        "sin" => poly0(arrow(t_float(), t_float())),
        "asin" => poly0(arrow(t_float(), t_float())),
        "cos" => poly0(arrow(t_float(), t_float())),
        "acos" => poly0(arrow(t_float(), t_float())),
        "tan" => poly0(arrow(t_float(), t_float())),
        "atan" => poly0(arrow(t_float(), t_float())),
        // vminst.ml:2819 `FloatArcTangent2`, params `(flt1, flt2)` in that
        // order, so `flt1.atan2(flt2)`.
        "atan2" => poly0(arrows(vec![t_float(), t_float()], t_float())),
        // vminst.ml:2835 `FloatLogarithm`: natural log, not log10.
        "log" => poly0(arrow(t_float(), t_float())),
        // vminst.ml:2850 `FloatExponential`.
        "exp" => poly0(arrow(t_float(), t_float())),
        // vminst.ml:2865/2880 `PrimitiveCeil`/`PrimitiveFloor` — result is
        // `float`, not `int` (contrast `round`, above).
        "ceil" => poly0(arrow(t_float(), t_float())),
        "floor" => poly0(arrow(t_float(), t_float())),
        // vminst.ml:2319 `PrimitiveShowFloat`.
        "show-float" => poly0(arrow(t_float(), t_string())),

        // vminst.ml:2159 `PrimitiveStringByteLength`.
        "string-byte-length" => poly0(arrow(t_string(), t_int())),
        // vminst.ml:2123 `PrimitiveStringSubBytes`.
        "string-sub-bytes" => poly0(arrows(vec![t_string(), t_int(), t_int()], t_string())),
        // vminst.ml:2196 `PrimitiveStringUnexplode`.
        "string-unexplode" => poly0(arrow(list(t_int()), t_string())),

        // vminst.ml:2056 `PrimitiveDisplayMessage`.
        "display-message" => poly0(arrow(t_string(), t_unit())),
        // vminst.ml:3133 `AbortWithMessage` — a fresh-per-instantiation type
        // variable (same pattern as `!`/`::`'s `poly1` above). ZERO-EDIT
        // row: dev-0-1-0's vminst.ml entry differs only in notation (`let
        // bid = …` vs `forall "a"`), not in the type — both generations are
        // the identical `∀a. string -> a`, so this arm serves both versions
        // unchanged.
        "abort-with-message" => poly1(|a| arrow(t_string(), a)),

        // ==== graphics primitives — paths, fill/stroke, and the `inline-graphics` on-page sink.
        // Argument order transcribed from `tools/gencode/vminst.ml`:
        // `start-path` :713, `line-to` :727, `terminate-path` :759,
        // `close-with-line` :773, `fill` :2398, `stroke` :2381,
        // `inline-graphics` :1872. ====================================
        //
        "start-path" => poly0(arrow(t_point(), t_prepath())),
        "line-to" => poly0(arrows(vec![t_point(), t_prepath()], t_prepath())),
        // finishes an OPEN subpath.
        "terminate-path" => poly0(arrow(t_prepath(), t_path())),
        // closes with a straight segment back to the subpath's start.
        "close-with-line" => poly0(arrow(t_prepath(), t_path())),
        // even-odd filled region.
        "fill" => poly0(arrows(vec![t_color(), t_path()], t_graphics())),
        "stroke" => poly0(arrows(vec![t_length(), t_color(), t_path()], t_graphics())),
        // a box of size (w, h, d) carrying the callback's graphics, the
        // minimal on-page sink for `graphics` values (`primitives.rs`'s body
        // notes an eager-callback-at-origin caveat this signature doesn't
        // capture). The callback's RESULT forks per version via
        // `t_graphics_output`; row stays untagged (`Both`) — see
        // `coerce_graphics_result`'s doc comment for why.
        "inline-graphics" => poly0(arrows(
            vec![
                t_length(),
                t_length(),
                t_length(),
                arrow(t_point(), t_graphics_output(version)),
            ],
            t_inline_boxes(),
        )),

        // v0.0.6 vminst.ml:539 (`tRULESF` at primitives.cppo.ml:141);
        // dev-0-1-0 inlines the same shape with a bare `tGR` result,
        // vminst.ml:487-489 — the ruled-grid primitive; same
        // per-version callback-result fork as `inline-graphics`.
        "tabular" => poly0(arrows(
            vec![
                list(list(t_cell())),
                arrows(
                    vec![list(t_length()), list(t_length())],
                    t_graphics_output(version),
                ),
            ],
            t_inline_boxes(),
        )),

        // vminst.ml:1891 `BackendInlineGraphicsOuter` — the callback's args
        // are (the resolved width, then the placed point). Same
        // per-version callback-result fork.
        "inline-graphics-outer" => poly0(arrows(
            vec![
                t_length(),
                t_length(),
                arrows(vec![t_length(), t_point()], t_graphics_output(version)),
            ],
            t_inline_boxes(),
        )),

        // ==== gr.satyh prims — signatures from `tools/gencode/vminst.ml`:
        // `bezier-to` :742, `close-with-bezier` :787, `shift-path` :663,
        // `linear-transform-path` :678, `shift-graphics` :2451,
        // `linear-transform-graphics` :2432, `get-graphics-bbox` :2466,
        // `dashed-stroke` :2414, `draw-text` :2363. ====================
        //
        "bezier-to" => poly0(arrows(
            vec![t_point(), t_point(), t_point(), t_prepath()],
            t_prepath(),
        )),
        "close-with-bezier" => poly0(arrows(vec![t_point(), t_point(), t_prepath()], t_path())),
        "shift-path" => poly0(arrows(vec![t_point(), t_path()], t_path())),
        "linear-transform-path" => poly0(arrows(
            vec![t_float(), t_float(), t_float(), t_float(), t_path()],
            t_path(),
        )),
        "shift-graphics" => poly0(arrows(vec![t_point(), t_graphics()], t_graphics())),
        // `primitives.rs`'s body notes an eager-vs-upstream's-lazy-`cm`
        // stroke-width caveat this signature doesn't capture.
        "linear-transform-graphics" => poly0(arrows(
            vec![t_float(), t_float(), t_float(), t_float(), t_graphics()],
            t_graphics(),
        )),
        // A version fork: v0.0.6 `graphics -> point * point` (vminst.ml:2466); v0.1
        // `graphics -> option (point * point)` (dev-0-1-0 vminst.ml:2301) —
        // `graphics` is a collection under 0.1, so an empty one legitimately
        // has no bbox.
        "get-graphics-bbox" => {
            let bbox_ty = product(vec![t_point(), t_point()]);
            if version.graphics_is_collection() {
                poly0(arrow(t_graphics(), t_option(bbox_ty)))
            } else {
                poly0(arrow(t_graphics(), bbox_ty))
            }
        }
        // dev-0-1-0 vminst.ml:3119 — v0.1-only, the same mirror-guard
        // idiom as the 0.1-only math rows above.
        "unite-graphics" if version.graphics_is_collection() => {
            poly0(arrow(list(t_graphics()), t_graphics()))
        }
        // dev-0-1-0 vminst.ml:3105 — v0.1-only.
        "clip-graphics-by-path" if version.graphics_is_collection() => {
            poly0(arrows(vec![t_path(), t_graphics()], t_graphics()))
        }
        // vminst.ml:696 `PathGetBoundingBox`.
        "get-path-bbox" => poly0(arrow(t_path(), product(vec![t_point(), t_point()]))),
        // like `stroke` (width first), with the dash pattern inserted next.
        "dashed-stroke" => poly0(arrows(
            vec![t_length(), t_dash(), t_color(), t_path()],
            t_graphics(),
        )),
        // FAITHFUL (`primitives.rs`'s `prim_draw_text`).
        "draw-text" => poly0(arrows(vec![t_point(), t_inline_boxes()], t_graphics())),

        // ==== pervasives.satyh unblockers ====
        //
        // vminst.ml:2020 `PrimitiveGetNaturalMetrics`.
        "get-natural-metrics" => poly0(arrow(
            t_inline_boxes(),
            product(vec![t_length(), t_length(), t_length()]),
        )),
        // vminst.ml:1787 `BackendOuterFrame`. STAND-IN body — see
        // primitives.rs's `prim_inline_frame_outer` doc comment.
        "inline-frame-outer" => poly0(arrows(
            vec![t_paddings(), t_deco(version), t_inline_boxes()],
            t_inline_boxes(),
        )),
        // vminst.ml:1807 `BackendInnerFrame` — same signature as
        // `inline-frame-outer` above.
        "inline-frame-inner" => poly0(arrows(
            vec![t_paddings(), t_deco(version), t_inline_boxes()],
            t_inline_boxes(),
        )),
        // vminst.ml:1661 `PrimitiveSetManualRising`.
        "set-manual-rising" => poly0(arrow(t_length(), arrow(t_context(), t_context()))),
        // vminst.ml:1908 `BackendScriptGuard`. STAND-IN body (identity) —
        // see primitives.rs's `prim_script_guard` doc comment.
        "script-guard" => poly0(arrows(vec![t_script(), t_inline_boxes()], t_inline_boxes())),
        // vminst.ml:1969 `BackendDiscretionary`, params `(pb, hblst0,
        // hblst1, hblst2)`.
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
        // vminst.ml:1739 `PrimitiveGetAxisHeight`. STAND-IN body — see
        // `primitives.rs`'s `prim_get_axis_height` doc comment.
        "get-axis-height" => poly0(arrow(t_context(), t_length())),

        // ==== hooks / annotations / cross-references ====
        //
        // vminstdef.yaml:576.
        "hook-page-break" => poly0(arrow(
            arrows(vec![t_pbinfo(), t_point()], t_unit()),
            t_inline_boxes(),
        )),
        // vminst.ml:632 `BackendHookPageBreakBlock` — the block-level analog
        // of `hook-page-break` above, FAITHFUL: see `primitives.rs`'s
        // `prim_hook_page_break_block`. `stdjareport.satyh`'s `document`
        // unblocker.
        "hook-page-break-block" => poly0(arrow(
            arrows(vec![t_pbinfo(), t_point()], t_unit()),
            t_block_boxes(),
        )),
        // vminstdef.yaml:1793.
        "register-cross-reference" => poly0(arrows(vec![t_string(), t_string()], t_unit())),
        // vminstdef.yaml:1808.
        "get-cross-reference" => poly0(arrow(t_string(), t_option(t_string()))),
        // vminst.ml:3043 `BackendProbeCrossReference` — `get-cross-reference`
        // without the recorded miss. FAITHFUL.
        "probe-cross-reference" => poly0(arrow(t_string(), t_option(t_string()))),

        // ==== annot.satyh's
        // prim surface. STAND-IN bodies — see primitives.rs's
        // `prim_get_leftmost_script`/`prim_inline_frame_breakable`/
        // `prim_register_destination` doc comments. ====
        //
        // vminstdef.yaml:1754/1767.
        "get-leftmost-script" => poly0(arrow(t_inline_boxes(), t_option(t_script()))),
        "get-rightmost-script" => poly0(arrow(t_inline_boxes(), t_option(t_script()))),
        // vminstdef.yaml:1672.
        "inline-frame-breakable" => poly0(arrows(
            vec![t_paddings(), t_decoset(version), t_inline_boxes()],
            t_inline_boxes(),
        )),
        // vminstdef.yaml:2738.
        "register-destination" => poly0(arrows(vec![t_string(), t_point()], t_unit())),
        // vminstdef.yaml:2753/2773.
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

        // ==== the faithful `Value::Math` primitive layer `math.satyh` is
        // built out of. Signatures transcribed from
        // `tools/gencode/vminst.ml` (cited per entry). ====
        //
        // vminst.ml:388 `BackendMathChar`; v0.1 (dev-0-1-0 vminst.ml:358) —
        // ctx ACCEPTED, not stored on the atom.
        "math-char" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_context(), t_math_class(), t_string()],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(vec![t_math_class(), t_string()], t_math()))
            }
        }
        // vminst.ml:405 `BackendMathBigChar` — same shape, large-operator
        // size class (layout does not yet upscale it; it renders the same
        // size as `math-char`). v0.1 (vminst.ml:374): same fork as `math-char`.
        "math-big-char" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_context(), t_math_class(), t_string()],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(vec![t_math_class(), t_string()], t_math()))
            }
        }
        // vminst.ml:422 `BackendMathCharWithKern`; v0.1 (vminst.ml:390):
        // same ctx-prepended fork as `math-char`.
        "math-char-with-kern" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![
                        t_context(),
                        t_math_class(),
                        t_string(),
                        t_math_kern_func(),
                        t_math_kern_func(),
                    ],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(
                    vec![
                        t_math_class(),
                        t_string(),
                        t_math_kern_func(),
                        t_math_kern_func(),
                    ],
                    t_math(),
                ))
            }
        }
        // vminst.ml:445 `BackendMathBigCharWithKern` — same shape. v0.1
        // (vminst.ml:411): same fork as `math-char-with-kern`.
        "math-big-char-with-kern" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![
                        t_context(),
                        t_math_class(),
                        t_string(),
                        t_math_kern_func(),
                        t_math_kern_func(),
                    ],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(
                    vec![
                        t_math_class(),
                        t_string(),
                        t_math_kern_func(),
                        t_math_kern_func(),
                    ],
                    t_math(),
                ))
            }
        }
        // vminst.ml:193 `BackendMathConcat`. v0.1 (vminst.ml:181): same
        // shape, `mb` instead of `math`.
        "math-concat" => {
            if version.math_is_split() {
                poly0(arrows(vec![t_math_boxes(), t_math_boxes()], t_math_boxes()))
            } else {
                poly0(arrows(vec![t_math(), t_math()], t_math()))
            }
        }
        // vminst.ml:209 `BackendMathGroup`. v0.1 (vminst.ml:194): same
        // shape, `mb` instead of `math`.
        "math-group" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_math_class(), t_math_class(), t_math_boxes()],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(
                    vec![t_math_class(), t_math_class(), t_math()],
                    t_math(),
                ))
            }
        }
        // vminst.ml:226 `BackendMathSuperscript`. v0.1 (vminst.ml:208): the
        // script argument is a context-taking callback, evaluated under
        // `enter_script`.
        "math-sup" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_context(), t_math_boxes(), t_math_script_fn()],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(vec![t_math(), t_math()], t_math()))
            }
        }
        // vminst.ml:242 `BackendMathSubscript`. v0.1 (vminst.ml:228): same
        // shape as `math-sup`.
        "math-sub" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_context(), t_math_boxes(), t_math_script_fn()],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(vec![t_math(), t_math()], t_math()))
            }
        }
        // vminst.ml:258 `BackendMathFraction`. v0.1 (vminst.ml:248):
        // ctx prepended, `mb` instead of `math`.
        "math-frac" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_context(), t_math_boxes(), t_math_boxes()],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(vec![t_math(), t_math()], t_math()))
            }
        }
        // vminst.ml:274 `BackendMathRadical`. v0.1 (vminst.ml:262):
        // ctx prepended, `mb` instead of `math`.
        "math-radical" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_context(), t_option(t_math_boxes()), t_math_boxes()],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(vec![t_option(t_math()), t_math()], t_math()))
            }
        }
        // vminst.ml:352 `BackendMathLowerLimit`. v0.1
        // (vminst.ml:338): same script-callback shape as `math-sup`.
        "math-lower" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_context(), t_math_boxes(), t_math_script_fn()],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(vec![t_math(), t_math()], t_math()))
            }
        }
        // vminst.ml:336 `BackendMathUpperLimit`. v0.1
        // (vminst.ml:318): same script-callback shape as `math-sup`.
        "math-upper" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_context(), t_math_boxes(), t_math_script_fn()],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(vec![t_math(), t_math()], t_math()))
            }
        }
        // vminst.ml:368 `BackendMathPullInScripts`.
        "math-pull-in-scripts" => poly0(arrows(
            vec![
                t_math_class(),
                t_math_class(),
                arrows(vec![t_option(t_math()), t_option(t_math())], t_math()),
            ],
            t_math(),
        )),
        // vminst.ml:488 `BackendMathColor`.
        "math-color" => poly0(arrows(vec![t_color(), t_math()], t_math())),
        // vminst.ml:504 `BackendMathCharClass`.
        "math-char-class" => poly0(arrows(vec![t_math_char_class(), t_math()], t_math())),
        // vminst.ml:111 `BackendMathVariantCharDirect`.
        "math-variant-char" => poly0(arrows(
            vec![t_math_class(), t_math_variant_style()],
            t_math(),
        )),
        // No bundled `.satyh` consumer, but v0.0.6-shaped (this arm
        // plus `get-left-math-class`/`get-right-math-class` below, the
        // boundary-class introspection pair). v0.1 (vminst.ml:36): the body
        // applies the selector once per each of the 9 `MathCharClass`
        // values and inserts into `math_variant_char_map` (eager
        // materialization of upstream's stored selector).
        "set-math-variant-char" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_int(), arrow(t_math_char_class(), t_int()), t_context()],
                    t_context(),
                ))
            } else {
                poly0(arrows(
                    vec![t_math_char_class(), t_int(), t_int(), t_context()],
                    t_context(),
                ))
            }
        }
        // v0.1 (vminst.ml:128) — ctx dropped.
        "get-left-math-class" => {
            if version.math_is_split() {
                poly0(arrow(t_math_boxes(), t_option(t_math_class())))
            } else {
                poly0(arrows(
                    vec![t_context(), t_math()],
                    t_option(t_math_class()),
                ))
            }
        }
        // v0.1 (vminst.ml:146): same fork as `get-left-math-class`.
        "get-right-math-class" => {
            if version.math_is_split() {
                poly0(arrow(t_math_boxes(), t_option(t_math_class())))
            } else {
                poly0(arrows(
                    vec![t_context(), t_math()],
                    t_option(t_math_class()),
                ))
            }
        }
        // vminst.ml:294 `BackendMathParen`. v0.1 (vminst.ml:279):
        // ctx prepended.
        "math-paren" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![
                        t_context(),
                        t_paren(version),
                        t_paren(version),
                        t_math_boxes(),
                    ],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(
                    vec![t_paren(version), t_paren(version), t_math()],
                    t_math(),
                ))
            }
        }
        // vminst.ml:314 `BackendMathParenWithMiddle`. v0.1
        // (vminst.ml:297): ctx prepended.
        "math-paren-with-middle" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![
                        t_context(),
                        t_paren(version),
                        t_paren(version),
                        t_paren(version),
                        list(t_math_boxes()),
                    ],
                    t_math_boxes(),
                ))
            } else {
                poly0(arrows(
                    vec![
                        t_paren(version),
                        t_paren(version),
                        t_paren(version),
                        list(t_math()),
                    ],
                    t_math(),
                ))
            }
        }
        // vminst.ml:468 `BackendMathText` (named `text-in-math`).
        "text-in-math" => poly0(arrows(
            vec![t_math_class(), arrow(t_context(), t_inline_boxes())],
            t_math(),
        )),
        // vminst.ml:61 `PrimitiveConvertStringForMath`. STAND-IN
        // body (see `primitives.rs`'s `prim_convert_string_for_math`).
        "convert-string-for-math" => poly0(arrows(
            vec![t_context(), t_math_char_class(), t_string()],
            t_string(),
        )),
        // vminst.ml:520 `BackendEmbeddedMath` (named `embed-math`) — the
        // bridge to the page; `\math` (math.satyh:439) wraps this. v0.1
        // (vminst.ml:472): `as_math_boxes` then the SAME
        // `layout_math_value`, the whole MATH-engine reuse in one primitive.
        "embed-math" => {
            if version.math_is_split() {
                poly0(arrows(vec![t_context(), t_math_boxes()], t_inline_boxes()))
            } else {
                poly0(arrows(vec![t_context(), t_math()], t_inline_boxes()))
            }
        }
        // vminst.ml:77 `PrimitiveSetMathCommand` — installs the default
        // command a bare `${…}`-in-text dispatches to. FAITHFUL, see
        // `primitives.rs`'s `prim_set_math_command`.
        "set-math-command" => poly0(arrow(
            inline_cmd(vec![mandatory(t_math())]),
            arrow(t_context(), t_context()),
        )),
        // `PrimitiveSetMathFont`, version-forked in its FIRST argument:
        // 0.0.6 `vminstdef.yaml:1364` takes the math font's ABBREV (a plain
        // string); saphe-split `tools/gencode/vminst.ml:1462` takes the
        // opaque [`t_font_key`] handle (its body writes
        // `ctx.math_font_key = Some(mathkey)`).
        "set-math-font" => {
            let dom = match version {
                RustyfiVersion::V0_1 => t_font_key(),
                _ => t_string(),
            };
            poly0(arrow(dom, arrow(t_context(), t_context())))
        }
        // LOCAL, non-upstream, V0_1-only — the port's stand-in for
        // upstream's internal `LoadSingleFont{path}` node, which has no
        // surface name upstream. Its argument is the port's font-store key
        // standing in for upstream's font-file path — see `primitives.rs`'s
        // `prim_load_single_font` for why. Same LOCAL-primitive precedent as
        // `set-font-key` (`primitives.rs`'s `prims!` table).
        "load-single-font" if version == RustyfiVersion::V0_1 => {
            poly0(arrow(t_string(), t_font_key()))
        }
        // vminst.ml:173 `BackendSpaceBetweenMaths`. STAND-IN body,
        // used by `math.satyh`'s `+align`. v0.1 (vminst.ml:164): shared
        // body, only the extractor forks.
        "space-between-maths" => {
            if version.math_is_split() {
                poly0(arrows(
                    vec![t_context(), t_math_boxes(), t_math_boxes()],
                    t_option(t_inline_boxes()),
                ))
            } else {
                poly0(arrows(
                    vec![t_context(), t_math(), t_math()],
                    t_option(t_inline_boxes()),
                ))
            }
        }
        // vminst.ml:1677 `PrimitiveRaiseInline` (name inferred from usage,
        // not independently confirmed against a `~name:` line). STAND-IN
        // body — see `primitives.rs`'s `prim_raise_inline` doc comment (no
        // per-box vertical-offset wrapper in the line model yet outside
        // `PureHorzBox::Math`).
        "raise-inline" => poly0(arrows(vec![t_length(), t_inline_boxes()], t_inline_boxes())),
        // vminst.ml:973 `PrimitiveEmbeddedVertBreakable` (named
        // `embed-block-breakable`). STAND-IN body — no nested
        // page-breakable block-in-inline box yet (see
        // `primitives.rs`'s `prim_embed_block_breakable`).
        "embed-block-breakable" => {
            poly0(arrows(vec![t_context(), t_block_boxes()], t_inline_boxes()))
        }
        // `gr.satyh`-adjacent path combinator; `math.satyh`'s `\norm`
        // unions two vertical bars into one path. FAITHFUL: a real path
        // union (concatenation of subpaths — see `primitives.rs`'s
        // `prim_unite_path`).
        "unite-path" => poly0(arrows(vec![t_path(), t_path()], t_path())),
        // vminst.ml:1291 `PrimitiveSetMinGapOfLines` — a *different*
        // context field than `set-leading` (see that primitive's own
        // comment); `math.satyh`'s `+math-list` calls this. STAND-IN body:
        // no separate `min_gap_of_lines` field on `Context` yet, so this is
        // a same-shape passthrough (see `primitives.rs`'s
        // `prim_set_min_gap_of_lines`).
        "set-min-gap-of-lines" => poly0(arrow(t_length(), arrow(t_context(), t_context()))),

        // ==== (rows 1-10): the
        // context-setter + box-combinator prims `code.satyh`/`itemize.satyh`
        // need. Signatures transcribed from `tools/gencode/vminst.ml` (cited
        // per entry). ====
        //
        // vminst.ml:1603 `PrimitiveSetTextColor`. FAITHFUL
        // (`primitives.rs`'s `prim_set_text_color`).
        "set-text-color" => poly0(arrow(t_color(), arrow(t_context(), t_context()))),
        // vminst.ml:1618 `PrimitiveGetTextColor`. FAITHFUL — `itemize.satyh`
        // feeds this straight into `fill`, see `primitives.rs`'s
        // `prim_get_text_color`/`make_color_value`.
        "get-text-color" => poly0(arrow(t_context(), t_color())),
        // vminst.ml:1692 `PrimitiveSetHyphenPenalty`. FAITHFUL store;
        // consumed by `flush_word`'s hyphenation injection when a
        // dictionary is installed.
        "set-hyphen-penalty" => poly0(arrow(t_int(), arrow(t_context(), t_context()))),
        // vminstdef.yaml:1163-1177 `PrimitiveSetHyphenMin`, params
        // `(left_hyphen_min, right_hyphen_min)`.
        "set-hyphen-min" => poly0(arrows(vec![t_int(), t_int(), t_context()], t_context())),
        // vminst.ml:1309 `PrimitiveSetSpaceRatio`, params `(natural, shrink,
        // stretch)`. FAITHFUL: read by `text_to_boxes`'s interword glue.
        "set-space-ratio" => poly0(arrows(
            vec![t_float(), t_float(), t_float(), t_context()],
            t_context(),
        )),
        // vminst.ml's `PrimitiveSetSpaceRatioBetweenScripts`, params
        // `(natural, shrink, stretch, then the two adjacent scripts)`. Used
        // by slydifi's arctic theme. STAND-IN — see `primitives.rs`'s
        // `prim_set_space_ratio_between_scripts` for why the observable
        // output still matches upstream.
        "set-space-ratio-between-scripts" => poly0(arrows(
            vec![
                t_float(),
                t_float(),
                t_float(),
                t_script(),
                t_script(),
                t_context(),
            ],
            t_context(),
        )),
        // vminst.ml:2269 `PrimitiveSplitIntoLines`. FAITHFUL — pure string
        // op, see `primitives.rs`'s `prim_split_into_lines`.
        "split-into-lines" => poly0(arrow(t_string(), list(product(vec![t_int(), t_string()])))),
        // vminst.ml:1090 `PrimitiveBlockFrameBreakable`. STAND-IN:
        // reduced-width + left-indent inner block, `deco-set` dropped —
        // see `primitives.rs`'s `prim_block_frame_breakable`.
        "block-frame-breakable" => poly0(arrows(
            vec![
                t_context(),
                t_paddings(),
                t_decoset(version),
                arrow(t_context(), t_block_boxes()),
            ],
            t_block_boxes(),
        )),
        // vminst.ml:1145 `PrimitiveEmbeddedVertTop` (named `embed-block-top`).
        // STAND-IN: top-aligned `PureHorzBox::EmbeddedBlock` — see
        // `primitives.rs`'s `prim_embed_block_top`.
        "embed-block-top" => poly0(arrows(
            vec![t_context(), t_length(), arrow(t_context(), t_block_boxes())],
            t_inline_boxes(),
        )),
        // vminst.ml:1185 `PrimitiveEmbeddedVertBottom` (named
        // `embed-block-bottom`). Same STAND-IN shape as `embed-block-top`
        // above — see `primitives.rs`'s `prim_embed_block_bottom`.
        "embed-block-bottom" => poly0(arrows(
            vec![t_context(), t_length(), arrow(t_context(), t_block_boxes())],
            t_inline_boxes(),
        )),
        // vminst.ml:1229 `PrimitiveLineStackBottom` (named
        // `line-stack-bottom`). FAITHFUL — see `primitives.rs`'s
        // `prim_line_stack_bottom`.
        "line-stack-bottom" => poly0(arrow(list(t_inline_boxes()), t_inline_boxes())),
        // vminstdef.yaml:1109 `BackendLineStackTop` — same shape as
        // `line-stack-bottom`, differing only in which stacked line's
        // baseline the result carries. FAITHFUL — see `prim_line_stack_top`.
        "line-stack-top" => poly0(arrow(list(t_inline_boxes()), t_inline_boxes())),
        // vminst.ml:1130 `PrimitiveAddFootnote`. FAITHFUL — see
        // primitives.rs's prim_add_footnote (footnote float accumulator).
        "add-footnote" => poly0(arrow(t_block_boxes(), t_inline_boxes())),
        // `PrimitiveSetFont`, version-forked in its SECOND argument only:
        // 0.0.6 `vminstdef.yaml:1335` (`tFONT = string * float * float`);
        // saphe-split `tools/gencode/vminst.ml:1433`
        // (`tFONTWR = font * float * float`). See [`t_font_with_ratio`].
        "set-font" => poly0(arrows(
            vec![t_script(), t_font_with_ratio(version), t_context()],
            t_context(),
        )),
        // 0.0.6 `vminstdef.yaml:1350` — the reader for the slot `set-font`
        // writes, so it forks at the SAME head via the same
        // [`t_font_with_ratio`]. FAITHFUL — see `primitives.rs`'s
        // `prim_get_font_v006`.
        "get-font" => poly0(arrows(
            vec![t_script(), t_context()],
            t_font_with_ratio(version),
        )),
        // `stdja:116`; orphan #4 — not in any vminst.ml table this port has
        // transcribed, so no upstream line is cited. STAND-IN: accepted and
        // dropped, like `set-math-command`/`set-math-font` above — see
        // `primitives.rs`'s `prim_set_code_text_command`.
        "set-code-text-command" => poly0(arrow(
            inline_cmd(vec![mandatory(t_string())]),
            arrow(t_context(), t_context()),
        )),
        // vminst.ml:2040 `PrimitiveGetNaturalLength` — `get-natural-width`'s
        // block sibling. FAITHFUL: block height+depth summed to one length
        // via `measure_block` (rustyfi-backend) — see `primitives.rs`'s
        // `prim_get_natural_length`.
        "get-natural-length" => poly0(arrow(t_block_boxes(), t_length())),

        // ==== the remaining stdja.satyh primitives, not grouped above.
        // `set-dominant-wide-script`/
        // `set-dominant-narrow-script`/`set-language` (rows 15/17/18) are
        // FAITHFUL stores with real getter round-trips just
        // below; `set-every-word-break` is a STAND-IN (accepted, dropped) —
        // see its `primitives.rs` doc comment. ====
        //
        // vminst.ml:1511 `PrimitiveSetDominantWideScript`.
        "set-dominant-wide-script" => poly0(arrow(t_script(), arrow(t_context(), t_context()))),
        // vminst.ml:1539 `PrimitiveSetDominantNarrowScript`: same shape.
        "set-dominant-narrow-script" => poly0(arrow(t_script(), arrow(t_context(), t_context()))),
        // vminst.ml:1568 `PrimitiveSetLangSys`.
        "set-language" => poly0(arrows(
            vec![t_script(), t_language(), t_context()],
            t_context(),
        )),
        // vminst.ml:1526/1555 `PrimitiveGetDominantWideScript`/
        // `...NarrowScript`. FAITHFUL.
        "get-dominant-wide-script" => poly0(arrow(t_context(), t_script())),
        "get-dominant-narrow-script" => poly0(arrow(t_context(), t_script())),
        // vminst.ml:1587 `PrimitiveGetLangSys`.
        "get-language" => poly0(arrows(vec![t_script(), t_context()], t_language())),
        // vminst.ml:3007 `PrimitiveSetEveryWordBreak`.
        "set-every-word-break" => poly0(arrows(
            vec![t_inline_boxes(), t_inline_boxes(), t_context()],
            t_context(),
        )),
        // vminstdef.yaml:2794 `BackendRegisterOutline` — a list of (depth,
        // title, label, is-frozen) PDF-outline entries. FAITHFUL — see
        // `primitives.rs`'s `prim_register_outline`.
        "register-outline" => poly0(arrow(
            list(product(vec![t_int(), t_string(), t_string(), t_bool()])),
            t_unit(),
        )),
        // vminstdef.yaml:1565 `PrimitiveExtract`. FAITHFUL (mirrors
        // `horzBox.ml`'s `extract_string`); see `primitives.rs`'s
        // `extract_string_pure_one`.
        "extract-string" => poly0(arrow(t_inline_boxes(), t_string())),

        // ==== (text-mode-context sliver): the three PURE text-info prims. The text/html backends
        // (`stringify-inline`/`stringify-block`, `.satyh-text` loading) are
        // deliberately out of scope for this PDF port — see
        // `primitives.rs`'s section comment. ====
        //
        // `get-initial-text-info` — a version fork (the
        // `t_page_or_geometry`-style version branch, inlined since it's
        // one row): v0.0.6 (vminst.ml:953 `TextGetInitialTextModeContext`)
        // takes unit; v0.1 (dev-0-1-0 vminst.ml:906) threads the text-mode
        // default math command (`inline [math-text]`) + a math-scripts
        // stringifier into `tctxsub`. Both bodies are the same STAND-IN
        // (`primitives.rs`'s `prim_get_initial_text_info_v01`).
        "get-initial-text-info" => {
            if version == RustyfiVersion::V0_1 {
                poly0(arrows(
                    vec![
                        inline_cmd(vec![mandatory(t_math_text())]),
                        arrows(
                            vec![t_string(), t_option(t_string()), t_option(t_string())],
                            t_string(),
                        ),
                    ],
                    t_text_info(),
                ))
            } else {
                poly0(arrow(t_unit(), t_text_info()))
            }
        }
        // vminst.ml:921 `TextDeepenIndent`.
        "deepen-indent" => poly0(arrows(vec![t_int(), t_text_info()], t_text_info())),
        // vminst.ml:935 `TextBreak`.
        "break" => poly0(arrow(t_text_info(), t_string())),

        // ==== 10 new v0.1-only rows, unbound under V0_0 (the same
        // mirror-guard idiom as the 0.1-only math rows). ====
        //
        // Bitwise ops (dev-0-1-0 vminst.ml :2495/:2477/:2527/:2541/:2513/
        // :2556) — `<<`/`>>` lex as ordinary opsymbol-run identifiers under
        // BOTH versions (`primitives.rs`'s `prims!` table comment), so only
        // the type table's V0_1 guard decides whether they resolve.
        "<<" | ">>" | "band" | "bor" | "bxor" if version == RustyfiVersion::V0_1 => {
            poly0(arrows(vec![t_int(), t_int()], t_int()))
        }
        "bnot" if version == RustyfiVersion::V0_1 => poly0(arrow(t_int(), t_int())),
        // Unicode string ops (dev-0-1-0 vminst.ml :2050/:2066/:2082) — REAL,
        // `primitives.rs`'s `prim_normalize_string_to_nfc`/`_nfd`/
        // `prim_split_grapheme_cluster`.
        "normalize-string-to-nfc" | "normalize-string-to-nfd"
            if version == RustyfiVersion::V0_1 =>
        {
            poly0(arrow(t_string(), t_string()))
        }
        "split-grapheme-cluster" if version == RustyfiVersion::V0_1 => {
            poly0(arrow(t_string(), list(t_string())))
        }
        // dev-0-1-0 vminst.ml:3073 — REAL, `primitives.rs`'s `prim_read_file`.
        //
        // Bound under BOTH generations, unlike its neighbours here. It was
        // added on the 0.0.6 DEV line, not in 0.1: `satysfi-code-printer`
        // 1.1.1 calls it from `+file-printer` while its own opam pins
        // `satysfi { >= "0.0.6-53-g2867e4d9" & < "0.1" }`, which is direct
        // evidence that a 0.0.6-generation compiler has it. Gating it to
        // `V0_1` made the whole package fail to load under 0.0.6 — an
        // `unbound variable 'read-file'` from a module body, so not even
        // reachable-code-dependent.
        "read-file" => poly0(arrow(t_string(), list(t_string()))),
        // dev-0-1-0 vminst.ml:2978 — REAL, `primitives.rs`'s
        // `prim_register_document_information`.
        "register-document-information" if version == RustyfiVersion::V0_1 => {
            poly0(arrow(t_doc_info_dictionary(), t_unit()))
        }

        // ---- language-completeness sweep: 0.1 float comparisons
        // (`primitives.rs`'s `prims!` table comment on ">."/"<."/">=."/
        // "<=." for the upstream citation + the confirmation these are
        // genuinely absent from 0.0.6) — unbound under V0_0.
        ">." | "<." | ">=." | "<=." if version == RustyfiVersion::V0_1 => {
            poly0(arrows(vec![t_float(), t_float()], t_bool()))
        }

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
/// anything directly. `VariantDecl::instantiate_ctor` is the only
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
    pub(crate) fn instantiate_ctor(
        &self,
        ctor: &str,
        args: &[MonoType],
    ) -> Option<(Option<MonoType>, MonoType)> {
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
/// v0.0.6 gives *every* constructor a payload type, using `tU` (unit)
/// for `None`'s "no real payload" case; this port's `Ast::Ctor`/
/// `Pattern::Ctor` (ast.rs) instead represent a nullary constructor as
/// `None` (the Rust `Option`, not the SATySFi one!) directly, so `None`'s
/// declared payload here is `Option::None`, not `Some(unit)`.
///
/// Takes the target `version` explicitly; mirrors the `base_env`/
/// `primitive_type` split above.
pub fn builtin_variants_with_version(version: RustyfiVersion) -> Vec<VariantDecl> {
    let option_param = types::new_ty_var(0);
    let option_decl = VariantDecl {
        name: "option".to_string(),
        params: 1,
        ctors: vec![
            ("None".to_string(), None),
            (
                "Some".to_string(),
                Some(MonoType::Var(option_param.clone())),
            ),
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

    // `color` — nullary variant, `primitives.cppo.ml:187-190`.
    // Unblocks `color.satyh`'s `Color.rgb`/`Color.gray`/`Color.cmyk`
    // constructor wrappers; `fill`/`stroke` also consume it.
    let color_decl = VariantDecl {
        name: "color".to_string(),
        params: 0,
        ctors: vec![
            ("Gray".to_string(), Some(t_float())),
            (
                "RGB".to_string(),
                Some(product(vec![t_float(), t_float(), t_float()])),
            ),
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
    // Japanese`), stored per script in `Context::langsys_scheme` and read
    // back by `get-language` (primitives.rs).
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

    // `page` — nullary variant, the exact constructor set at
    // `primitives.cppo.ml:204-212`: 8 nullary paper-size constants plus
    // `UserDefinedPaper`. `page-break`'s first argument; `as_page`
    // (`primitives.rs`) maps each ctor to a backend `PaperSize`.
    //
    // GONE in v0.1 upstream (no replacement ADT — paper sizes are a
    // plain `length * length` tuple there, see `t_page_or_geometry`), so
    // this declaration is gated on
    // `has_page_adt()` below rather than being unconditionally registered.
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

    // `cell` — nullary variant, transcribed from `primitives.cppo.ml:214-217`.
    // `EmptyCell`'s payload is `None` (this port's nullary-constructor
    // spelling, see this fn's doc comment), matching upstream's `Poly(tU)`
    // "no real payload" case.
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

    // `math-class` — nullary variant, transcribed from
    // `primitives.cppo.ml:162-170`. **Distinct** from `math-char-class`
    // below (the styling variant) — do not conflate.
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

    // `math-char-class` — nullary variant. Constructor set is
    // version-dependent: `v0.0.6` upstream has
    // exactly these 9 (`v0.0.6:src/backend/horzBox.ml:147-158`'s exact set,
    // literally "TEMPORARY; should add more"); dev-0-1-0 widens
    // `math_char_class` 9 → 14 (`b836d512:src/backend/horzBox.ml:98-113`),
    // adding `MathSansSerif`/`MathBoldSansSerif`/`MathItalicSansSerif`/
    // `MathBoldItalicSansSerif`/`MathTypewriter`. Needed for `math.satyh`'s
    // `\mathrm`/`\mathbf`/`\mathcal`/`\mathfrak`/`\mathbb`/`\bm`/`\mathsf`/
    // `\mathtt` to type-check (each applies `math-char-class` to one of
    // these). Gated on `math_is_split()` so the frozen 0.0.6 surface never
    // learns the 5 new names (unknown-constructor error preserved,
    // `typecheck.rs:2257/2347`).
    let mut math_char_class_ctors = vec![
        ("MathItalic".to_string(), None),
        ("MathBoldItalic".to_string(), None),
        ("MathRoman".to_string(), None),
        ("MathBoldRoman".to_string(), None),
        ("MathScript".to_string(), None),
        ("MathBoldScript".to_string(), None),
        ("MathFraktur".to_string(), None),
        ("MathBoldFraktur".to_string(), None),
        ("MathDoubleStruck".to_string(), None),
    ];
    if version.math_is_split() {
        math_char_class_ctors.extend([
            ("MathSansSerif".to_string(), None),
            ("MathBoldSansSerif".to_string(), None),
            ("MathItalicSansSerif".to_string(), None),
            ("MathBoldItalicSansSerif".to_string(), None),
            ("MathTypewriter".to_string(), None),
        ]);
    }
    let math_char_class_decl = VariantDecl {
        name: "math-char-class".to_string(),
        params: 0,
        ctors: math_char_class_ctors,
        param_vars: Vec::new(),
    };

    let mut decls = vec![
        option_decl,
        itemize_decl,
        color_decl,
        script_decl,
        language_decl,
        cell_decl,
        math_class_decl,
        math_char_class_decl,
    ];
    if version.has_page_adt() {
        decls.push(page_decl);
    }
    decls
}
