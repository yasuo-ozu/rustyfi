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

use crate::ast::{BText, IText, MathElem};
use crate::eval::{available_fields, eval_error, DecoEntry, EvalError, Interp};
use crate::value::{DocumentValue, Env, TextInfo, Value};
use std::collections::BTreeMap;
use satysfi_backend::{
    break_into_lines, break_opportunities, chop_page, default_math_variant_char, fit_cell,
    graphics_bbox, linear_transform_graphics, linear_transform_path, measure_block,
    natural_metrics, path_bbox, place_block_at, shift_graphics, shift_path, Annot, AnnotAction,
    BreakKind, Cell, Closing, Color, Context, Dash, DecoId, DocExtras, FontKey, GraphicsElem,
    GraphicsFnId, HookId, HorzBox, HorzStringInfo, ImageId, ImageResource, Language, Length,
    MathCharClass, MathConstants, MathCorner, MathGlyph, MathKind, NamedDest, Paddings, Page,
    PageGeometry, OutlineEntry, PaperSize, Path, PathSeg, Point, PrePath, PureHorzBox, Script,
    ScriptFont, Subpath, TabularBox, VertBox, VertVariantPolicy, FORCED_BREAK_PENALTY,
};
use satysfi_backend::char_script;
use std::rc::Rc;
use std::sync::Arc;

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
    // `page -> (pbinfo -> page-content-scheme) -> (pbinfo -> page-parts) ->
    // block-boxes -> document` (vminst.ml:1024, `BackendPageBreaking`);
    // docs/plans/document-page-model.md §Slice 1.
    "page-break" (4) => prim_page_break;
    // `page -> length list -> (unit -> block-boxes) -> (unit ->
    // block-boxes) -> (pbinfo -> page-content-scheme) -> (pbinfo ->
    // page-parts) -> block-boxes -> document` (vminst.ml:1065
    // `BackendPageBreakingMultiColumn`) — FAITHFUL, see
    // `prim_page_break_multicolumn`'s doc comment and `page_break_core`
    // (pageBreak.ml:726 `main_multicolumn`).
    "page-break-multicolumn" (7) => prim_page_break_multicolumn;
    // `page -> length -> (unit -> block-boxes) -> (pbinfo ->
    // page-content-scheme) -> (pbinfo -> page-parts) -> block-boxes ->
    // document` (vminst.ml:1041 `BackendPageBreakingTwoColumn`) — sugar for
    // a two-column `MultiColumn` (vminst.ml:1062), see
    // `prim_page_break_two_column`.
    "page-break-two-column" (6) => prim_page_break_two_column;

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
    // embedded directly in inline text). FAITHFUL: the second argument is
    // interned via `Interp::register_math_command` and installed as
    // `Context::math_command`, consulted by `read_inline`'s `EmbedMath` arm.
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
    // ---- images (Slice 1: raster images; docs/plans/math-images.md).
    // Mirrors v0.0.6 vminstdef.yaml:540/:554; `load-pdf-image`
    // (vminstdef.yaml:525) is deferred. ------------------------------------
    "load-image"          (1) => prim_load_image;         // string -> image
    "use-image-by-width"  (2) => prim_use_image_by_width; // image -> length -> inline-boxes
    // ==== Slice 1 graphics primitives (docs/plans/graphics-subsystem.md) ====
    // Paths, fill/stroke, and the `inline-graphics` on-page sink. Argument
    // order transcribed from `tools/gencode/vminst.ml`: `start-path` :713,
    // `line-to` :727, `terminate-path` :759, `close-with-line` :773,
    // `fill` :2398, `stroke` :2381, `inline-graphics` :1872.
    "start-path" (1) => prim_start_path;
    "line-to" (2) => prim_line_to;
    "terminate-path" (1) => prim_terminate_path;
    "close-with-line" (1) => prim_close_with_line;
    "fill" (2) => prim_fill;
    "stroke" (3) => prim_stroke;
    "inline-graphics" (4) => prim_inline_graphics;
    // `tabular : (cell list) list -> (length list -> length list ->
    // graphics list) -> inline-boxes` (vminst.ml:539);
    // docs/plans/table-subsystem.md §Slice 1.
    "tabular" (2) => prim_tabular;
    // `inline-graphics-outer : length -> length -> (length -> point ->
    // graphics list) -> inline-boxes` (vminst.ml:1891
    // `BackendInlineGraphicsOuter`) — roadmap C2.
    "inline-graphics-outer" (3) => prim_inline_graphics_outer;
    // ---- gr.satyh roadmap prims (docs/plans/graphics-subsystem.md §Full
    // roadmap A/B/C/D) — see that plan + tools/gencode/vminst.ml for exact
    // signatures: `bezier-to` :742, `close-with-bezier` :787, `shift-path`
    // :663, `linear-transform-path` :678, `shift-graphics` :2451,
    // `linear-transform-graphics` :2432, `get-graphics-bbox` :2466,
    // `get-path-bbox` :696, `dashed-stroke` :2414, `draw-text` :2363
    // (roadmap C1: FAITHFUL, see `prim_draw_text`'s body).
    "bezier-to" (4) => prim_bezier_to;
    "close-with-bezier" (3) => prim_close_with_bezier;
    "shift-path" (2) => prim_shift_path;
    "linear-transform-path" (5) => prim_linear_transform_path;
    "shift-graphics" (2) => prim_shift_graphics;
    "linear-transform-graphics" (5) => prim_linear_transform_graphics;
    "get-graphics-bbox" (1) => prim_get_graphics_bbox;
    "get-path-bbox" (1) => prim_get_path_bbox;
    "dashed-stroke" (4) => prim_dashed_stroke;
    "draw-text" (2) => prim_draw_text;

    // ==== pervasives.satyh unblockers (docs/plans/stdlib-port.md) ====
    // The 5 primitives `lib-satysfi/dist/packages/pervasives.satyh` calls
    // that this port didn't already have; everything else it uses
    // (`read-inline`, `line-break`, `inline-skip`, `set-font-size`,
    // `get-font-size`, `get-text-width`, `inline-nil`, `inline-fil`, `++`,
    // ...) was already registered above. Argument order transcribed from
    // `tools/gencode/vminst.ml`: `get-natural-metrics` :2020,
    // `inline-frame-outer` :1787, `set-manual-rising` :1661,
    // `script-guard` :1908, `discretionary` :1969.
    "get-natural-metrics" (1) => prim_get_natural_metrics;
    "inline-frame-outer" (3) => prim_inline_frame_outer;
    // vminst.ml:1807 `BackendInnerFrame`: same `tPADS @-> tDECO @-> tIB @->
    // tIB` as `inline-frame-outer`.
    "inline-frame-inner" (3) => prim_inline_frame_inner;
    "set-manual-rising" (2) => prim_set_manual_rising;
    "script-guard" (2) => prim_script_guard;
    "discretionary" (4) => prim_discretionary;

    // ==== Tier-2 decoration/graphics packages (deco/hdecoset/vdecoset/
    // picture.satyh) — the only genuinely-missing primitive among them.
    // `get-axis-height` :1739 `PrimitiveGetAxisHeight` — STAND-IN, see body.
    "get-axis-height" (1) => prim_get_axis_height;

    // ==== docs/plans/hooks-annotations-crossref.md §Slice 1: the
    // page-break-hook callback seam + cross-reference fixpoint ====
    "hook-page-break" (1) => prim_hook_page_break;
    "hook-page-break-block" (1) => prim_hook_page_break_block;
    "register-cross-reference" (2) => prim_register_cross_reference;
    "get-cross-reference" (1) => prim_get_cross_reference;
    "probe-cross-reference" (1) => prim_probe_cross_reference;

    // ==== docs/plans/hooks-annotations-crossref.md §B/§D: `annot.satyh`'s
    // prim surface (link annotations + the frame/script stand-ins it
    // needs to type-check) ====
    "get-leftmost-script" (1) => prim_get_leftmost_script;
    "get-rightmost-script" (1) => prim_get_rightmost_script;
    "inline-frame-breakable" (3) => prim_inline_frame_breakable;
    "register-destination" (2) => prim_register_destination;
    "register-link-to-uri" (6) => prim_register_link_to_uri;
    "register-link-to-location" (6) => prim_register_link_to_location;

    // ==== docs/plans/math-engine.md §A + §G: the faithful `Value::Math`
    // primitive layer `math.satyh` is built out of ====
    "math-char" (2) => prim_math_char;
    "math-big-char" (2) => prim_math_big_char;
    "math-char-with-kern" (4) => prim_math_char_with_kern;
    "math-big-char-with-kern" (4) => prim_math_big_char_with_kern;
    "math-concat" (2) => prim_math_concat;
    "math-group" (3) => prim_math_group;
    "math-sup" (2) => prim_math_sup;
    "math-sub" (2) => prim_math_sub;
    "math-frac" (2) => prim_math_frac;
    "math-radical" (2) => prim_math_radical;
    "math-lower" (2) => prim_math_lower;
    "math-upper" (2) => prim_math_upper;
    "math-pull-in-scripts" (3) => prim_math_pull_in_scripts;
    "math-color" (2) => prim_math_color;
    "math-char-class" (2) => prim_math_char_class;
    "math-variant-char" (2) => prim_math_variant_char;
    // ==== gap 7 (`docs/plans/math-mode-language-gaps.md`): the
    // `set-math-variant-char`/`get-left-math-class`/`get-right-math-class`
    // trio no bundled `.satyh` consumer needed yet, built on gap 5's
    // `Context::math_variant_char_map` + `VariantCharPending`. ====
    "set-math-variant-char" (4) => prim_set_math_variant_char;
    "get-left-math-class" (2) => prim_get_left_math_class;
    "get-right-math-class" (2) => prim_get_right_math_class;
    "math-paren" (3) => prim_math_paren;
    "math-paren-with-middle" (4) => prim_math_paren_with_middle;
    "text-in-math" (2) => prim_text_in_math;
    "convert-string-for-math" (3) => prim_convert_string_for_math;
    "embed-math" (2) => prim_embed_math;
    "set-math-command" (2) => prim_set_math_command;
    "set-math-font" (2) => prim_set_math_font;
    "space-between-maths" (3) => prim_space_between_maths;
    "raise-inline" (2) => prim_raise_inline;
    "embed-block-breakable" (2) => prim_embed_block_breakable;
    "unite-path" (2) => prim_unite_path;
    "set-min-gap-of-lines" (2) => prim_set_min_gap_of_lines;

    // ==== docs/plans/context-box-prims.md §Slice 1 (rows 1-10): the
    // context-setter + box-combinator prims `code.satyh`/`itemize.satyh`
    // need. Argument order transcribed from `tools/gencode/vminst.ml`:
    // `set-text-color` :1603, `get-text-color` :1618, `set-hyphen-penalty`
    // :1692, `set-space-ratio` :1309, `split-into-lines` :2269,
    // `block-frame-breakable` :1090, `embed-block-top` :1145, `set-font`
    // :1463; `set-code-text-command`/`get-natural-length` are the two
    // orphans from `docs/plans/build-order-to-stdja.md` (not in vminst.ml). ====
    "set-text-color" (2) => prim_set_text_color;
    "get-text-color" (1) => prim_get_text_color;
    "set-hyphen-penalty" (2) => prim_set_hyphen_penalty;
    "set-space-ratio" (4) => prim_set_space_ratio;
    "split-into-lines" (1) => prim_split_into_lines;
    "block-frame-breakable" (4) => prim_block_frame_breakable;
    "embed-block-top" (3) => prim_embed_block_top;
    "set-font" (3) => prim_set_font;
    "set-code-text-command" (2) => prim_set_code_text_command;
    "get-natural-length" (1) => prim_get_natural_length;

    // ==== `docs/plans/build-order-to-stdja.md` step 8/9 orphans:
    // `set-dominant-wide-script`/`set-dominant-narrow-script`/`set-language`
    // are now FAITHFUL stores (context-box-prims.md §C landed, group E2) with
    // real getter round-trips below; `set-every-word-break` and
    // `register-outline` remain STAND-INs (accepted and dropped) — see their
    // doc comments. ====
    "set-dominant-wide-script" (2) => prim_set_dominant_wide_script;
    "set-dominant-narrow-script" (2) => prim_set_dominant_narrow_script;
    "set-language" (3) => prim_set_language;
    "get-dominant-wide-script" (1) => prim_get_dominant_wide_script;
    "get-dominant-narrow-script" (1) => prim_get_dominant_narrow_script;
    "get-language" (2) => prim_get_language;
    "set-every-word-break" (3) => prim_set_every_word_break;
    "register-outline" (1) => prim_register_outline;
    "extract-string" (1) => prim_extract_string;

    // ==== proof.satyh/footnote-scheme.satyh unblockers (tail-prims sweep):
    // `embed-block-bottom` :1185, `line-stack-bottom` :1229 (both
    // `tools/gencode/vminst.ml`), `add-footnote` :1130. ====
    "embed-block-bottom" (3) => prim_embed_block_bottom;
    "line-stack-bottom" (1) => prim_line_stack_bottom;
    "add-footnote" (1) => prim_add_footnote;

    // ==== docs/plans/context-box-prims.md §G (text-mode-context sliver):
    // the three PURE text-info prims — `get-initial-text-info` :953,
    // `deepen-indent` :921, `break` :935 (all tools/gencode/vminst.ml,
    // text-mode). SCOPING: the text/html backends (`stringify-inline`/
    // `stringify-block`, `.satyh-text` loading, `--text-mode` output) are
    // OUT of scope for this PDF port; upstream's only dist consumer is
    // html-base.satyh-html. Registered in the single shared env (upstream
    // keys prims per mode; this port has one env — deliberate, harmless). ====
    "get-initial-text-info" (1) => prim_get_initial_text_info;
    "deepen-indent" (2) => prim_deepen_indent;
    "break" (1) => prim_break;
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
    // `omit-skip-after : inline-boxes` (`primitives.cppo.ml:567`: `("omit-
    // skip-after", ~% tIB, ..)`) — like `inline-fil` above, a bare
    // CONSTANT (no vminst.ml entry, never a function to call), marking
    // `HorzOmitSkipAfter` — a line-breaking hint telling the paragraph
    // breaker to drop the interword glue that would otherwise follow (used
    // at the tail of `math.satyh`'s `\eqn`/`\math-list`/`\align`). STAND-IN:
    // this port's line-breaker has no such marker box yet (a genuinely
    // separate hyphenation/line-break feature, `docs/plans/text-
    // rendering.md` territory, not this plan's), so it's simply the empty
    // `inline-boxes` list — never actually consulted at load time (none of
    // `math.satyh`'s block/inline math wrappers are called by the file
    // itself).
    env.define("omit-skip-after", Value::InlineBoxes(Vec::new()));
    // `clear-page : block-boxes` (`primitives.cppo.ml:569`) — like
    // `inline-fil` above, a bare CONSTANT (no vminst.ml entry): a
    // single-element block-boxes list carrying the `VertBox::ClearPage`
    // marker `chop_page` (satysfi-backend) treats as "end this page here".
    // FAITHFUL — `mitou-report.satyh`'s `document` unblocker.
    env.define("clear-page", Value::BlockBoxes(vec![VertBox::ClearPage]));
    env
}

// ---- argument extractors ------------------------------------------------------

fn as_context(v: Value) -> Result<Context, EvalError> {
    match v {
        Value::Context(c) => Ok(*c),
        other => eval_error(format!("expected a context, got {}", other.type_name())),
    }
}

fn as_text_info(v: Value) -> Result<TextInfo, EvalError> {
    match v {
        Value::TextInfo(t) => Ok(t),
        other => eval_error(format!("expected a text-info, got {}", other.type_name())),
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

fn as_image(v: Value) -> Result<ImageId, EvalError> {
    match v {
        Value::Image(id) => Ok(id),
        other => eval_error(format!("expected image, got {}", other.type_name())),
    }
}

// ---- graphics argument extractors (Slice 1: docs/plans/graphics-
// subsystem.md §2) ---------------------------------------------------------

/// `point` = `Value::Tuple([Length, Length])` (mirrors `evalUtil.ml:228`'s
/// point extraction).
fn as_point(v: Value) -> Result<Point, EvalError> {
    match v {
        Value::Tuple(vs) if vs.len() == 2 => {
            let mut it = vs.into_iter();
            let x = as_length(it.next().unwrap())?;
            let y = as_length(it.next().unwrap())?;
            Ok((x, y))
        }
        other => eval_error(format!(
            "expected a point (length * length), got {}",
            other.type_name()
        )),
    }
}

/// `color` = `Value::Ctor("Gray"|"RGB"|"CMYK", ..)` (mirrors
/// `evalUtil.ml:124`'s `get_color` exactly — `fill`/`stroke` are this
/// extractor's first callers, so a wrong shape here would fail only at draw
/// time; see the plan's Risks section).
fn as_color(v: Value) -> Result<Color, EvalError> {
    match v {
        Value::Ctor(name, payload) => match (name.as_str(), payload.map(|b| *b)) {
            ("Gray", Some(p)) => Ok(Color::Gray(as_float(p)?)),
            ("RGB", Some(Value::Tuple(vs))) if vs.len() == 3 => {
                let mut it = vs.into_iter();
                let r = as_float(it.next().unwrap())?;
                let g = as_float(it.next().unwrap())?;
                let b = as_float(it.next().unwrap())?;
                Ok(Color::Rgb(r, g, b))
            }
            ("CMYK", Some(Value::Tuple(vs))) if vs.len() == 4 => {
                let mut it = vs.into_iter();
                let c = as_float(it.next().unwrap())?;
                let m = as_float(it.next().unwrap())?;
                let y = as_float(it.next().unwrap())?;
                let k = as_float(it.next().unwrap())?;
                Ok(Color::Cmyk(c, m, y, k))
            }
            (other, _) => eval_error(format!(
                "expected a color (Gray/RGB/CMYK), got variant '{other}'"
            )),
        },
        other => eval_error(format!("expected a color, got {}", other.type_name())),
    }
}

/// `script` = nullary `Value::Ctor` (prim_types.rs `script_decl`); mirrors
/// upstream `get_script` (evalUtil.ml:235-241).
fn as_script(v: Value) -> Result<Script, EvalError> {
    match v {
        Value::Ctor(name, None) => match name.as_str() {
            "HanIdeographic" => Ok(Script::HanIdeographic),
            "Kana" => Ok(Script::Kana),
            "Latin" => Ok(Script::Latin),
            "OtherScript" => Ok(Script::OtherScript),
            other => eval_error(format!("expected a script, got variant '{other}'")),
        },
        other => eval_error(format!("expected a script, got {}", other.type_name())),
    }
}

/// Inverse of [`as_script`] (upstream `make_script_value`, evalUtil.ml:244).
fn make_script_value(s: Script) -> Value {
    let name = match s {
        Script::HanIdeographic => "HanIdeographic",
        Script::Kana => "Kana",
        Script::Latin => "Latin",
        Script::OtherScript => "OtherScript",
    };
    Value::Ctor(name.to_string(), None)
}

/// `language` = nullary `Value::Ctor` (prim_types.rs `language_decl`);
/// mirrors upstream `get_language_system` (evalUtil.ml:262).
fn as_language(v: Value) -> Result<Language, EvalError> {
    match v {
        Value::Ctor(name, None) => match name.as_str() {
            "Japanese" => Ok(Language::Japanese),
            "English" => Ok(Language::English),
            "NoLanguageSystem" => Ok(Language::NoLanguageSystem),
            other => eval_error(format!("expected a language, got variant '{other}'")),
        },
        other => eval_error(format!("expected a language, got {}", other.type_name())),
    }
}

/// Inverse of [`as_language`] (upstream `make_language_system_value`).
fn make_language_value(l: Language) -> Value {
    let name = match l {
        Language::Japanese => "Japanese",
        Language::English => "English",
        Language::NoLanguageSystem => "NoLanguageSystem",
    };
    Value::Ctor(name.to_string(), None)
}

/// `page` = `Value::Ctor("A4Paper"|.., None | Some(Tuple[Length;2]))`
/// (docs/plans/document-page-model.md §Slice 1) — `page-break`'s first
/// argument, mapped to the backend's `PaperSize`.
fn as_page(v: Value) -> Result<PaperSize, EvalError> {
    match v {
        Value::Ctor(name, payload) => match (name.as_str(), payload.map(|b| *b)) {
            ("A0Paper", None) => Ok(PaperSize::A0),
            ("A1Paper", None) => Ok(PaperSize::A1),
            ("A2Paper", None) => Ok(PaperSize::A2),
            ("A3Paper", None) => Ok(PaperSize::A3),
            ("A4Paper", None) => Ok(PaperSize::A4),
            ("A5Paper", None) => Ok(PaperSize::A5),
            ("USLetter", None) => Ok(PaperSize::USLetter),
            ("USLegal", None) => Ok(PaperSize::USLegal),
            ("UserDefinedPaper", Some(Value::Tuple(vs))) if vs.len() == 2 => {
                let mut it = vs.into_iter();
                let w = as_length(it.next().unwrap())?;
                let h = as_length(it.next().unwrap())?;
                Ok(PaperSize::UserDefined(w, h))
            }
            (other, _) => eval_error(format!(
                "expected a page (A4Paper/.../UserDefinedPaper), got variant '{other}'"
            )),
        },
        other => eval_error(format!("expected a page, got {}", other.type_name())),
    }
}

/// `paddings` = `Value::Tuple([Length; 4])` in `(paddingL, paddingR,
/// paddingT, paddingB)` order (mirrors `evalUtil.ml`'s `get_paddings`).
/// `inline-frame-outer`'s first argument.
fn as_paddings(v: Value) -> Result<(Length, Length, Length, Length), EvalError> {
    match v {
        Value::Tuple(vs) if vs.len() == 4 => {
            let mut it = vs.into_iter();
            let l = as_length(it.next().unwrap())?;
            let r = as_length(it.next().unwrap())?;
            let t = as_length(it.next().unwrap())?;
            let b = as_length(it.next().unwrap())?;
            Ok((l, r, t, b))
        }
        other => eval_error(format!(
            "expected paddings (length * length * length * length), got {}",
            other.type_name()
        )),
    }
}

/// `cell` = `Value::Ctor("NormalCell"|"EmptyCell"|"MultiCell", ..)` (mirrors
/// `evalUtil.ml:102`'s `get_cell`) — `tabular`'s grid entries;
/// docs/plans/table-subsystem.md §Slice 1.
fn as_cell(v: Value) -> Result<Cell, EvalError> {
    match v {
        Value::Ctor(name, payload) => match (name.as_str(), payload.map(|b| *b)) {
            ("NormalCell", Some(Value::Tuple(vs))) if vs.len() == 2 => {
                let mut it = vs.into_iter();
                let (l, r, t, b) = as_paddings(it.next().unwrap())?;
                let ib = as_inline_boxes(it.next().unwrap())?;
                Ok(Cell::Normal(Paddings { l, r, t, b }, ib))
            }
            ("EmptyCell", None) => Ok(Cell::Empty),
            ("MultiCell", Some(Value::Tuple(vs))) if vs.len() == 4 => {
                let mut it = vs.into_iter();
                let numrow = as_int(it.next().unwrap())?;
                let numcol = as_int(it.next().unwrap())?;
                let (l, r, t, b) = as_paddings(it.next().unwrap())?;
                let ib = as_inline_boxes(it.next().unwrap())?;
                Ok(Cell::Multi(
                    numrow.max(0) as usize,
                    numcol.max(0) as usize,
                    Paddings { l, r, t, b },
                    ib,
                ))
            }
            (other, _) => eval_error(format!(
                "expected a cell (NormalCell/EmptyCell/MultiCell), got variant '{other}'"
            )),
        },
        other => eval_error(format!("expected a cell, got {}", other.type_name())),
    }
}

/// `(cell list) list` — `tabular`'s first argument.
fn as_cell_grid(v: Value) -> Result<Vec<Vec<Cell>>, EvalError> {
    as_list(v)?
        .into_iter()
        .map(|row| -> Result<Vec<Cell>, EvalError> {
            as_list(row)?.into_iter().map(as_cell).collect()
        })
        .collect()
}

fn as_prepath(v: Value) -> Result<PrePath, EvalError> {
    match v {
        Value::PrePath(p) => Ok(p),
        other => eval_error(format!("expected pre-path, got {}", other.type_name())),
    }
}

fn as_path(v: Value) -> Result<Path, EvalError> {
    match v {
        Value::Path(p) => Ok(p),
        other => eval_error(format!("expected path, got {}", other.type_name())),
    }
}

fn as_graphics(v: Value) -> Result<GraphicsElem, EvalError> {
    match v {
        Value::Graphics(g) => Ok(g),
        other => eval_error(format!("expected graphics, got {}", other.type_name())),
    }
}

/// `dash` = `length * length * length` (mirrors `evalUtil.ml`'s `get_tuple3
/// get_length`) — `dashed-stroke`'s 2nd argument, `(d1, d2, d0)` = on-length,
/// off-length, phase.
fn as_dash(v: Value) -> Result<Dash, EvalError> {
    match v {
        Value::Tuple(vs) if vs.len() == 3 => {
            let mut it = vs.into_iter();
            let d1 = as_length(it.next().unwrap())?;
            let d2 = as_length(it.next().unwrap())?;
            let d0 = as_length(it.next().unwrap())?;
            Ok((d1, d2, d0))
        }
        other => eval_error(format!(
            "expected a dash pattern (length * length * length), got {}",
            other.type_name()
        )),
    }
}

/// The inverse of `as_point` (mirrors `evalUtil.ml:228`'s point
/// construction) — used by `inline-graphics` to build the `(0pt, 0pt)`
/// origin its callback is (eagerly) invoked with; see that primitive's doc
/// comment for the shift-covariance caveat this stands in for.
fn make_point_value(pt: Point) -> Value {
    Value::Tuple(vec![Value::Length(pt.0), Value::Length(pt.1)])
}

/// `length list` construction (mirrors `evalUtil.ml:709`) — builds the
/// box-local grid-line coordinates `tabular`'s rule callback is (eagerly)
/// invoked with; see `prim_tabular`'s doc comment.
fn make_length_list(lens: &[Length]) -> Value {
    Value::List(lens.iter().map(|l| Value::Length(*l)).collect())
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
            IText::EmbedMath { elems, .. } => {
                // Upstream: a bare `${…}` in inline text evaluates by
                // applying the context's installed `[math] inline-cmd` to
                // (ctx, the math value) — `apply(cmd, ctx)` then
                // `apply(_, math)`, exactly like `IText::Cmd` above.
                let installed = ctx
                    .math_command
                    .and_then(|id| interp.math_commands.get(id.0).cloned());
                match installed {
                    Some(cmd) => {
                        let v = interp.apply(cmd, Value::Context(Box::new(ctx.clone())))?;
                        let v = interp.apply(
                            v,
                            Value::MathText {
                                elems: Rc::clone(elems),
                                env: env.clone(),
                            },
                        )?;
                        out.extend(as_inline_boxes(v)?);
                    }
                    None => {
                        // No installed command (contexts built by
                        // `Context::initial` directly, i.e. unit tests):
                        // reflect + lay out through the faithful engine so
                        // `\cmd`/`#var` still evaluate — the same machinery
                        // `+math(${…})` uses via `as_math`.
                        let mut atoms = Vec::new();
                        for e in elems.iter() {
                            reflect_math_elem(interp, e, env, &mut atoms)?;
                        }
                        out.push(HorzBox::Pure(layout_math_value(interp, ctx, &atoms)?));
                    }
                }
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

/// UAX#14 (docs/plans/text-rendering.md §3) byte offsets in `text` that are
/// a real, content-driven break candidate: every `break_opportunities`
/// boundary except the one always reported at `text.len()` (the segmenter's
/// "always break at the end of text" convention — an artifact of
/// segmenting this one run in isolation, not a signal about what follows
/// it in the paragraph, since `text_to_boxes` is called once per
/// `IText::Text` leaf and more content may follow via a sibling `Cmd`,
/// `Embed`, or `EmbedMath`).
fn uax14_boundaries(text: &str) -> Vec<Option<BreakKind>> {
    let mut boundary = vec![None; text.len() + 1];
    for (offset, kind) in break_opportunities(text) {
        if offset < text.len() {
            boundary[offset] = Some(kind);
        }
    }
    boundary
}

/// This run's `(font, size, rising)` for `script` (D1b resolution rule, see
/// `Context::font_scheme`'s doc comment): `Latin` reads `ctx.font` itself
/// (NOT `font_scheme[Latin].font`) so `set-font-key`/`\bold`/`\emph` — every
/// primitive that only ever touches `ctx.font` — keep working exactly as
/// before D1, while still picking up `font_scheme[Latin]`'s ratio/rising
/// (written in lockstep by `set-font Latin ..`, see that primitive).
fn script_font(ctx: &Context, script: Script) -> ScriptFont {
    if script == Script::Latin {
        ScriptFont {
            font: ctx.font,
            ..ctx.font_scheme[Script::Latin as usize]
        }
    } else {
        ctx.font_scheme[script as usize]
    }
}

/// Measure `text` (already known to be one script run) at `size` under
/// `font`, falling back per-glyph to `fallback_font` (`ctx.font`) when
/// `font` has no glyph for a character — the "CJK per-glyph metrics path
/// stubbed" case the plan flags: a character within a script-run's bucket
/// that its assigned font happens to lack (e.g. a fullwidth-form character
/// absent from a narrow CJK face) still measures via the Latin default
/// rather than failing the whole run. Errors (both fonts lack the glyph)
/// name the offending character and font key, replacing the old fixed
/// "milestone-1 base fonts (WinAnsi only)" message which no longer applies
/// once a real font store is configured.
///
/// **Known limitation** (documented, not fixed by this slice): the
/// measurement here can fall back per-glyph, but `PureHorzBox::InnerString`
/// carries one `HorzStringInfo::font` for its WHOLE text run — so if a
/// fallback glyph is actually used, the PDF writer's `emit_box` still tries
/// to look it up in `font`'s face at render time and fails there instead.
/// Splitting a run into sub-boxes at the source-font-only/fallback boundary
/// (a faithful fix) is future work; every stdja default face configuration
/// covers its script's whole repertoire, so this path is not expected to
/// trigger in practice.
fn measure_run(
    interp: &Interp,
    font: FontKey,
    fallback_font: FontKey,
    text: &str,
    size: Length,
) -> Result<Length, EvalError> {
    let mut width = Length::ZERO;
    for c in text.chars() {
        let advance = interp
            .metrics
            .advance(font, c, size)
            .or_else(|| interp.metrics.advance(fallback_font, c, size))
            .ok_or_else(|| EvalError {
                span: None,
                msg: format!(
                    "character {c:?} is not available in font {font:?} \
                     (nor the fallback font {fallback_font:?})"
                ),
            })?;
        width += advance;
    }
    Ok(width)
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
    let boundary = uax14_boundaries(text);
    let mut word = String::new();
    let flush_word = |word: &mut String, script: Script, out: &mut Vec<HorzBox>| -> Result<(), EvalError> {
        if word.is_empty() {
            return Ok(());
        }
        let sf = script_font(ctx, script);
        let size = ctx.font_size * sf.ratio;
        let rising = ctx.font_size * sf.rising;
        let width = measure_run(interp, sf.font, ctx.font, word, size)?;
        out.push(HorzBox::Pure(PureHorzBox::InnerString {
            info: HorzStringInfo {
                font: sf.font,
                size,
                rising,
            },
            text: std::mem::take(word),
            width,
            height: interp.metrics.ascender(sf.font, size),
            depth: interp.metrics.descender(sf.font, size),
        }));
        Ok(())
    };
    // `Some(s)` exactly when `word` is non-empty — the script of the run
    // currently being accumulated (D1b: a run also breaks on a script
    // change, not just on whitespace/UAX#14, see `char_script`).
    let mut word_script: Option<Script> = None;
    for (i, c) in text.char_indices() {
        if c == ' ' || c == '\n' {
            if let Some(s) = word_script.take() {
                flush_word(&mut word, s, out)?;
            }
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
            continue;
        }
        let script = char_script(c);
        if let Some(cur) = word_script {
            if cur != script {
                flush_word(&mut word, cur, out)?;
            }
        }
        word_script = Some(script);
        word.push(c);
        // Only non-ASCII text gets UAX#14 discretionaries: plain ASCII
        // stays on exactly today's space/newline-only splitter, so
        // existing Latin fixtures wrap identically (a real, tested
        // divergence otherwise — UAX#14 allows a break after a hyphen,
        // which would fragment e.g. "SATySFi-in-Rust" into three
        // `InnerString`s instead of one, changing the PDF content stream
        // even though the zero-width discretionaries between them render
        // no differently when unchosen). CJK and other non-ASCII scripts
        // have no such existing behavior to preserve, and are exactly
        // where UAX#14 breaking is the whole point (no interword glue at
        // all otherwise, see `is_break_point`'s doc).
        if !c.is_ascii() {
            let after = i + c.len_utf8();
            if let Some(kind) = boundary[after] {
                flush_word(&mut word, script, out)?;
                word_script = None;
                out.push(HorzBox::Pure(PureHorzBox::Discretionary {
                    penalty: match kind {
                        BreakKind::Allowed => 0,
                        BreakKind::Mandatory => FORCED_BREAK_PENALTY,
                    },
                    pre_break: Vec::new(),
                    post_break: Vec::new(),
                    no_break: Vec::new(),
                }));
            }
        }
    }
    match word_script {
        Some(s) => flush_word(&mut word, s, out),
        None => Ok(()),
    }
}

// ---- math conversion (docs/plans/math-engine.md §Slice 1) ----------------------
//
// Walks the already-elaborated `MathElem` tree straight into one
// `PureHorzBox::Math`, fixed-constant shift/scale (no MATH table — see the
// plan's "What Slice 1 deliberately does NOT do").

/// Superscript/subscript size ratio, used ONLY as `MathC`'s fallback when
/// the current math font has no OpenType MATH table (`script_percent_scale_down
/// / 100`, §B1). Not read anywhere outside `MathC` — every layout site goes
/// through `MathC::script_scale`/`sup_shift_clamped`/etc. so a MATH-table
/// font gets the real per-font ratio instead.
const SCRIPT_SCALE: f64 = 0.7;
/// Superscript raise, as a fraction of `ctx.font_size` — `MathC`'s
/// no-MATH-table fallback (roadmap: `superscript_shift_up` clamped per
/// `math.ml:527`, §B1). Not read outside `MathC`.
const SUP_SHIFT: f64 = 0.5;
/// Subscript drop, as a fraction of `ctx.font_size` — `MathC`'s
/// no-MATH-table fallback (roadmap: `subscript_shift_down` per
/// `math.ml:545`, §B1). Not read outside `MathC`.
const SUB_SHIFT: f64 = 0.25;
/// `MathC::frac_numer_shift`'s no-MATH-table fallback: a flat,
/// content-independent numerator raise, as a fraction of the fraction's own
/// LOCAL size (§B2 — mirrors `sup_shift_clamped`'s None-branch style, which
/// also ignores ink extent with no MATH table). Not read outside `MathC`.
const FRAC_NUMER_SHIFT_FALLBACK: f64 = 0.33;
/// `MathC::frac_denom_shift`'s no-MATH-table fallback (mirrors
/// `FRAC_NUMER_SHIFT_FALLBACK`; applied as a downward, i.e. negative, shift
/// by the caller). Not read outside `MathC`.
const FRAC_DENOM_SHIFT_FALLBACK: f64 = 0.33;

/// `docs/plans/math-engine.md` §B1's MATH-table resolver: one query of
/// `interp.metrics.math_constants(font)` per laid-out math run, memoized
/// here so every shift/scale/kern site in that run reads the SAME `Option`
/// instead of re-querying — and so a font with no MATH table (every
/// `Base14Metrics` call, and any TTF that lacks one) transparently falls
/// back to the flat pre-MATH-table constants above, keeping today's
/// fixtures byte-identical. Fields are ratios of the font size (`§1`);
/// callers multiply by whichever size is in scope (`ctx.font_size` for the
/// shift magnitudes — matching the pre-existing "shift doesn't shrink with
/// nesting" behavior these constants always had — or the atom's own local
/// `size` for glyph-relative queries like `script_scale`/kerning).
struct MathC {
    c: Option<MathConstants>,
}

impl MathC {
    fn of(interp: &Interp, font: FontKey) -> Self {
        Self {
            c: interp.metrics.math_constants(font),
        }
    }

    /// Flat, unclamped superscript raise (`math.ml:527`'s `h_supstd` alone,
    /// no `math.ml:524-533` clamp) — the shape `layout_math_atom`'s callers
    /// need when no base/script ink extent is at hand yet.
    fn sup_shift(&self, s: Length) -> Length {
        self.c
            .map(|c| s * c.superscript_shift_up)
            .unwrap_or(s * SUP_SHIFT)
    }

    /// Flat, unclamped subscript drop (mirrors `sup_shift`).
    fn sub_shift(&self, s: Length) -> Length {
        self.c
            .map(|c| s * c.subscript_shift_down)
            .unwrap_or(s * SUB_SHIFT)
    }

    /// `script_percent_scale_down / 100`, or the fixed `SCRIPT_SCALE`
    /// fallback. Nesting-level scale gap (documented in the plan): upstream
    /// switches to `script_script_percent_scale_down` one level deeper;
    /// this port applies `script_scale_down` uniformly at every depth.
    fn script_scale(&self) -> f64 {
        self.c.map(|c| c.script_scale_down).unwrap_or(SCRIPT_SCALE)
    }

    /// `math.ml`'s `h_bar` (axis height): the vertical center math content
    /// (fraction bars, `get-axis-height`) aligns to. Falls back to the same
    /// fixed `0.25` ratio `prim_get_axis_height` used before this slice.
    fn axis(&self, s: Length) -> Length {
        self.c.map(|c| s * c.axis_height).unwrap_or(s * 0.25)
    }

    /// `math.ml:524-533` `superscript_baseline_height`, clamped: the
    /// MAGNITUDE of the upward shift a superscript needs given the base's
    /// own ink height (`h_base`, a positive extent above ITS baseline) and
    /// the superscript's own ink depth (`d_sup`, a positive extent below
    /// ITS baseline — i.e. `MathGlyph.height`/`.depth`, not upstream's
    /// signed `Length.negate`d fields). Falls back to the flat `sup_shift`
    /// (ignoring `h_base`/`d_sup`) when there's no MATH table, so base-14
    /// output is untouched by this clamp.
    fn sup_shift_clamped(&self, s: Length, h_base: Length, d_sup: Length) -> Length {
        match self.c {
            None => self.sup_shift(s),
            Some(c) => {
                let cand1 = s * c.superscript_shift_up;
                let cand2 = h_base - s * c.superscript_baseline_drop_max;
                let cand3 = s * c.superscript_bottom_min + d_sup;
                cand1.max(cand2).max(cand3)
            }
        }
    }

    /// `math.ml:545-553` `subscript_baseline_depth`, clamped: the MAGNITUDE
    /// of the downward shift, given the base's own ink depth (`d_base`) and
    /// the subscript's own ink height (`h_sub`). Mirrors
    /// `sup_shift_clamped`'s fallback behavior.
    fn sub_shift_clamped(&self, s: Length, d_base: Length, h_sub: Length) -> Length {
        match self.c {
            None => self.sub_shift(s),
            Some(c) => {
                let cand1 = s * c.subscript_shift_down;
                let cand2 = d_base + s * c.subscript_baseline_drop_min;
                let cand3 = h_sub - s * c.subscript_top_max;
                cand1.max(cand2).max(cand3)
            }
        }
    }

    /// `math.ml:562-573` `correct_script_baseline_heights`: when a base
    /// carries BOTH a subscript and a superscript, nudge the two
    /// already-clamped shift magnitudes apart so their ink keeps at least
    /// `sub_superscript_gap_min` clearance. `d_sup`/`h_sub` are the same ink
    /// extents `sup_shift_clamped`/`sub_shift_clamped` took; `sup`/`sub` are
    /// their (already clamped) outputs. A no-op when there's no MATH table
    /// — the flat fallback shifts are never additionally corrected, so
    /// base-14 output stays exactly `(sup, sub)`.
    fn correct_script_gap(
        &self,
        s: Length,
        d_sup: Length,
        h_sub: Length,
        sup: Length,
        sub: Length,
    ) -> (Length, Length) {
        let Some(c) = self.c else {
            return (sup, sub);
        };
        let gap_min = s * c.sub_superscript_gap_min;
        let gap = (sup - d_sup) - (h_sub - sub);
        if gap < gap_min {
            let corr = (gap_min - gap) * 0.5;
            (sup + corr, sub + corr)
        } else {
            (sup, sub)
        }
    }

    /// `math.ml:596-602` `upper_limit_baseline_height`, clamped: the
    /// MAGNITUDE of the upward shift for an `\overset`-like upper limit,
    /// given the base's own ink height (`h_base`) and the limit content's
    /// own ink depth (`d_up`). Falls back to the flat `sup_shift` (same
    /// shape upstream's superscript raise uses) with no MATH table.
    fn upper_limit_shift(&self, s: Length, h_base: Length, d_up: Length) -> Length {
        match self.c {
            None => self.sup_shift(s),
            Some(c) => {
                let cand1 = h_base + s * c.upper_limit_baseline_rise_min;
                let cand2 = h_base + s * c.upper_limit_gap_min + d_up;
                cand1.max(cand2)
            }
        }
    }

    /// `math.ml:605-611` `lower_limit_baseline_depth`, clamped: mirrors
    /// `upper_limit_shift` for a lower limit, given the base's own ink
    /// depth (`d_base`) and the limit content's own ink height (`h_low`).
    fn lower_limit_shift(&self, s: Length, d_base: Length, h_low: Length) -> Length {
        match self.c {
            None => self.sub_shift(s),
            Some(c) => {
                let cand1 = d_base + s * c.lower_limit_baseline_drop_min;
                let cand2 = d_base + s * c.lower_limit_gap_min + h_low;
                cand1.max(cand2)
            }
        }
    }

    /// `math.ml:982-991` `horz_fraction_bar`'s rule thickness (also
    /// `radical_bar_metrics`'s `t_bar` — both are "the same generic rule
    /// ratio" in the pre-MATH-table fixed-constant world, §B2):
    /// `fraction_rule_thickness`, or the fixed `0.04` fallback. Multiplied
    /// by the ambient LOCAL nesting `size` (not `ctx.font_size` — a
    /// fraction/radical's own metrics DO shrink with nesting, matching
    /// upstream's `FontInfo.actual_math_font_size`, unlike the sup/sub shift
    /// constants' documented `ctx.font_size` simplification above).
    fn frac_rule(&self, s: Length) -> Length {
        self.c
            .map(|c| s * c.fraction_rule_thickness)
            .unwrap_or(s * 0.04)
    }

    /// `math.ml:574-583` `numerator_baseline_height`, clamped: the
    /// MAGNITUDE of the upward shift a numerator needs given its own ink
    /// depth (`d_numer`, a positive extent below ITS baseline — this port's
    /// convention, see `sup_shift_clamped`'s doc comment; upstream's
    /// `Length.negate d_numer` becomes a plain ADD of `d_numer` here, not a
    /// subtract — getting this sign wrong would shrink the raise for a
    /// deeper numerator instead of growing it, overlapping the bar). Falls
    /// back to a flat, content-independent ratio with no MATH table
    /// (mirrors `sup_shift_clamped`'s None-branch style).
    fn frac_numer_shift(&self, s: Length, d_numer: Length) -> Length {
        match self.c {
            None => s * FRAC_NUMER_SHIFT_FALLBACK,
            Some(c) => {
                let std = s * c.fraction_numer_shift_up;
                let gap = self.axis(s) + self.frac_rule(s) * 0.5 + s * c.fraction_numer_gap_min
                    + d_numer;
                std.max(gap)
            }
        }
    }

    /// `math.ml:585-594` `denominator_baseline_depth`, clamped: mirrors
    /// `frac_numer_shift`. Returns the SIGNED (already-negative) drop the
    /// caller applies straight to `dy` — unlike the sup/sub methods'
    /// positive-magnitude-then-caller-negates convention — because
    /// upstream's own `d_denombl` is signed too, so there's no sign flip to
    /// make here (and `h_denom`, a HEIGHT not a depth, is subtracted
    /// directly, matching upstream's un-negated use of it).
    fn frac_denom_shift(&self, s: Length, h_denom: Length) -> Length {
        match self.c {
            None => -(s * FRAC_DENOM_SHIFT_FALLBACK),
            Some(c) => {
                let std = -(s * c.fraction_denom_shift_down);
                let gap = self.axis(s) - self.frac_rule(s) * 0.5 - s * c.fraction_denom_gap_min
                    - h_denom;
                std.min(gap)
            }
        }
    }

    /// `math.ml:620-626` `radical_bar_metrics`: `(h_bar, t_bar, l_extra)` —
    /// the bar's height above baseline (radicand height + gap, so the bar
    /// always clears the radicand with no separate raise needed), its rule
    /// thickness, and the extra ascender the WHOLE radical run reports
    /// above the bar. Fallback ratios (§B2, no MATH table):
    /// vertical_gap=0.06, rule=0.04 (same fixed ratio `frac_rule` falls back
    /// to), extra_ascender=0.06.
    fn radical_bar_metrics(&self, s: Length, h_cont: Length) -> (Length, Length, Length) {
        match self.c {
            Some(c) => (
                h_cont + s * c.radical_vertical_gap,
                s * c.radical_rule_thickness,
                s * c.radical_extra_ascender,
            ),
            None => (h_cont + s * 0.06, s * 0.04, s * 0.06),
        }
    }
}

/// The ink height/depth of an already-laid-out run, as positive magnitudes
/// (`MathGlyph.dy` is signed, up-positive; `.height`/`.depth` are always
/// non-negative extents from EACH glyph's own local baseline) — the same
/// aggregate `read_math`/`layout_math_value` compute for a whole
/// `PureHorzBox::Math`, reused here per sub-run so `MathC`'s clamp formulas
/// have an `h_base`/`d_sup`/etc to clamp against. Empty input -> `(ZERO,
/// ZERO)` (an empty base/script contributes no clamp pressure).
fn glyphs_extent(glyphs: &[MathGlyph]) -> (Length, Length) {
    let mut height = Length::ZERO;
    let mut depth = Length::ZERO;
    for g in glyphs {
        height = height.max(g.dy + g.height);
        depth = depth.max(g.depth - g.dy);
    }
    (height, depth)
}

/// `glyphs_extent` plus `rules`' own bounding boxes folded in — exactly the
/// aggregate `layout_math_value` computes for a whole `PureHorzBox::Math`
/// (see that function's doc comment on why a bare `Fill`, e.g. a fraction
/// bar/radical sign, needs its own bbox folded in rather than being silently
/// undercounted). §B3b(i) reuses this to size a stretchy delimiter to its
/// enclosed run's REAL ink (glyphs + any drawn rules), not just its glyphs.
fn inner_ink_extent(glyphs: &[MathGlyph], rules: &[GraphicsElem]) -> (Length, Length) {
    let (mut height, mut depth) = glyphs_extent(glyphs);
    for r in rules {
        let ((_, min_y), (_, max_y)) = graphics_bbox(r);
        height = height.max(max_y);
        depth = depth.max(-min_y);
    }
    (height, depth)
}

/// `math.ml:1040-1075`'s superscript kern tuck: the italic correction of
/// the base's TRAILING glyph plus the two corner kerns — the base's
/// top-right sampled at the height the raised superscript's ink starts
/// (`l_base = sup_shift - d_sup`, `superscript_correction_heights`'s first
/// component), and the superscript's own bottom-left sampled (at the
/// superscript's OWN size) at the height the base's ink ends (`l_sup =
/// h_base - sup_shift`, that function's second component) — the extra
/// horizontal gap upstream inserts between a base and a raised superscript
/// so slanted glyphs (an italic integral, say) don't collide with what's
/// stacked above them. `size`/`script_size` are the local sizes the base/
/// script glyphs were actually measured at (NOT `ctx.font_size`, unlike the
/// shift magnitude — these feed a design-units conversion that must match
/// each glyph's own em square). Every lookup misses to `Length::ZERO` (no
/// MATH table, no glyph, no kern data, ...), so base-14 output is
/// untouched: this returns exactly `Length::ZERO` whenever `ctx.math_font`
/// has no MATH table.
#[allow(clippy::too_many_arguments)]
fn superscript_kern(
    interp: &Interp,
    ctx: &Context,
    size: Length,
    script_size: Length,
    base_glyphs: &[MathGlyph],
    script_glyphs: &[MathGlyph],
    sup_shift: Length,
    h_base: Length,
    d_sup: Length,
) -> Length {
    let font = ctx.math_font;
    let last_base = base_glyphs.last().and_then(|g| g.text.chars().last());
    let first_script = script_glyphs.first().and_then(|g| g.text.chars().next());
    let l_italic = last_base
        .and_then(|c| interp.metrics.italic_correction(font, c, size))
        .unwrap_or(Length::ZERO);
    let l_base = sup_shift - d_sup;
    let l_sup = h_base - sup_shift;
    let l_kernbase = last_base
        .and_then(|c| interp.metrics.math_kern(font, c, size, MathCorner::TopRight, l_base))
        .unwrap_or(Length::ZERO);
    let l_kernsup = first_script
        .and_then(|c| {
            interp
                .metrics
                .math_kern(font, c, script_size, MathCorner::BottomLeft, l_sup)
        })
        .unwrap_or(Length::ZERO);
    l_italic + l_kernbase + l_kernsup
}

/// A minimal stand-in for v0.0.6's per-codepoint math-class table
/// (`primitives.cppo.ml`) + `normalize_math_kind` (`math.ml:240`) — just
/// enough for `${a+b}` to get binary-operator spacing. Letters/digits/
/// everything else default to `Ord`.
fn ascii_math_kind(c: char) -> MathKind {
    match c {
        '+' | '-' | '*' | '/' => MathKind::Bin,
        '=' | '<' | '>' => MathKind::Rel,
        ',' | ';' | ':' | '.' => MathKind::Punct,
        _ => MathKind::Ord,
    }
}

/// A deliberately tiny stand-in for `space_between_math_kinds`
/// (`math.ml:319-410`, a 40-pair table driven by context ratios + MATH-table
/// `space_after_script`, roadmap A): a thin space when either neighbor is
/// `Bin`, a thick space when either is `Rel`, none otherwise.
fn space_before(prev: MathKind, cur: MathKind, font_size: Length) -> Length {
    if prev == MathKind::Bin || cur == MathKind::Bin {
        font_size * 0.22
    } else if prev == MathKind::Rel || cur == MathKind::Rel {
        font_size * 0.28
    } else {
        Length::ZERO
    }
}

/// FontKey a math glyph c@size should measure/emit in: dedicated ctx.math_font
/// when it can render c, else text ctx.font. The one place math diverges from
/// text font; the MATH-table slice keys lookups on the same returned FontKey.
fn math_glyph_font(interp: &Interp, ctx: &Context, c: char, size: Length) -> FontKey {
    if interp.metrics.advance(ctx.math_font, c, size).is_some() {
        ctx.math_font
    } else {
        ctx.font
    }
}

/// gap-5 metrics-probe predicate, now math-font-aware.
fn math_char_available(interp: &Interp, ctx: &Context, c: char, size: Length) -> bool {
    interp.metrics.advance(ctx.math_font, c, size).is_some()
        || interp.metrics.advance(ctx.font, c, size).is_some()
}

/// Measure one math character at `size` under `math_glyph_font(ctx, c)` and
/// push it as a `MathGlyph` at the running `*x` (`dy = 0`; callers shift
/// scripts afterward), advancing `*x` past it.
fn push_char_glyph(
    interp: &mut Interp,
    ctx: &Context,
    c: char,
    size: Length,
    out: &mut Vec<MathGlyph>,
    x: &mut Length,
) -> Result<(), EvalError> {
    let font = math_glyph_font(interp, ctx, c, size);
    let advance = interp.metrics.advance(font, c, size).ok_or_else(|| EvalError {
        span: None,
        msg: format!("math character '{c}' is not available in the current math font"),
    })?;
    out.push(MathGlyph {
        info: HorzStringInfo { font, size, rising: Length::ZERO },
        text: c.to_string(),
        gid: None,
        dx: *x,
        dy: Length::ZERO,
        width: advance,
        height: interp.metrics.ascender(font, size),
        depth: interp.metrics.descender(font, size),
    });
    *x += advance;
    Ok(())
}

/// `push_char_glyph`'s big-operator sibling (§B3a): try the v0.0.6 `BigOp`
/// vertical variant (`fontInfo.ml:386-401` — the 2nd `MathVariants` record if
/// present, else the 1st) unconditionally. Upstream's own guard is
/// `is_in_display && is_big`, but `math.ml`'s `convert_math_char` hardcodes
/// `is_in_display = true`, so it reduces to just `is_big` — a big operator
/// grows even inline, even at script size, exactly like upstream; the port
/// tracks no display/inline distinction and needs none here. On any miss (no
/// MATH table, no vertical construction for `c`, or a variant/hmtx/bbox
/// lookup failure — every base-14 call, always) falls back to
/// `push_char_glyph`, byte-identical to pre-§B3 output.
fn push_big_char_glyph(
    interp: &mut Interp,
    ctx: &Context,
    c: char,
    size: Length,
    out: &mut Vec<MathGlyph>,
    x: &mut Length,
) -> Result<(), EvalError> {
    let font = math_glyph_font(interp, ctx, c, size);
    match interp
        .metrics
        .math_vertical_variant(font, c, size, VertVariantPolicy::BigOp)
    {
        Some(v) => {
            out.push(MathGlyph {
                info: HorzStringInfo { font, size, rising: Length::ZERO },
                text: c.to_string(),
                gid: Some(v.gid),
                dx: *x,
                dy: Length::ZERO,
                width: v.advance,
                height: v.height,
                depth: v.depth,
            });
            *x += v.advance;
            Ok(())
        }
        None => push_char_glyph(interp, ctx, c, size, out, x),
    }
}

/// One stretchy-delimiter glyph (§B3b(i)): the smallest `MathVariants`
/// record whose `advance_measurement` covers `target` (else the largest
/// record — `VertVariantPolicy::AtLeast`), centered on the math axis
/// (`dy = axis - (h - d) / 2`; y-**up**, same sign convention as
/// `shift_and_append`'s `dy_shift` — see that function's doc comment on the
/// mirroring trap a flipped sign causes). Falls back to the pre-§B3 baseline
/// base glyph (`push_char_glyph`) when there's no vertical construction,
/// keeping base-14 output identical to before this slice.
fn push_delimiter_glyph(
    interp: &mut Interp,
    ctx: &Context,
    c: char,
    size: Length,
    target: Length,
    axis: Length,
    out: &mut Vec<MathGlyph>,
    x: &mut Length,
) -> Result<(), EvalError> {
    let font = math_glyph_font(interp, ctx, c, size);
    let variant = interp
        .metrics
        .math_vertical_variant(font, c, size, VertVariantPolicy::AtLeast(target));
    // §B (`GlyphAssembly`): if even the largest discrete variant's own ink
    // extent (`height + depth`) still doesn't span `target` — a delimiter
    // taller than anything the font enumerates as a prepared variant — grow
    // it from the assembly parts instead (stack top + repeated extenders +
    // bottom). `None` (no MATH table / no assembly / base-14) leaves the
    // discrete/base path below byte-identical.
    let discrete_covers = variant
        .map(|v| (v.height + v.depth).0 >= target.0)
        .unwrap_or(false);
    if !discrete_covers {
        if let Some(parts) = interp.metrics.math_vertical_assembly(font, c, size, target) {
            if !parts.is_empty() {
                // Horizontal advance of the delimiter column: the largest
                // discrete variant's own hmtx advance when we have one (the
                // parts share the same nominal delimiter width), else the base
                // glyph's advance.
                let hadv = match variant {
                    Some(v) => v.advance,
                    None => interp.metrics.advance(font, c, size).unwrap_or(Length::ZERO),
                };
                // Total vertical extent of the stacked assembly (local, from
                // the bottom part's baseline at 0), then center it on the math
                // axis exactly like the discrete path centers a variant's ink.
                let total = parts
                    .last()
                    .map(|(_, dy, adv)| *dy + *adv)
                    .unwrap_or(Length::ZERO);
                let base_off = axis - total * 0.5;
                for (i, (gid, dy_local, adv)) in parts.iter().enumerate() {
                    out.push(MathGlyph {
                        info: HorzStringInfo { font, size, rising: Length::ZERO },
                        text: c.to_string(),
                        gid: Some(*gid),
                        dx: *x,
                        dy: base_off + *dy_local,
                        // Only the first part carries the column's horizontal
                        // width (all parts are stacked in the SAME x column);
                        // its baseline-relative extent is the part's vertical
                        // advance (up), so `glyphs_extent` folds the whole
                        // stacked column into the box's height/depth.
                        width: if i == 0 { hadv } else { Length::ZERO },
                        height: *adv,
                        depth: Length::ZERO,
                    });
                }
                *x += hadv;
                return Ok(());
            }
        }
    }
    match variant {
        Some(v) => {
            let dy = axis - (v.height - v.depth) * 0.5;
            out.push(MathGlyph {
                info: HorzStringInfo { font, size, rising: Length::ZERO },
                text: c.to_string(),
                gid: Some(v.gid),
                dx: *x,
                dy,
                width: v.advance,
                height: v.height,
                depth: v.depth,
            });
            *x += v.advance;
            Ok(())
        }
        None => push_char_glyph(interp, ctx, c, size, out, x),
    }
}

/// Lay out `elems` in isolation (its own local `x` starting at 0, its own
/// spacing state) at `size` — the shape a `Sup`/`Sub`/`Primes` script needs
/// before its glyphs get re-anchored onto the base's running `x` and
/// shifted by the caller. `size` is the caller's `MathC::script_scale`-
/// derived script size (real MATH-table ratio when available, `SCRIPT_SCALE`
/// otherwise — §B1). Returns the glyphs (still at local coordinates) and
/// the script's total width.
fn layout_script(
    interp: &mut Interp,
    ctx: &Context,
    elems: &[MathElem],
    size: Length,
) -> Result<(Vec<MathGlyph>, Length), EvalError> {
    let mut glyphs = Vec::new();
    let mut x = Length::ZERO;
    let mut last_kind: Option<MathKind> = None;
    for e in elems {
        layout_math_elem(interp, ctx, e, size, &mut glyphs, &mut x, &mut last_kind)?;
    }
    Ok((glyphs, x))
}

/// Re-anchor an isolated script's glyphs (`layout_script`'s output) onto the
/// base's running `*x`, adding `dy_shift` to every glyph's vertical offset —
/// `dy_shift > 0` raises (superscript), `< 0` lowers (subscript). Advances
/// `*x` past the whole script.
fn place_script(
    out: &mut Vec<MathGlyph>,
    x: &mut Length,
    script_glyphs: Vec<MathGlyph>,
    script_width: Length,
    dy_shift: Length,
) {
    let base_x = *x;
    for mut g in script_glyphs {
        g.dx = base_x + g.dx;
        g.dy = g.dy + dy_shift;
        out.push(g);
    }
    *x = base_x + script_width;
}

/// The recursive core of `read_math`: lays out one `MathElem` into `out`,
/// advancing `*x` and threading `*last_kind` (the trailing `MathKind` of
/// whatever was laid out immediately before, for `space_before`) through
/// siblings — the Slice-1 analog of `convert_to_low` + `horz_of_low_math`
/// (`math.ml:753`/`:1016`), fused and with fixed constants.
fn layout_math_elem(
    interp: &mut Interp,
    ctx: &Context,
    elem: &MathElem,
    size: Length,
    out: &mut Vec<MathGlyph>,
    x: &mut Length,
    last_kind: &mut Option<MathKind>,
) -> Result<(), EvalError> {
    match elem {
        MathElem::Chars(s) => {
            for c in s.chars() {
                let kind = ascii_math_kind(c);
                if let Some(prev) = *last_kind {
                    *x += space_before(prev, kind, ctx.font_size);
                }
                push_char_glyph(interp, ctx, c, size, out, x)?;
                *last_kind = Some(kind);
            }
            Ok(())
        }
        MathElem::Group(elems) => {
            for e in elems {
                layout_math_elem(interp, ctx, e, size, out, x, last_kind)?;
            }
            Ok(())
        }
        MathElem::Sup(base, script) => {
            let base_start = out.len();
            layout_math_elem(interp, ctx, base, size, out, x, last_kind)?;
            let mc = MathC::of(interp, ctx.math_font);
            let script_size = ctx.font_size * mc.script_scale();
            let (h_base, _) = glyphs_extent(&out[base_start..]);
            let (script_glyphs, script_width) = layout_script(interp, ctx, script, script_size)?;
            let (_, d_sup) = glyphs_extent(&script_glyphs);
            let sup_shift = mc.sup_shift_clamped(ctx.font_size, h_base, d_sup);
            let kern = superscript_kern(
                interp,
                ctx,
                size,
                script_size,
                &out[base_start..],
                &script_glyphs,
                sup_shift,
                h_base,
                d_sup,
            );
            *x += kern;
            place_script(out, x, script_glyphs, script_width, sup_shift);
            Ok(())
        }
        MathElem::Sub(base, script) => {
            let base_start = out.len();
            layout_math_elem(interp, ctx, base, size, out, x, last_kind)?;
            let mc = MathC::of(interp, ctx.math_font);
            let script_size = ctx.font_size * mc.script_scale();
            let (_, d_base) = glyphs_extent(&out[base_start..]);
            let (script_glyphs, script_width) = layout_script(interp, ctx, script, script_size)?;
            let (h_sub, _) = glyphs_extent(&script_glyphs);
            let sub_shift = mc.sub_shift_clamped(ctx.font_size, d_base, h_sub);
            place_script(out, x, script_glyphs, script_width, -sub_shift);
            Ok(())
        }
        MathElem::Primes(base, n) => {
            let base_start = out.len();
            layout_math_elem(interp, ctx, base, size, out, x, last_kind)?;
            let mc = MathC::of(interp, ctx.math_font);
            let script_size = ctx.font_size * mc.script_scale();
            let (h_base, _) = glyphs_extent(&out[base_start..]);
            // Upstream desugars primes to exactly this: a superscript of `n`
            // U+2032 `′` chars (`parser.mly:1082`).
            let primes = vec![MathElem::Chars("\u{2032}".repeat(*n))];
            let (script_glyphs, script_width) =
                layout_script(interp, ctx, &primes, script_size)?;
            let (_, d_sup) = glyphs_extent(&script_glyphs);
            let sup_shift = mc.sup_shift_clamped(ctx.font_size, h_base, d_sup);
            let kern = superscript_kern(
                interp,
                ctx,
                size,
                script_size,
                &out[base_start..],
                &script_glyphs,
                sup_shift,
                h_base,
                d_sup,
            );
            *x += kern;
            place_script(out, x, script_glyphs, script_width, sup_shift);
            Ok(())
        }
        MathElem::Cmd { name, span, .. } => Err(EvalError {
            span: Some(*span),
            msg: format!("math command `{name}` needs the math package (phase 7 roadmap A)"),
        }),
        MathElem::Embed { span, .. } => Err(EvalError {
            span: Some(*span),
            msg: "embedding a program value in math needs the math package \
                  (phase 7 roadmap A)"
                .into(),
        }),
    }
}

/// Walk an elaborated `${…}` tree (`read_inline`'s `EmbedMath` arm) into one
/// `PureHorzBox::Math`, measuring every glyph through `interp.metrics` at
/// `ctx.font`/`ctx.font_size` — the same `FontMetrics` seam `text_to_boxes`
/// uses. See `docs/plans/math-engine.md` §Slice 1 for the box-model rationale
/// (a math run carries its own pre-shifted sub-glyphs since the line model
/// has no per-box vertical slot).
pub fn read_math(
    interp: &mut Interp,
    ctx: &Context,
    elems: &[MathElem],
) -> Result<PureHorzBox, EvalError> {
    let mut glyphs: Vec<MathGlyph> = Vec::new();
    let mut x = Length::ZERO;
    let mut last_kind: Option<MathKind> = None;
    for e in elems {
        layout_math_elem(interp, ctx, e, ctx.font_size, &mut glyphs, &mut x, &mut last_kind)?;
    }
    let width = x;
    let mut height = Length::ZERO;
    let mut depth = Length::ZERO;
    for g in &glyphs {
        height = height.max(g.dy + g.height);
        depth = depth.max(g.depth - g.dy);
    }
    Ok(PureHorzBox::Math {
        width,
        height,
        depth,
        glyphs,
        rules: Vec::new(),
    })
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
fn prim_line_break(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ib = as_inline_boxes(args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    let _is_breakable_bottom = as_bool(args.pop().unwrap())?;
    let _is_breakable_top = as_bool(args.pop().unwrap())?;
    let mut lines = break_into_lines(&ctx, ib);
    for vb in &mut lines {
        if let VertBox::Line { contents, .. } = vb {
            resolve_outer_graphics_in_contents(interp, contents)?;
        }
    }
    Ok(Value::BlockBoxes(lines))
}

/// Look up `field` in a scheme record's fields, erroring with the
/// available-fields hint (mirrors `evalUtil.ml`'s `report_bug_value` arms
/// for a missing/mistyped scheme field) if it's absent.
fn record_field(
    fields: &BTreeMap<String, Value>,
    record_name: &str,
    field: &str,
) -> Result<Value, EvalError> {
    match fields.get(field) {
        Some(v) => Ok(v.clone()),
        None => eval_error(format!(
            "{record_name} record is missing field '{field}' (available fields: {})",
            available_fields(fields)
        )),
    }
}

/// Extract `(text-origin, text-height)` from a `page-content-scheme`
/// record (`{| text-origin : point; text-height : length |}`) — the direct
/// port of `make_page_content_scheme_func`'s field pull (`evalUtil.ml:558-
/// 565`).
fn read_content_scheme(v: Value) -> Result<(Point, Length), EvalError> {
    let fields = match v {
        Value::Record(m) => m,
        other => {
            return eval_error(format!(
                "a page-content-scheme closure must return a record, got {}",
                other.type_name()
            ))
        }
    };
    let origin = as_point(record_field(&fields, "page-content-scheme", "text-origin")?)?;
    let height = as_length(record_field(&fields, "page-content-scheme", "text-height")?)?;
    Ok((origin, height))
}

/// Extract `(header-origin, header-content, footer-origin, footer-content)`
/// from a `page-parts` record — the direct port of
/// `make_page_parts_scheme_func`'s field pull (`evalUtil.ml:576-595`).
fn read_parts_scheme(v: Value) -> Result<(Point, Vec<VertBox>, Point, Vec<VertBox>), EvalError> {
    let fields = match v {
        Value::Record(m) => m,
        other => {
            return eval_error(format!(
                "a page-parts closure must return a record, got {}",
                other.type_name()
            ))
        }
    };
    let header_origin = as_point(record_field(&fields, "page-parts", "header-origin")?)?;
    let header_content = as_block_boxes(record_field(&fields, "page-parts", "header-content")?)?;
    let footer_origin = as_point(record_field(&fields, "page-parts", "footer-origin")?)?;
    let footer_content = as_block_boxes(record_field(&fields, "page-parts", "footer-content")?)?;
    Ok((header_origin, header_content, footer_origin, footer_content))
}

/// Upstream's `--page-number-limit` default (main.ml:1029). v0.0.6 guards
/// only the multicolumn loop (pageBreak.ml:765, `PageNumberLimitExceeded`);
/// the port guards the shared loop unconditionally — a hook-less run is
/// already bounded by the vbox count (`chop_page`'s progress guarantee), so
/// the guard can only fire when column hooks inject content, exactly the
/// case upstream added it for.
const PAGE_NUMBER_LIMIT: i64 = 10_000;

/// The real 4-arg `page-break` (docs/plans/document-page-model.md §Slice 1)
/// — upstream `BCDocument(pagesize, SingleColumn, (fun () -> []),
/// (fun () -> []), …)` (vminst.ml:1039): one zero-shift column, no hooks.
fn prim_page_break(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let bb = as_block_boxes(args.pop().unwrap())?;
    let pagepartsf = args.pop().unwrap();
    let pagecontf = args.pop().unwrap();
    let paper = as_page(args.pop().unwrap())?;
    page_break_core(
        interp,
        paper,
        vec![Length::ZERO],
        None,
        None,
        pagecontf,
        pagepartsf,
        bb,
    )
}

/// `page-break-two-column : page -> length -> (unit -> block-boxes) ->
/// (pbinfo -> page-content-scheme) -> (pbinfo -> page-parts) ->
/// block-boxes -> document` (vminst.ml:1041 `BackendPageBreakingTwoColumn`)
/// — upstream builds `MultiColumn([origin_shift])` with the user's column
/// hook and a trivial column-end hook (vminst.ml:1062); the `length` is the
/// x-shift of the SECOND column's origin.
fn prim_page_break_two_column(
    interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let bb = as_block_boxes(args.pop().unwrap())?;
    let pagepartsf = args.pop().unwrap();
    let pagecontf = args.pop().unwrap();
    let columnhookf = args.pop().unwrap();
    let origin_shift = as_length(args.pop().unwrap())?;
    let paper = as_page(args.pop().unwrap())?;
    page_break_core(
        interp,
        paper,
        vec![Length::ZERO, origin_shift],
        Some(columnhookf),
        None,
        pagecontf,
        pagepartsf,
        bb,
    )
}

/// `page-break-multicolumn : page -> length list -> (unit -> block-boxes)
/// -> (unit -> block-boxes) -> (pbinfo -> page-content-scheme) -> (pbinfo
/// -> page-parts) -> block-boxes -> document` (vminst.ml:1065
/// `BackendPageBreakingMultiColumn`) — FAITHFUL: the shift list gives
/// columns 2..N's x-origin shifts; upstream prepends `Length.zero` for
/// column 1 (pageBreak.ml:762), so `stdjareport.satyh:403`'s `[]` is a
/// one-column layout whose hooks still fire per column/page.
fn prim_page_break_multicolumn(
    interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let bb = as_block_boxes(args.pop().unwrap())?;
    let pagepartsf = args.pop().unwrap();
    let pagecontf = args.pop().unwrap();
    let columnendhookf = args.pop().unwrap();
    let columnhookf = args.pop().unwrap();
    let mut origin_shifts = vec![Length::ZERO];
    for v in as_list(args.pop().unwrap())? {
        origin_shifts.push(as_length(v)?);
    }
    let paper = as_page(args.pop().unwrap())?;
    page_break_core(
        interp,
        paper,
        origin_shifts,
        Some(columnhookf),
        Some(columnendhookf),
        pagecontf,
        pagepartsf,
        bb,
    )
}

/// Apply a `unit -> block-boxes` column hook and PREPEND its result to the
/// remaining content — the port of `chop_single_column_with_insertion`
/// (pageBreak.ml:699-702; the upstream `normalize` is a no-op here because
/// block-boxes are already solid `Vec<VertBox>`).
fn apply_column_hook(
    interp: &mut Interp,
    hook: &Value,
    remaining: &mut Vec<VertBox>,
) -> Result<(), EvalError> {
    let inserted = as_block_boxes(interp.apply(hook.clone(), Value::Unit)?)?;
    remaining.splice(0..0, inserted);
    Ok(())
}

/// The shared per-page loop backing `page-break`, `page-break-two-column`,
/// and `page-break-multicolumn` — the port of `PageBreak.main` /
/// `main_multicolumn` (pageBreak.ml:705-781). Lang-side because it is the
/// one place that legally holds `&mut Interp` to apply the scheme/hook
/// closures (the `fire_hooks` seam). `origin_shifts` is the FULL column
/// list (leading zero included by the callers); `None` hooks are upstream's
/// `(fun () -> [])`.
///
/// Per page: apply `pagecontf` once; per column: fire `columnhookf`
/// (start of EVERY column, pageBreak.ml:700), chop one column at
/// `(x0 + shift, y0)` (footnotes bottom-place per column inside
/// `chop_page`), stop early when content runs out; then fire
/// `columnendhookf` exactly once (both upstream arms — exhausted
/// mid-columns `:751` and shifts-exhausted `:736` — reduce to "prepend its
/// output to the remainder"); then apply `pagepartsf` and place the parts.
#[allow(clippy::too_many_arguments)]
fn page_break_core(
    interp: &mut Interp,
    paper: PaperSize,
    origin_shifts: Vec<Length>,
    columnhookf: Option<Value>,
    columnendhookf: Option<Value>,
    pagecontf: Value,
    pagepartsf: Value,
    bb: Vec<VertBox>,
) -> Result<Value, EvalError> {
    let (paper_w, paper_h) = paper.dims();

    let mut remaining = bb;
    let mut pages: Vec<Page> = Vec::new();
    let mut pageno: i64 = 1;
    loop {
        if pageno > PAGE_NUMBER_LIMIT {
            return eval_error(format!(
                "page number limit exceeded ({PAGE_NUMBER_LIMIT}); a column hook keeps injecting content"
            ));
        }
        let mut pb_fields = BTreeMap::new();
        pb_fields.insert("page-number".to_string(), Value::Int(pageno));
        let pbinfo = Value::Record(pb_fields);

        // ---- content scheme: this page's text area (applied ONCE per
        // page, shared by all its columns — pageBreak.ml:769) ----
        let sch = interp.apply(pagecontf.clone(), pbinfo.clone())?;
        let (origin, height) = read_content_scheme(sch)?;
        let (x0, y0) = origin;

        // ---- columns ----
        let mut lines = Vec::new();
        for shift in &origin_shifts {
            if let Some(hook) = &columnhookf {
                apply_column_hook(interp, hook, &mut remaining)?;
            }
            lines.extend(chop_page((x0 + *shift, y0), height, &mut remaining));
            if remaining.is_empty() {
                break; // content exhausted: remaining columns are skipped
            }
        }
        if let Some(hook) = &columnendhookf {
            apply_column_hook(interp, hook, &mut remaining)?;
        }

        // ---- parts scheme: this page's header + footer ----
        let parts = interp.apply(pagepartsf.clone(), pbinfo)?;
        let (header_origin, header_content, footer_origin, footer_content) =
            read_parts_scheme(parts)?;
        lines.extend(place_block_at(header_origin, header_content));
        lines.extend(place_block_at(footer_origin, footer_content));

        pages.push(Page { lines });
        if remaining.is_empty() {
            break;
        }
        pageno += 1;
    }

    // Every image `load-image` decoded while evaluating this document (see
    // `Interp::images`'s doc comment) rides along in the packaged
    // `DocumentValue` so the PDF writer can emit XObjects for the ones
    // actually placed on a page.
    let images = interp.images.clone();
    Ok(Value::Document(Rc::new(DocumentValue {
        geometry: PageGeometry::for_paper(paper_w, paper_h),
        pages,
        images,
        // Filled in by `compile_document_cst_with_trials` once `fire_hooks`
        // has walked the final trial's placed geometry (see
        // `DocumentValue::extras`'s doc comment) — hooks/decos haven't fired
        // yet at this point in `page-break`'s own evaluation.
        extras: DocExtras::default(),
    })))
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

/// `get-initial-context : length -> [math] inline-cmd -> context`
/// (vminst.ml `PrimitiveGetInitialContext`) — the second argument is the
/// default math command a bare `${…}` in inline text dispatches to (v0.0.6
/// `context_main.math_command`); interned via
/// `Interp::register_math_command`, carried as `Context::math_command`.
fn prim_get_initial_context(
    interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let cmd = args.pop().unwrap();
    let width = as_length(args.pop().unwrap())?;
    let mut ctx = Context::initial(width);
    ctx.math_command = Some(interp.register_math_command(cmd));
    // D1a/D1b: overlay the configured `default-font.satysfi-hash` `scripts`
    // block, if any, so a bare document with a configured font root renders
    // CJK/etc. with zero `set-font` calls (`interp.metrics.
    // default_script_font` is `None` for every script on a provider with no
    // such config — `Base14Metrics` and a bare `TtfFontStore::load` both —
    // so this loop is a no-op there, keeping every pre-D1 fixture's initial
    // context byte-identical).
    for (idx, script) in [
        Script::HanIdeographic,
        Script::Kana,
        Script::Latin,
        Script::OtherScript,
    ]
    .into_iter()
    .enumerate()
    {
        if let Some((font, ratio, rising)) = interp.metrics.default_script_font(script) {
            ctx.font_scheme[idx] = ScriptFont { font, ratio, rising };
            if script == Script::Latin {
                ctx.font = font;
            }
        }
    }
    Ok(Value::Context(Box::new(ctx)))
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

// ---- images (Slice 1: raster images; docs/plans/math-images.md) -----------

/// `load-image : string -> image` (v0.0.6 vminstdef.yaml:540). Resolves
/// `path` against the process's current working directory: this milestone
/// has no "job directory" threaded through `Interp` yet (that would need a
/// new field plumbed all the way from `compile_document_cst`/`main.rs`,
/// out of scope for this slice), so this is a deliberately simple stand-in
/// for v0.0.6's real job-directory-relative resolution — good enough for a
/// CLI invoked from the document's own directory, and for this crate's own
/// fixture-driven tests, which pass an absolute path.
///
/// Decoding is eager (via the `image` crate, to 8-bit `DeviceRGB` — see
/// `ImageResource`'s doc comment for the alpha-dropping/format caveats),
/// matching v0.0.6's `ImageInfo.add_image` (imageInfo.ml): a missing or
/// undecodable file is a clean `EvalError` at the `load-image` call site
/// itself, not a surprise deferred all the way to the PDF writer.
fn prim_load_image(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let path = as_str(args.pop().unwrap())?;
    let decoded = image::open(&path).map_err(|e| EvalError {
        span: None,
        msg: format!("load-image: cannot decode '{path}': {e}"),
    })?;
    let rgb = decoded.to_rgb8();
    let (px_w, px_h) = rgb.dimensions();
    let id = ImageId(interp.images.len());
    interp.images.push(ImageResource {
        samples: rgb.into_raw(),
        px_w,
        px_h,
    });
    Ok(Value::Image(id))
}

/// `use-image-by-width : image -> length -> inline-boxes` (v0.0.6
/// vminstdef.yaml:554). Computes the on-page height from the source
/// image's own pixel aspect ratio (v0.0.6
/// `ImageInfo.get_height_from_width`, imageInfo.ml:44): `height = width *
/// px_h / px_w`.
fn prim_use_image_by_width(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let width = as_length(args.pop().unwrap())?;
    let image = as_image(args.pop().unwrap())?;
    let resource = interp.images.get(image.0).ok_or_else(|| EvalError {
        span: None,
        msg: format!("internal error: image id {} out of range", image.0),
    })?;
    if resource.px_w == 0 {
        return eval_error("use-image-by-width: image has zero width, cannot scale");
    }
    let height = width * (resource.px_h as f64 / resource.px_w as f64);
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::Image {
            width,
            height,
            image,
        },
    )]))
}

// ============================================================================
// ---- Slice 1 graphics primitives (docs/plans/graphics-subsystem.md) ------
// `start-path`/`line-to`/`terminate-path`/`close-with-line`/`fill`/`stroke`/
// `inline-graphics` — see that plan's §2/§3 for the full design. Argument
// order matches `tools/gencode/vminst.ml` (point-first for `line-to`,
// width-first for `stroke`).
// ============================================================================

/// `start-path : point -> pre-path` (vminst.ml:713).
fn prim_start_path(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let start = as_point(args.pop().unwrap())?;
    Ok(Value::PrePath(PrePath {
        start,
        segs: Vec::new(),
    }))
}

/// `line-to : point -> pre-path -> pre-path` (vminst.ml:727) — appends a
/// straight segment to the pre-path's forward-accumulated `segs`.
fn prim_line_to(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let mut pp = as_prepath(args.pop().unwrap())?;
    let pt = as_point(args.pop().unwrap())?;
    pp.segs.push(PathSeg::Line(pt));
    Ok(Value::PrePath(pp))
}

/// `terminate-path : pre-path -> path` (vminst.ml:759) — finishes an OPEN
/// subpath (no closing segment).
fn prim_terminate_path(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let pp = as_prepath(args.pop().unwrap())?;
    Ok(Value::Path(Path {
        subpaths: vec![Subpath {
            start: pp.start,
            segs: pp.segs,
            closing: Closing::Open,
        }],
    }))
}

/// `close-with-line : pre-path -> path` (vminst.ml:773) — closes the subpath
/// with a straight segment back to its start (PDF `h`).
fn prim_close_with_line(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let pp = as_prepath(args.pop().unwrap())?;
    Ok(Value::Path(Path {
        subpaths: vec![Subpath {
            start: pp.start,
            segs: pp.segs,
            closing: Closing::Line,
        }],
    }))
}

/// `fill : color -> path -> graphics` (vminst.ml:2398) — a filled region;
/// the PDF writer (`place_graphics`, satysfi-pdf) paints it with the
/// even-odd rule, matching upstream's `op_f'`.
fn prim_fill(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let path = as_path(args.pop().unwrap())?;
    let color = as_color(args.pop().unwrap())?;
    Ok(Value::Graphics(GraphicsElem::Fill(color, path)))
}

/// `stroke : length -> color -> path -> graphics` (vminst.ml:2381) — width
/// first, then color, then path.
fn prim_stroke(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let path = as_path(args.pop().unwrap())?;
    let color = as_color(args.pop().unwrap())?;
    let wid = as_length(args.pop().unwrap())?;
    Ok(Value::Graphics(GraphicsElem::Stroke(wid, color, path)))
}

/// `inline-graphics : length -> length -> length -> (point -> graphics
/// list) -> inline-boxes` (vminst.ml:1872 `BackendInlineGraphics`) — a box
/// of size `(w, h, d)` carrying the callback's resolved graphics elements,
/// the minimal on-page sink for a `graphics` value (§3 of the plan).
///
/// **Slice-1 eager-callback shortcut.** Upstream defers the callback until
/// the box's *placed* point is known on the page, then calls
/// `gfun(placed_point)`. A lang closure cannot live inside a backend box
/// (`PureHorzBox::Graphics` only holds resolved `GraphicsElem`s), and the
/// placed point isn't known until page-break/render time — so instead this
/// calls `gfun` immediately at `(0pt, 0pt)`, and the PDF writer
/// (`place_graphics`, satysfi-pdf) translates the *whole* box to its placed
/// position via a single `cm` at render time. This equals upstream's
/// behavior if and only if `gfun` uses its point argument purely additively
/// (shift-covariant) — true of every real `Gr`/`deco` generator, but not
/// enforced by this signature. See `docs/plans/graphics-subsystem.md`
/// §3/Risks for the full discussion; faithfully deferring this is the same
/// architecture roadmap phase E (decoration hooks) needs.
fn prim_inline_graphics(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let gfun = args.pop().unwrap();
    let d = as_length(args.pop().unwrap())?;
    let h = as_length(args.pop().unwrap())?;
    let w = as_length(args.pop().unwrap())?;
    let origin = make_point_value((Length::ZERO, Length::ZERO));
    let list_v = interp.apply(gfun, origin)?;
    let items = match list_v {
        Value::List(v) => v,
        other => {
            return eval_error(format!(
                "inline-graphics: callback must return a graphics list, got {}",
                other.type_name()
            ))
        }
    };
    let mut elems = Vec::with_capacity(items.len());
    for it in items {
        elems.push(as_graphics(it)?);
    }
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::Graphics {
            width: w,
            height: h,
            depth: d,
            elems,
        },
    )]))
}

/// `inline-graphics-outer : length -> length -> (length -> point -> graphics
/// list) -> inline-boxes` (vminst.ml:1891 `BackendInlineGraphicsOuter`) — a
/// graphics box whose width stretches like `inline-fil` (upstream widinfo
/// `Fils(1)`). The callback needs the RESOLVED width, unknown until line
/// layout, so it is deferred through `Interp::outer_graphics` (the `HookId`
/// pattern) and fired by `resolve_outer_graphics_in_contents` (called from
/// `line-break`/`tabular`/`draw-text`) with the width `justify_line` wrote
/// into the box and the point `(0pt, 0pt)` — the same shift-covariance
/// shortcut as `inline-graphics` above (the writer's `cm` supplies the
/// placed point); the width argument is faithful.
fn prim_inline_graphics_outer(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let gfun = args.pop().unwrap();
    let d = as_length(args.pop().unwrap())?;
    let h = as_length(args.pop().unwrap())?;
    interp.outer_graphics.push(gfun);
    let fn_id = GraphicsFnId(interp.outer_graphics.len() - 1);
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::GraphicsOuter {
            height: h,
            depth: d,
            width: Length::ZERO,
            fn_id,
        },
    )]))
}

/// Fire every deferred `inline-graphics-outer` callback in an already-
/// justified run, replacing its `GraphicsOuter` marker with a resolved
/// `Graphics` box (see `prim_inline_graphics_outer`). Idempotent (a resolved
/// box no longer matches) and cheap when nothing matches (one pass, no
/// allocation).
fn resolve_outer_graphics_in_contents(
    interp: &mut Interp,
    contents: &mut [(Length, PureHorzBox)],
) -> Result<(), EvalError> {
    for (_, bx) in contents.iter_mut() {
        if let PureHorzBox::GraphicsOuter { height, depth, width, fn_id } = bx {
            let (w, h, d) = (*width, *height, *depth);
            let gfun = match interp.outer_graphics.get(fn_id.0) {
                Some(f) => f.clone(),
                None => {
                    return eval_error(format!(
                        "inline-graphics-outer: dangling callback index {}",
                        fn_id.0
                    ))
                }
            };
            let partial = interp.apply(gfun, Value::Length(w))?;
            let listv = interp.apply(partial, make_point_value((Length::ZERO, Length::ZERO)))?;
            let items = as_list(listv)?;
            let mut elems = Vec::with_capacity(items.len());
            for it in items {
                elems.push(as_graphics(it)?);
            }
            *bx = PureHorzBox::Graphics { width: w, height: h, depth: d, elems };
        }
    }
    Ok(())
}

/// `tabular : (cell list) list -> (length list -> length list -> graphics
/// list) -> inline-boxes` (vminst.ml:539) — solve the grid (backend
/// `satysfi_backend::tabular::main`, docs/plans/table-subsystem.md §1) and
/// eagerly drive the rule callback with the solved box-local grid-line
/// coordinates.
///
/// **Why eager is faithful here, unlike `inline-graphics`.** The callback's
/// arguments are the grid-line coordinates, fully determined by cell
/// content alone (`main` computes them before any placement) — so calling
/// it once at construction time with the true box-local `xs`/`ys` is exactly
/// what upstream's later, placement-time call produces once the PDF
/// writer's per-box `cm` translate (shared with `place_graphics`, see
/// `satysfi-pdf`) shifts the resulting rule paths into position. No
/// shift-covariance caveat (contrast `prim_inline_graphics` above).
fn prim_tabular(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let rulesf = args.pop().unwrap();
    let rows = as_cell_grid(args.pop().unwrap())?;
    let mut solved = satysfi_backend::tabular::main(rows);
    for cell in &mut solved.cells {
        resolve_outer_graphics_in_contents(interp, &mut cell.contents)?;
    }

    let xs = make_length_list(&solved.xs);
    let ys = make_length_list(&solved.ys);
    let partial = interp.apply(rulesf, xs)?;
    let gval = interp.apply(partial, ys)?;
    let items = as_list(gval)?;
    let mut rules = Vec::with_capacity(items.len());
    for it in items {
        rules.push(as_graphics(it)?);
    }

    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::Tabular(TabularBox {
            width: solved.width,
            height: solved.height,
            depth: Length::ZERO,
            cells: solved.cells,
            rules,
        }),
    )]))
}

// ============================================================================
// ---- gr.satyh roadmap graphics primitives (docs/plans/graphics-
// subsystem.md §Full roadmap A/B/C/D) -------------------------------------
// ============================================================================

/// `bezier-to : point -> point -> point -> pre-path -> pre-path`
/// (vminst.ml:742) — appends a cubic Bézier segment (`ptS`/`ptT` control
/// points, `pt1` destination) to the pre-path's forward-accumulated `segs`.
fn prim_bezier_to(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let mut pp = as_prepath(args.pop().unwrap())?;
    let pt1 = as_point(args.pop().unwrap())?;
    let pt_t = as_point(args.pop().unwrap())?;
    let pt_s = as_point(args.pop().unwrap())?;
    pp.segs.push(PathSeg::Bezier(pt_s, pt_t, pt1));
    Ok(Value::PrePath(pp))
}

/// `close-with-bezier : point -> point -> pre-path -> path` (vminst.ml:787)
/// — closes the subpath with a cubic Bézier back to its start (`ptS`/`ptT`
/// control points; the destination is always the subpath's own `start`, per
/// `Closing::Bezier`'s doc comment).
fn prim_close_with_bezier(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let pp = as_prepath(args.pop().unwrap())?;
    let pt_t = as_point(args.pop().unwrap())?;
    let pt_s = as_point(args.pop().unwrap())?;
    Ok(Value::Path(Path {
        subpaths: vec![Subpath {
            start: pp.start,
            segs: pp.segs,
            closing: Closing::Bezier(pt_s, pt_t),
        }],
    }))
}

/// `shift-path : point -> path -> path` (vminst.ml:663) — translate every
/// point of the path by the given vector (`satysfi_backend::shift_path`).
fn prim_shift_path(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let path = as_path(args.pop().unwrap())?;
    let v = as_point(args.pop().unwrap())?;
    Ok(Value::Path(shift_path(v, &path)))
}

/// `linear-transform-path : float -> float -> float -> float -> path ->
/// path` (vminst.ml:678) — apply the 2x2 matrix `(a, b, c, d)` to every
/// point of the path (`satysfi_backend::linear_transform_path`).
fn prim_linear_transform_path(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let path = as_path(args.pop().unwrap())?;
    let d = as_float(args.pop().unwrap())?;
    let c = as_float(args.pop().unwrap())?;
    let b = as_float(args.pop().unwrap())?;
    let a = as_float(args.pop().unwrap())?;
    Ok(Value::Path(linear_transform_path((a, b, c, d), &path)))
}

/// `shift-graphics : point -> graphics -> graphics` (vminst.ml:2451) —
/// translate every point of the graphics element by the given vector.
fn prim_shift_graphics(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let g = as_graphics(args.pop().unwrap())?;
    let v = as_point(args.pop().unwrap())?;
    Ok(Value::Graphics(shift_graphics(v, &g)))
}

/// `linear-transform-graphics : float -> float -> float -> float ->
/// graphics -> graphics` (vminst.ml:2432). **Eager, unlike upstream**:
/// `graphicD.ml`'s `make_linear_trans` lazily wraps the element in a
/// `LinearTrans` node, deferring the matrix to a PDF `cm` operator at render
/// time — which also scales any wrapped `Stroke`/`DashedStroke`'s effective
/// line width (width is specified in the pre-transform coordinate space).
/// This port instead rewrites every point up front (see
/// `docs/plans/graphics-subsystem.md`'s roadmap A/B: "PURE coordinate maps
/// … no PDF change needed") and leaves `width` untouched, so a non-uniform
/// `scale-graphics` (`gr.satyh`) will NOT scale a stroke's line width the
/// way upstream does — invisible for pure rotation (`rotate-graphics`,
/// orthonormal, preserves lengths) and for `Fill`, which is the only
/// `GraphicsElem` shape any bundled package actually strokes-then-scales.
fn prim_linear_transform_graphics(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let g = as_graphics(args.pop().unwrap())?;
    let d = as_float(args.pop().unwrap())?;
    let c = as_float(args.pop().unwrap())?;
    let b = as_float(args.pop().unwrap())?;
    let a = as_float(args.pop().unwrap())?;
    Ok(Value::Graphics(linear_transform_graphics((a, b, c, d), &g)))
}

/// `get-graphics-bbox : graphics -> point * point` (vminst.ml:2466) — see
/// `satysfi_backend::graphics_bbox`'s doc comment; a `Text` element's bbox
/// is its stored `natural_metrics` extent at the anchor.
fn prim_get_graphics_bbox(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let g = as_graphics(args.pop().unwrap())?;
    let (pmin, pmax) = graphics_bbox(&g);
    Ok(Value::Tuple(vec![
        make_point_value(pmin),
        make_point_value(pmax),
    ]))
}

/// `get-path-bbox : path -> point * point` (vminst.ml:696
/// `PathGetBoundingBox`) — `satysfi_backend::path_bbox` (see that function's
/// doc comment for the exact cubic-extrema policy shared with
/// `get-graphics-bbox`).
fn prim_get_path_bbox(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let path = as_path(args.pop().unwrap())?;
    let (pmin, pmax) = path_bbox(&path);
    Ok(Value::Tuple(vec![
        make_point_value(pmin),
        make_point_value(pmax),
    ]))
}

/// `dashed-stroke : length -> (length*length*length) -> color -> path ->
/// graphics` (vminst.ml:2414) — width first, then the dash pattern, then
/// color, then path (mirrors `stroke`'s argument order with one extra
/// dash-pattern argument inserted).
fn prim_dashed_stroke(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let path = as_path(args.pop().unwrap())?;
    let color = as_color(args.pop().unwrap())?;
    let dash = as_dash(args.pop().unwrap())?;
    let wid = as_length(args.pop().unwrap())?;
    Ok(Value::Graphics(GraphicsElem::DashedStroke(
        wid, dash, color, path,
    )))
}

/// `draw-text : point -> inline-boxes -> graphics` (vminst.ml:2363
/// `PrimitiveDrawText`) — FAITHFUL: lays the run out at natural width
/// (upstream `LineBreak.natural`; here `natural_metrics` + `fit_cell` at that
/// width, so slack is 0 and every box keeps its natural advance) and stores
/// the placed run in `GraphicsElem::Text`. Also resolves any
/// `inline-graphics-outer` marker the run carries (`resolve_outer_graphics_
/// in_contents` — width 0 there, since slack is 0 at natural width, upstream
/// identical: `widperfil = 0`).
fn prim_draw_text(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ib = as_inline_boxes(args.pop().unwrap())?;
    let pt = as_point(args.pop().unwrap())?;
    let (width, height, depth) = natural_metrics(&ib);
    let (mut contents, _, _) = fit_cell(ib, width);
    resolve_outer_graphics_in_contents(interp, &mut contents)?;
    Ok(Value::Graphics(GraphicsElem::Text { pt, contents, width, height, depth }))
}

// ============================================================================
// ---- pervasives.satyh unblockers (docs/plans/stdlib-port.md) -------------
// ============================================================================

/// `get-natural-metrics : inline-boxes -> length * length * length`
/// (vminst.ml:2020 `PrimitiveGetNaturalMetrics`) — FAITHFUL: delegates to
/// `satysfi_backend::natural_metrics` (see that function's doc comment for
/// why no depth sign-flip is needed here, unlike upstream).
fn prim_get_natural_metrics(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ib = as_inline_boxes(args.pop().unwrap())?;
    let (width, height, depth) = natural_metrics(&ib);
    Ok(Value::Tuple(vec![
        Value::Length(width),
        Value::Length(height),
        Value::Length(depth),
    ]))
}

/// Build the atomic `PureHorzBox::Frame` for `inline-frame-outer`/`-inner`/
/// `-breakable`: fit `inner` at its natural width (`fit_cell` — the same
/// no-Context fit tabular cells use), pad the fitted run by `pads`, intern
/// `deco` into `interp.decos`. §D (docs/plans/hooks-annotations-crossref.md):
/// `deco` is fired lang-side, after placement, by `fire_hooks`/
/// `fire_inline_frame` — this constructor never calls it.
fn make_inline_frame(
    interp: &mut Interp,
    (pad_l, pad_r, pad_t, pad_b): (Length, Length, Length, Length),
    deco: Value,
    inner: Vec<HorzBox>,
) -> Value {
    let (w, _, _) = natural_metrics(&inner);
    let (contents, height, depth) = fit_cell(inner, w);
    let contents = contents.into_iter().map(|(x, b)| (x + pad_l, b)).collect();
    let id = DecoId(interp.decos.len());
    interp.decos.push(DecoEntry::Inline { deco });
    Value::InlineBoxes(vec![HorzBox::Pure(PureHorzBox::Frame {
        width: pad_l + w + pad_r,
        height: height + pad_t,
        depth: depth + pad_b,
        deco: id,
        contents,
    })])
}

/// `inline-frame-outer : paddings -> deco -> inline-boxes -> inline-boxes`
/// (vminst.ml:1787 `BackendOuterFrame`) — FAITHFUL: builds the atomic
/// `PureHorzBox::Frame` (§D); see [`make_inline_frame`]. Upstream's
/// outer/inner distinction is glue participation in the enclosing line
/// (`PHGOuterFrame` vs `PHGInnerFrame`), which this atomic box model
/// collapses — both this and [`prim_inline_frame_inner`] build the exact
/// same box.
fn prim_inline_frame_outer(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let inner = as_inline_boxes(args.pop().unwrap())?;
    let deco = args.pop().unwrap();
    let pads = as_paddings(args.pop().unwrap())?;
    Ok(make_inline_frame(interp, pads, deco, inner))
}

/// `inline-frame-inner : paddings -> deco -> inline-boxes -> inline-boxes`
/// (vminst.ml:1807 `BackendInnerFrame`) — same construction as
/// [`prim_inline_frame_outer`]; see that function's doc comment for the
/// outer/inner distinction this atomic model collapses.
fn prim_inline_frame_inner(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let inner = as_inline_boxes(args.pop().unwrap())?;
    let deco = args.pop().unwrap();
    let pads = as_paddings(args.pop().unwrap())?;
    Ok(make_inline_frame(interp, pads, deco, inner))
}

/// `set-manual-rising : length -> context -> context` (vminst.ml:1661
/// `PrimitiveSetManualRising`) — FAITHFULLY stores the rising in the new
/// `Context::manual_rising` field, the same shape as `set-font-size`/
/// `set-leading` above. Like `paragraph_top`/`paragraph_bottom`, nothing
/// reads this field yet: no box-producing primitive in this port shifts
/// text vertically by it (upstream's `PHGRising`/`raise-inline` box has no
/// analogue here yet), so the effect is currently a no-op — a stand-in only
/// in that downstream sense, not in what it stores.
fn prim_set_manual_rising(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let rising = as_length(args.pop().unwrap())?;
    Ok(Value::Context(Box::new(Context {
        manual_rising: rising,
        ..ctx
    })))
}

/// `script-guard : script -> inline-boxes -> inline-boxes` (vminst.ml:1908
/// `BackendScriptGuard`).
///
/// STAND-IN: upstream wraps `hblst` in a `HorzScriptGuard` that tells the
/// line breaker which script to assume at each edge, for inter-script
/// spacing rules (`lineBreak.ml`'s script-boundary handling). This port's
/// line breaker has no script-aware spacing at all yet, so this is the
/// identity function: the `script` argument is accepted (so callers like
/// pervasives.satyh's `\SATySFi`/`\LaTeX`/`\TeX` type-check and run) and
/// discarded.
fn prim_script_guard(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ib = as_inline_boxes(args.pop().unwrap())?;
    let _script = args.pop().unwrap();
    Ok(Value::InlineBoxes(ib))
}

/// Unwrap `inline-boxes`' `Vec<HorzBox>` down to the bare `Vec<PureHorzBox>`
/// a `PureHorzBox::Discretionary` slot stores (mirrors `prim_line_break`'s
/// identical unwrap, linebreak.rs's only other consumer of this shape).
fn into_pure(boxes: Vec<HorzBox>) -> Vec<PureHorzBox> {
    boxes.into_iter().map(|HorzBox::Pure(p)| p).collect()
}

/// `discretionary : int -> inline-boxes -> inline-boxes -> inline-boxes ->
/// inline-boxes` (vminst.ml:1969 `BackendDiscretionary`), params `(pb,
/// hblst0, hblst1, hblst2)` — FAITHFUL: builds the same
/// `PureHorzBox::Discretionary` the UAX#14 line breaker already produces
/// internally. `hblst0` (`no_break`) renders when this point is NOT chosen
/// as a line break; `hblst1`/`hblst2` (`pre_break`/`post_break`) render at
/// the end/start of the two lines a break here would produce.
fn prim_discretionary(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let post_break = as_inline_boxes(args.pop().unwrap())?;
    let pre_break = as_inline_boxes(args.pop().unwrap())?;
    let no_break = as_inline_boxes(args.pop().unwrap())?;
    let penalty = as_int(args.pop().unwrap())?;
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::Discretionary {
            penalty: penalty.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            pre_break: into_pure(pre_break),
            post_break: into_pure(post_break),
            no_break: into_pure(no_break),
        },
    )]))
}

/// `get-axis-height : context -> length` (vminst.ml:1739
/// `PrimitiveGetAxisHeight`), needed by `picture.satyh`'s `Picture.node`
/// (docs/plans/stdlib-port.md's Tier-2 decoration/graphics wave) — centers
/// text vertically around the math axis.
///
/// FAITHFUL (§B1): reads `axis_height` from `ctx.math_font`'s OpenType MATH
/// table via `MathC` (`FontInfo.get_axis_height mfabbrev fontsize`), falling
/// back to the same fixed `0.25` ratio of `ctx.font_size` this returned
/// before this slice (`pervasives.satyh`'s `\SATySFi`/`\LaTeX` manual-rising
/// ratio) whenever the font has no MATH table — so base-14/non-math output
/// is unchanged.
fn prim_get_axis_height(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let mc = MathC::of(interp, ctx.math_font);
    Ok(Value::Length(mc.axis(ctx.font_size)))
}

// ============================================================================
// ---- docs/plans/hooks-annotations-crossref.md §Slice 1: page-break hooks
// + cross-references ----
// ============================================================================

/// `hook-page-break : (page-break-info -> point -> unit) -> inline-boxes`
/// (vminstdef.yaml:576). Pushes the closure argument onto `interp.hooks`
/// (the lang-side table `fire_hooks` reads back after placement) and
/// returns an inline box carrying only the opaque `HookId` — exactly
/// `prim_load_image`'s shape (`ImageId`/`interp.images`), applied to a
/// deferred *computation* instead of a resource. The backend places this
/// box like any other zero-width content and never sees the closure.
fn prim_hook_page_break(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let closure = args.pop().unwrap();
    let id = HookId(interp.hooks.len());
    interp.hooks.push(closure);
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::HookPageBreak { id },
    )]))
}

/// `hook-page-break-block : (page-break-info -> point -> unit) ->
/// block-boxes` (vminst.ml:632 `BackendHookPageBreakBlock`) — the
/// block-level analog of `prim_hook_page_break` above, FAITHFUL: same
/// `interp.hooks` push, same opaque `HookId`, but wrapped in a
/// `VertBox::HookPageBreak` marker instead of an inline box. `chop_page`/
/// `place_block_at` (satysfi-backend) place it as a zero-height
/// `PlacedLine` carrying the SAME `PureHorzBox::HookPageBreak` wrapper the
/// inline primitive uses, so `fire_hooks` (lib.rs) fires it through the
/// exact same scan with no changes of its own.
fn prim_hook_page_break_block(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let closure = args.pop().unwrap();
    let id = HookId(interp.hooks.len());
    interp.hooks.push(closure);
    Ok(Value::BlockBoxes(vec![VertBox::HookPageBreak(id)]))
}

/// `register-cross-reference : string -> string -> unit` (vminstdef.yaml:1793).
/// Callable anywhere (not just from a hook) — ordinary strict primitive
/// over the shared `crossrefs` table.
fn prim_register_cross_reference(
    interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let value = as_str(args.pop().unwrap())?;
    let key = as_str(args.pop().unwrap())?;
    interp.crossrefs.borrow_mut().register(key, value);
    Ok(Value::Unit)
}

/// `get-cross-reference : string -> string option` (vminstdef.yaml:1808).
/// A miss is recorded (`CrossRefs::get`) so an unresolved forward reference
/// forces another fixpoint trial; the result surfaces as the SATySFi
/// `option` variant (`None` / `Some(string)`).
fn prim_get_cross_reference(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let key = as_str(args.pop().unwrap())?;
    Ok(match interp.crossrefs.borrow_mut().get(&key) {
        Some(v) => Value::Ctor("Some".to_string(), Some(Box::new(Value::Str(v)))),
        None => Value::Ctor("None".to_string(), None),
    })
}

/// `probe-cross-reference : string -> string option` (vminst.ml:3043
/// `BackendProbeCrossReference`) — FAITHFUL: `get-cross-reference` minus the
/// miss bookkeeping (`CrossRefs::probe`, crossRef.ml:112), so a `None` here
/// never forces another fixpoint trial.
fn prim_probe_cross_reference(
    interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let key = as_str(args.pop().unwrap())?;
    Ok(match interp.crossrefs.borrow().probe(&key) {
        Some(v) => Value::Ctor("Some".to_string(), Some(Box::new(Value::Str(v)))),
        None => Value::Ctor("None".to_string(), None),
    })
}

// ============================================================================
// ---- docs/plans/hooks-annotations-crossref.md §B/§D: annot.satyh's prim
// surface (link annotations + the frame/script stand-ins it needs) ----
// ============================================================================

/// `get-leftmost-script`/`get-rightmost-script : inline-boxes -> script
/// option` (vminstdef.yaml:1754/1767 `BackendGetLeftmostScript`/
/// `BackendGetRightmostScript`) — STAND-IN: upstream inspects the actual
/// Unicode script of the first/last character in `hblst`
/// (`LineBreak.get_leftmost_script`/`get_rightmost_script`), which
/// `annot.satyh`'s `\href` uses to `script-guard` the link's edges so
/// inter-script spacing isn't inserted right at the boundary. This port's
/// `PureHorzBox::InnerString` carries no per-character script tag (no
/// script-aware line breaking at all yet — `script-guard` above is already
/// an identity stand-in for the same reason), so both primitives
/// unconditionally return `None`: `\href` then takes its `None` arm
/// (`inline-nil`, no guard inserted) — a safe, honest default rather than
/// fabricating a script this port cannot actually see.
fn prim_get_leftmost_script(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let _ib = as_inline_boxes(args.pop().unwrap())?;
    Ok(Value::Ctor("None".to_string(), None))
}

/// See [`prim_get_leftmost_script`] — the rightmost-edge twin, identical
/// stand-in reasoning.
fn prim_get_rightmost_script(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let _ib = as_inline_boxes(args.pop().unwrap())?;
    Ok(Value::Ctor("None".to_string(), None))
}

/// `inline-frame-breakable : paddings -> deco-set -> inline-boxes ->
/// inline-boxes` (vminstdef.yaml:1672 `BackendOuterFrameBreakable`) — the
/// `inline-frame-outer` playbook applied to a deco *set* instead of a single
/// `deco`: upstream wraps `hblst` in a `HorzFrameBreakable` that can split
/// across a line break, invoking a different one of the four `deco-set`
/// closures (start/head/middle/tail) once each fragment is placed. This
/// port's atomic `PureHorzBox::Frame` never splits, so only `decoS` (the
/// whole-frame closure) is kept — correct for the unbroken case, and this
/// port never breaks it. Faithful for unbroken frames (`\href` in running
/// text that fits one line — the dominant case); a documented deviation for
/// a frame that would have split (upstream would additionally fire
/// `decoH`/`decoM`/`decoT` per fragment).
fn prim_inline_frame_breakable(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let inner = as_inline_boxes(args.pop().unwrap())?;
    let decoset = as_decoset(args.pop().unwrap())?;
    let pads = as_paddings(args.pop().unwrap())?;
    let [deco_s, _h, _m, _t] = decoset;
    Ok(make_inline_frame(interp, pads, deco_s, inner))
}

/// `deco-set` = `Value::Tuple` of 4 closures (`(decoS, decoH, decoM,
/// decoT)`, evalUtil.ml:169 `get_decoset`) — no type check on the elements
/// themselves (they're closures, applied later by `apply_deco`).
fn as_decoset(v: Value) -> Result<[Value; 4], EvalError> {
    match v {
        Value::Tuple(vs) if vs.len() == 4 => {
            let mut it = vs.into_iter();
            let a = it.next().unwrap();
            let b = it.next().unwrap();
            let c = it.next().unwrap();
            let d = it.next().unwrap();
            Ok([a, b, c, d])
        }
        other => eval_error(format!(
            "expected a deco-set (4-tuple of decorations), got {}",
            other.type_name()
        )),
    }
}

/// `make_frame_deco` (evalUtil.ml:604): apply a curried
/// `point -> length -> length -> length -> graphics list` deco and coerce
/// the result. Depths here are already user-sign (nonnegative), so no
/// negate (upstream negates because ITS internal depths are nonpositive).
pub(crate) fn apply_deco(
    interp: &mut Interp,
    deco: Value,
    pt: Point,
    w: Length,
    h: Length,
    d: Length,
) -> Result<Vec<GraphicsElem>, EvalError> {
    let v = interp.apply(deco, make_point_value(pt))?;
    let v = interp.apply(v, Value::Length(w))?;
    let v = interp.apply(v, Value::Length(h))?;
    let v = interp.apply(v, Value::Length(d))?;
    as_list(v)?.into_iter().map(as_graphics).collect()
}

/// `(length * color) option` — `register-link-to-uri`/`-to-location`'s
/// trailing border argument (vminstdef.yaml:2755/2775's `vborderopt`),
/// parsed the same way [`as_color`]/[`as_page`] read a `Value::Ctor`.
fn as_border_option(v: Value) -> Result<Option<(Length, Color)>, EvalError> {
    match v {
        Value::Ctor(name, payload) => match (name.as_str(), payload.map(|b| *b)) {
            ("None", None) => Ok(None),
            ("Some", Some(Value::Tuple(vs))) if vs.len() == 2 => {
                let mut it = vs.into_iter();
                let w = as_length(it.next().unwrap())?;
                let c = as_color(it.next().unwrap())?;
                Ok(Some((w, c)))
            }
            (other, _) => eval_error(format!(
                "expected a border option (None / Some(length * color)), got variant '{other}'"
            )),
        },
        other => eval_error(format!("expected an option, got {}", other.type_name())),
    }
}

/// `register-destination : string -> point -> unit` (vminstdef.yaml:2738) —
/// FAITHFUL: upstream `NamedDest.register` + `notify_pagebreak` collapsed
/// into one step, since our firing window (`fire_hooks`) already knows the
/// page. Errors outside that window (§0.5, `annotation.ml:15`'s
/// `State.during_page_break` gate).
fn prim_register_destination(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let (x, y) = as_point(args.pop().unwrap())?;
    let key = as_str(args.pop().unwrap())?;
    let Some(page) = interp.current_page else {
        return eval_error(
            "register-destination can only be called during page breaking \
             (from a page-break hook or a decoration)",
        );
    };
    let name = interp.dest_name(&key);
    interp.destinations.push(NamedDest { page, name, x, y });
    Ok(Value::Unit)
}

/// Shared body of `register-link-to-uri` / `register-link-to-location`
/// (vminstdef.yaml:2753/2773): pops the common `point/w/h/d/border` suffix,
/// builds `annotation.ml:22`'s rect `(x, y - d, x + w, y + h)` (PDF y-up
/// points; our depths are already nonnegative), and pushes the `Annot`.
fn register_link(
    interp: &mut Interp,
    mut args: Vec<Value>,
    prim_name: &str,
    make_action: impl FnOnce(&mut Interp, String) -> AnnotAction,
) -> Result<Value, EvalError> {
    let border = as_border_option(args.pop().unwrap())?;
    let dpt = as_length(args.pop().unwrap())?;
    let hgt = as_length(args.pop().unwrap())?;
    let wid = as_length(args.pop().unwrap())?;
    let (x, y) = as_point(args.pop().unwrap())?;
    let target = as_str(args.pop().unwrap())?;
    let Some(page) = interp.current_page else {
        return eval_error(format!(
            "{prim_name} can only be called during page breaking \
             (from a page-break hook or a decoration)"
        ));
    };
    let action = make_action(interp, target);
    interp.annotations.push(Annot {
        page,
        rect: (x, y - dpt, x + wid, y + hgt),
        action,
        border,
    });
    Ok(Value::Unit)
}

/// `register-link-to-uri : string -> point -> length -> length -> length ->
/// (length * color) option -> unit` (vminstdef.yaml:2753
/// `BackendRegisterLinkToUri`) — FAITHFUL: see [`register_link`].
fn prim_register_link_to_uri(interp: &mut Interp, args: Vec<Value>) -> Result<Value, EvalError> {
    register_link(interp, args, "register-link-to-uri", |_, uri| AnnotAction::Uri(uri))
}

/// `register-link-to-location : string -> point -> length -> length ->
/// length -> (length * color) option -> unit` (vminstdef.yaml:2773
/// `BackendRegisterLinkToLocation`) — FAITHFUL: same shape as
/// [`prim_register_link_to_uri`], but upstream's action is
/// `GotoName(NamedDest.get name)` — the key goes through the SAME name table
/// as [`prim_register_destination`], so a link to a not-(yet-)registered
/// destination still mints a stable name (a viewer no-ops on it), exactly
/// like upstream.
fn prim_register_link_to_location(interp: &mut Interp, args: Vec<Value>) -> Result<Value, EvalError> {
    register_link(interp, args, "register-link-to-location", |interp, key| {
        AnnotAction::GotoName(interp.dest_name(&key))
    })
}

// ============================================================================
// docs/plans/math-engine.md §A + §G: the faithful `Value::Math` primitive
// layer `math.satyh` is built out of. Every `math-*` primitive here builds
// or consumes a `Value::Math(Rc<Vec<Math>>)` (`value.rs`'s `Math`); a
// `math`-typed argument may equally arrive as a `Value::MathText` (a `${…}`
// literal — `as_math` accepts either, reflecting a `MathText`'s `MathElem`
// tree into `Math` nodes on the fly, see below).
// ============================================================================

use crate::value::{Math, MathElement, MathVariantStyle};

/// `math-class` = `Value::Ctor("MathOrd"|"MathBin"|…, None)` — mirrors
/// `as_color`/`as_page`'s shape exactly.
fn as_math_kind(v: Value) -> Result<MathKind, EvalError> {
    match v {
        Value::Ctor(name, None) => match name.as_str() {
            "MathOrd" => Ok(MathKind::Ord),
            "MathBin" => Ok(MathKind::Bin),
            "MathRel" => Ok(MathKind::Rel),
            "MathOp" => Ok(MathKind::Op),
            "MathPunct" => Ok(MathKind::Punct),
            "MathOpen" => Ok(MathKind::Open),
            "MathClose" => Ok(MathKind::Close),
            "MathPrefix" => Ok(MathKind::Prefix),
            "MathInner" => Ok(MathKind::Inner),
            other => eval_error(format!("expected a math-class constructor, got '{other}'")),
        },
        other => eval_error(format!("expected a math-class, got {}", other.type_name())),
    }
}

/// `math-char-class` = `Value::Ctor("MathItalic"|…, None)`, resolved to the
/// backend's [`MathCharClass`] (gap 5, `docs/plans/math-engine.md` §F — see
/// `value.rs`'s `Math::ChangeCharClass` doc comment).
fn as_math_char_class(v: Value) -> Result<MathCharClass, EvalError> {
    match v {
        Value::Ctor(name, None) => match name.as_str() {
            "MathItalic" => Ok(MathCharClass::Italic),
            "MathBoldItalic" => Ok(MathCharClass::BoldItalic),
            "MathRoman" => Ok(MathCharClass::Roman),
            "MathBoldRoman" => Ok(MathCharClass::BoldRoman),
            "MathScript" => Ok(MathCharClass::Script),
            "MathBoldScript" => Ok(MathCharClass::BoldScript),
            "MathFraktur" => Ok(MathCharClass::Fraktur),
            "MathBoldFraktur" => Ok(MathCharClass::BoldFraktur),
            "MathDoubleStruck" => Ok(MathCharClass::DoubleStruck),
            other => eval_error(format!(
                "expected a math-char-class constructor, got '{other}'"
            )),
        },
        other => eval_error(format!("expected a math-char-class, got {}", other.type_name())),
    }
}

/// `math-variant-char`'s 9-field style record (`value.rs`'s
/// `MathVariantStyle`; `prim_types::t_math_variant_style`'s runtime
/// counterpart).
fn as_math_variant_style(v: Value) -> Result<MathVariantStyle, EvalError> {
    match v {
        Value::Record(mut fields) => {
            let mut take = |label: &str| -> Result<String, EvalError> {
                match fields.remove(label) {
                    Some(v) => as_str(v),
                    None => eval_error(format!(
                        "math-variant-char style record missing field '{label}'"
                    )),
                }
            };
            Ok(MathVariantStyle {
                italic: take("italic")?,
                bold_italic: take("bold-italic")?,
                roman: take("roman")?,
                bold_roman: take("bold-roman")?,
                script: take("script")?,
                bold_script: take("bold-script")?,
                fraktur: take("fraktur")?,
                bold_fraktur: take("bold-fraktur")?,
                double_struck: take("double-struck")?,
            })
        }
        other => eval_error(format!(
            "expected a math-variant-char style record, got {}",
            other.type_name()
        )),
    }
}

/// A `math` argument: either an already-faithful `Value::Math` (built by
/// another `math-*` primitive), or a `${…}` literal `Value::MathText`,
/// reflected into `Math` nodes on the fly via [`reflect_math_elem`] — see
/// `value.rs`'s `Value::Math` doc comment for why both are interchangeable.
fn as_math(interp: &mut Interp, v: Value) -> Result<Rc<Vec<Math>>, EvalError> {
    match v {
        Value::Math(m) => Ok(m),
        Value::MathText { elems, env } => {
            let mut out = Vec::new();
            for e in elems.iter() {
                reflect_math_elem(interp, e, &env, &mut out)?;
            }
            Ok(Rc::new(out))
        }
        other => eval_error(format!("expected math, got {}", other.type_name())),
    }
}

/// Reflect one elaborated `${…}` literal `MathElem` (Slice 1's fused,
/// math-class-free form) into zero-or-more faithful `Math` atoms, pushed
/// onto `out` — the "less churn" resolution `docs/plans/math-engine.md`'s
/// design-debt Risk names: `MathElem` stays the fast path for a bare
/// `${x^2}` in prose (`read_inline`'s `EmbedMath` arm, untouched), and only
/// gets reflected into `Value::Math` at a command/primitive boundary (here
/// — whenever a `${…}` literal is passed where a faithful `math` value is
/// expected). `Cmd`/`Embed` are resolved by actually evaluating them against
/// `env` (the literal's own captured environment) and recursively
/// reflecting/flattening the result — this is the "Embed of a `#…` program
/// value that itself evaluates to math" roadmap-A add the plan calls for.
fn reflect_math_elem(
    interp: &mut Interp,
    elem: &MathElem,
    env: &Env,
    out: &mut Vec<Math>,
) -> Result<(), EvalError> {
    match elem {
        MathElem::Chars(s) => {
            // One atom per MATHCHAR token (gap 5's "one atom per run" —
            // the lexer already grouped a symbol run or a single latin
            // digit/letter into `s`); class + codepoint remap are both
            // deferred to `layout_math_atom`'s `VariantCharPending` arm,
            // where `Context::math_class_map`/`math_variant_char_map` and
            // the current font are available.
            out.push(Math::Pure(MathElement::VariantCharPending(s.clone())));
            Ok(())
        }
        MathElem::Group(elems) => {
            for e in elems {
                reflect_math_elem(interp, e, env, out)?;
            }
            Ok(())
        }
        MathElem::Sub(base, script) => {
            let mut base_v = Vec::new();
            reflect_math_elem(interp, base, env, &mut base_v)?;
            let mut script_v = Vec::new();
            for e in script {
                reflect_math_elem(interp, e, env, &mut script_v)?;
            }
            out.push(Math::Sub(base_v, script_v));
            Ok(())
        }
        MathElem::Sup(base, script) => {
            let mut base_v = Vec::new();
            reflect_math_elem(interp, base, env, &mut base_v)?;
            let mut script_v = Vec::new();
            for e in script {
                reflect_math_elem(interp, e, env, &mut script_v)?;
            }
            out.push(Math::Sup(base_v, script_v));
            Ok(())
        }
        MathElem::Primes(base, n) => {
            let mut base_v = Vec::new();
            reflect_math_elem(interp, base, env, &mut base_v)?;
            let primes: String = std::iter::repeat('\u{2032}').take(*n).collect();
            out.push(Math::Sup(
                base_v,
                vec![Math::Pure(MathElement::Char {
                    class: MathKind::Ord,
                    big: false,
                    chars: primes,
                })],
            ));
            Ok(())
        }
        MathElem::Cmd { name, span, args } => {
            let cmd = env.lookup(name).ok_or_else(|| EvalError {
                span: Some(*span),
                msg: format!("unbound math command '{name}' at run time"),
            })?;
            let mut v = cmd;
            for arg in args {
                let arg_v = interp.eval_arg(env, arg)?;
                v = interp.apply(v, arg_v)?;
            }
            let m = as_math(interp, v)?;
            out.extend(m.iter().cloned());
            Ok(())
        }
        MathElem::Embed { expr, span: _ } => {
            let v = interp.eval_arg(env, expr)?;
            let m = as_math(interp, v)?;
            out.extend(m.iter().cloned());
            Ok(())
        }
    }
}

fn single_math(m: Math) -> Value {
    Value::Math(Rc::new(vec![m]))
}

fn prim_math_char(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let s = as_str(args.pop().unwrap())?;
    let class = as_math_kind(args.pop().unwrap())?;
    let _ = interp;
    Ok(single_math(Math::Pure(MathElement::Char {
        class,
        big: false,
        chars: s,
    })))
}

fn prim_math_big_char(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let s = as_str(args.pop().unwrap())?;
    let class = as_math_kind(args.pop().unwrap())?;
    let _ = interp;
    Ok(single_math(Math::Pure(MathElement::Char {
        class,
        big: true,
        chars: s,
    })))
}

fn prim_math_char_with_kern(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let kern_r = args.pop().unwrap();
    let kern_l = args.pop().unwrap();
    let s = as_str(args.pop().unwrap())?;
    let class = as_math_kind(args.pop().unwrap())?;
    let _ = interp;
    Ok(single_math(Math::Pure(MathElement::CharWithKern {
        class,
        big: false,
        chars: s,
        kern_l: Box::new(kern_l),
        kern_r: Box::new(kern_r),
    })))
}

fn prim_math_big_char_with_kern(
    interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let kern_r = args.pop().unwrap();
    let kern_l = args.pop().unwrap();
    let s = as_str(args.pop().unwrap())?;
    let class = as_math_kind(args.pop().unwrap())?;
    let _ = interp;
    Ok(single_math(Math::Pure(MathElement::CharWithKern {
        class,
        big: true,
        chars: s,
        kern_l: Box::new(kern_l),
        kern_r: Box::new(kern_r),
    })))
}

/// `math-concat : math -> math -> math` (vminst.ml:193) — FAITHFUL: a plain
/// list append (`math` is always a flat sequence of atoms; see `value.rs`'s
/// `Value::Math` doc comment).
fn prim_math_concat(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m2 = args.pop().unwrap();
    let m1 = args.pop().unwrap();
    let m1 = as_math(interp, m1)?;
    let m2 = as_math(interp, m2)?;
    let mut out = (*m1).clone();
    out.extend((*m2).iter().cloned());
    Ok(Value::Math(Rc::new(out)))
}

fn prim_math_group(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m = args.pop().unwrap();
    let cls2 = as_math_kind(args.pop().unwrap())?;
    let cls1 = as_math_kind(args.pop().unwrap())?;
    let inner = as_math(interp, m)?;
    Ok(single_math(Math::Group(cls1, cls2, (*inner).clone())))
}

fn prim_math_sup(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m2 = args.pop().unwrap();
    let m1 = args.pop().unwrap();
    let base = as_math(interp, m1)?;
    let script = as_math(interp, m2)?;
    Ok(single_math(Math::Sup((*base).clone(), (*script).clone())))
}

fn prim_math_sub(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m2 = args.pop().unwrap();
    let m1 = args.pop().unwrap();
    let base = as_math(interp, m1)?;
    let script = as_math(interp, m2)?;
    Ok(single_math(Math::Sub((*base).clone(), (*script).clone())))
}

fn prim_math_frac(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m2 = args.pop().unwrap();
    let m1 = args.pop().unwrap();
    let num = as_math(interp, m1)?;
    let den = as_math(interp, m2)?;
    Ok(single_math(Math::Fraction((*num).clone(), (*den).clone())))
}

/// `math-radical : math option -> math -> math` (vminst.ml:274) — `None`
/// degree is `\sqrt`; upstream's `MathRadicalWithDegree` (`\sqrt[n]`) is
/// unimplemented too (`math.ml:886`), carried faithfully but not rendered
/// specially, matching upstream by parity (see `value.rs`'s `Math::Radical`).
fn prim_math_radical(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m2 = args.pop().unwrap();
    let opt = args.pop().unwrap();
    let radicand = as_math(interp, m2)?;
    let degree = match opt {
        Value::Ctor(name, payload) if name == "None" && payload.is_none() => None,
        Value::Ctor(name, Some(payload)) if name == "Some" => Some((*as_math(interp, *payload)?).clone()),
        other => {
            return eval_error(format!(
                "expected a math option (None/Some), got {}",
                other.type_name()
            ))
        }
    };
    Ok(single_math(Math::Radical(degree, (*radicand).clone())))
}

fn prim_math_lower(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m2 = args.pop().unwrap();
    let m1 = args.pop().unwrap();
    let base = as_math(interp, m1)?;
    let lower = as_math(interp, m2)?;
    Ok(single_math(Math::LowerLimit((*base).clone(), (*lower).clone())))
}

fn prim_math_upper(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m2 = args.pop().unwrap();
    let m1 = args.pop().unwrap();
    let base = as_math(interp, m1)?;
    let upper = as_math(interp, m2)?;
    Ok(single_math(Math::UpperLimit((*base).clone(), (*upper).clone())))
}

/// `math-pull-in-scripts : math-class -> math-class -> (math option -> math
/// option -> math) -> math` (vminst.ml:368) — FAITHFUL construction: the
/// resolver closure is stored opaquely here, only ever invoked by
/// `layout_pull_in_scripts` (Gap 2, `class-signature-lang-gaps.md`) — with
/// the subscript/superscript actually pulled in off an enclosing `Sub`/`Sup`
/// (`{scripts} m^{sup}`-style), or with `(None, None)` for the common
/// unscripted case (a bare `\sum`/`\int` with nothing pulled in).
fn prim_math_pull_in_scripts(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let resolver = args.pop().unwrap();
    let cls2 = as_math_kind(args.pop().unwrap())?;
    let cls1 = as_math_kind(args.pop().unwrap())?;
    let _ = interp;
    Ok(single_math(Math::PullInScripts(cls1, cls2, Box::new(resolver))))
}

fn prim_math_color(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m = args.pop().unwrap();
    let color = as_color(args.pop().unwrap())?;
    let inner = as_math(interp, m)?;
    Ok(single_math(Math::ChangeColor(color, (*inner).clone())))
}

fn prim_math_char_class(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m = args.pop().unwrap();
    let cls = as_math_char_class(args.pop().unwrap())?;
    let inner = as_math(interp, m)?;
    Ok(single_math(Math::ChangeCharClass(cls, (*inner).clone())))
}

fn prim_math_variant_char(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let style = as_math_variant_style(args.pop().unwrap())?;
    let class = as_math_kind(args.pop().unwrap())?;
    let _ = interp;
    Ok(single_math(Math::Pure(MathElement::VariantChar {
        class,
        big: false,
        style: Box::new(style),
    })))
}

fn prim_math_paren(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m = args.pop().unwrap();
    let paren_r = args.pop().unwrap();
    let paren_l = args.pop().unwrap();
    let inner = as_math(interp, m)?;
    Ok(single_math(Math::Paren(
        Box::new(paren_l),
        Box::new(paren_r),
        (*inner).clone(),
    )))
}

fn prim_math_paren_with_middle(
    interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let mlst = args.pop().unwrap();
    let middle = args.pop().unwrap();
    let paren_r = args.pop().unwrap();
    let paren_l = args.pop().unwrap();
    let items = as_list(mlst)?;
    let mut mlstlst = Vec::with_capacity(items.len());
    for it in items {
        mlstlst.push((*as_math(interp, it)?).clone());
    }
    Ok(single_math(Math::ParenWithMiddle(
        Box::new(paren_l),
        Box::new(paren_r),
        Box::new(middle),
        mlstlst,
    )))
}

fn prim_text_in_math(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let body = args.pop().unwrap();
    let class = as_math_kind(args.pop().unwrap())?;
    let _ = interp;
    Ok(single_math(Math::Pure(MathElement::EmbeddedText {
        class,
        body: Box::new(body),
    })))
}

/// `convert-string-for-math : context -> math-char-class -> string ->
/// string` (`vminstdef.yaml` `PrimitiveConvertStringForMath`). Faithful to
/// upstream: it overrides the context's `math_char_class` with the passed
/// `mccls`, then runs `MathContext.convert_math_variant_char`
/// (`types.cppo.ml:1602`) over the whole string —
///  1. if the WHOLE string is a key of the (token-level) `math_class_map`
///     (`default_math_class_map`, e.g. `"-"` → `"−"` U+2212), return its
///     replacement codepoints; else
///  2. remap each char via the runtime `math_variant_char_map`
///     (`set-math-variant-char` overrides, keyed by `(char, mccls)`) first,
///     then the built-in `default_math_variant_char` table (gap 5's
///     Mathematical-Alphanumeric-Symbols remap), keeping any char with no
///     mapping.
/// Unlike the *rendering*-path `resolve_variant_char`, this string primitive
/// does NOT gate on font glyph availability (upstream's
/// `convert_math_variant_char` never does — it returns codepoints, not
/// glyphs), so `abc` under `MathItalic` yields U+1D44E/44F/450 regardless of
/// the active font.
fn prim_convert_string_for_math(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let s = as_str(args.pop().unwrap())?;
    let class = as_math_char_class(args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    // (1) whole-token class-map hit -> its replacement codepoints verbatim.
    if let Some((target, _mk)) = ctx.math_class_map.get(&s) {
        return Ok(Value::Str(target.clone()));
    }
    // (2) per-char variant remap under the PASSED class (which upstream
    // installs as the effective `math_char_class` before converting).
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let mapped = ctx
            .math_variant_char_map
            .get(&(ch, class))
            .copied()
            .or_else(|| default_math_variant_char(class, ch))
            .unwrap_or(ch);
        out.push(mapped);
    }
    Ok(Value::Str(out))
}

/// `set-math-variant-char : math-char-class -> int -> int -> context ->
/// context` (gap 7, `docs/plans/math-mode-language-gaps.md`) — FAITHFUL:
/// installs a per-`(source char, style)` override into
/// `Context::math_variant_char_map`, consulted by `resolve_variant_char`
/// BEFORE the built-in `default_math_variant_char` table. `Arc::make_mut`
/// copy-on-writes the map so contexts that never call this keep sharing one
/// `Arc`-refcounted empty table.
fn prim_set_math_variant_char(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let mut ctx = as_context(args.pop().unwrap())?;
    let cpto = as_int(args.pop().unwrap())?;
    let cpfrom = as_int(args.pop().unwrap())?;
    let cls = as_math_char_class(args.pop().unwrap())?;
    let from = u32::try_from(cpfrom)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| EvalError {
            span: None,
            msg: format!("set-math-variant-char: {cpfrom} is not a valid Unicode codepoint"),
        })?;
    let to = u32::try_from(cpto)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| EvalError {
            span: None,
            msg: format!("set-math-variant-char: {cpto} is not a valid Unicode codepoint"),
        })?;
    Arc::make_mut(&mut ctx.math_variant_char_map).insert((from, cls), to);
    Ok(Value::Context(Box::new(ctx)))
}

/// The `MathKind` one `MathElement` atom presents as its own boundary class
/// — `Char`/`CharWithKern`/`EmbeddedText`/`VariantChar` carry an explicit
/// `class` field; `VariantCharPending` (gap 5 — not yet resolved to a class
/// at this point in the tree) consults `ctx.math_class_map` the same way
/// `layout_math_atom`'s own arm does, defaulting to `Ord` when the token
/// isn't a whole-token class-map entry (mirrors `layout_math_atom`'s
/// fallback path, whose per-char variant remap never changes the class).
fn math_element_kind(ctx: &Context, me: &MathElement) -> MathKind {
    match me {
        MathElement::Char { class, .. }
        | MathElement::CharWithKern { class, .. }
        | MathElement::EmbeddedText { class, .. }
        | MathElement::VariantChar { class, .. } => *class,
        MathElement::VariantCharPending(s) => ctx
            .math_class_map
            .get(s.as_str())
            .map(|(_, kind)| *kind)
            .unwrap_or(MathKind::Ord),
    }
}

/// Upstream `get_left_math_kind`/`get_right_math_kind` (math.ml:481-524),
/// fused into one direction-parameterized walk over a `&[Math]` list's
/// FIRST (`left = true`) or LAST (`left = false`) element: `Pure` atoms
/// report their own class (`math_element_kind`); `Group`/`PullInScripts`
/// present an explicit, possibly-asymmetric left/right pair; `Sup`/`Sub`/
/// `UpperLimit`/`LowerLimit` recurse into their `base`; `Fraction`/
/// `Radical` are always `Inner`; `Paren`/`ParenWithMiddle` are always
/// `Open`/`Close`; `ChangeColor`/`ChangeCharClass` recurse into `inner`; an
/// empty list is the synthetic `End` boundary sentinel (`MathKind::End`,
/// `horzBox.ml:134`) — `make_math_class_option_value` maps that to `None`,
/// same as upstream's own list-boundary handling.
fn boundary_math_kind(ctx: &Context, ms: &[Math], left: bool) -> MathKind {
    let m = if left { ms.first() } else { ms.last() };
    let Some(m) = m else {
        return MathKind::End;
    };
    match m {
        Math::Pure(me) => math_element_kind(ctx, me),
        Math::Group(cls1, cls2, _) => {
            if left {
                *cls1
            } else {
                *cls2
            }
        }
        Math::PullInScripts(cls1, cls2, _) => {
            if left {
                *cls1
            } else {
                *cls2
            }
        }
        Math::Sup(base, _)
        | Math::Sub(base, _)
        | Math::UpperLimit(base, _)
        | Math::LowerLimit(base, _) => boundary_math_kind(ctx, base, left),
        Math::Fraction(..) | Math::Radical(..) => MathKind::Inner,
        Math::Paren(..) | Math::ParenWithMiddle(..) => {
            if left {
                MathKind::Open
            } else {
                MathKind::Close
            }
        }
        Math::ChangeColor(_, inner) | Math::ChangeCharClass(_, inner) => {
            boundary_math_kind(ctx, inner, left)
        }
    }
}

fn left_math_kind(ctx: &Context, ms: &[Math]) -> MathKind {
    boundary_math_kind(ctx, ms, true)
}

fn right_math_kind(ctx: &Context, ms: &[Math]) -> MathKind {
    boundary_math_kind(ctx, ms, false)
}

/// `math-class option` — `MathKind::End` (the empty-list sentinel) becomes
/// `None`; every real class becomes `Some(<ctor>)`, round-tripping exactly
/// with `as_math_kind`'s ctor names.
fn make_math_class_option_value(mk: MathKind) -> Value {
    let name = match mk {
        MathKind::Ord => "MathOrd",
        MathKind::Bin => "MathBin",
        MathKind::Rel => "MathRel",
        MathKind::Op => "MathOp",
        MathKind::Punct => "MathPunct",
        MathKind::Open => "MathOpen",
        MathKind::Close => "MathClose",
        MathKind::Prefix => "MathPrefix",
        MathKind::Inner => "MathInner",
        MathKind::End => return Value::Ctor("None".to_string(), None),
    };
    Value::Ctor(
        "Some".to_string(),
        Some(Box::new(Value::Ctor(name.to_string(), None))),
    )
}

/// `get-left-math-class : context -> math -> math-class option` (gap 7).
fn prim_get_left_math_class(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m = as_math(interp, args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    Ok(make_math_class_option_value(left_math_kind(&ctx, &m)))
}

/// `get-right-math-class : context -> math -> math-class option` (gap 7).
fn prim_get_right_math_class(
    interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let m = as_math(interp, args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    Ok(make_math_class_option_value(right_math_kind(&ctx, &m)))
}

/// `set-math-command : [math] inline-cmd -> context -> context`
/// FAITHFUL: installs the command `read_inline`'s `EmbedMath` arm applies
/// to bare `${…}`.
fn prim_set_math_command(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let mut ctx = as_context(args.pop().unwrap())?;
    let cmd = args.pop().unwrap();
    ctx.math_command = Some(interp.register_math_command(cmd));
    Ok(Value::Context(Box::new(ctx)))
}

/// Resolve a font abbrev to one of the 3 base faces by name heuristic — the
/// only font-name resolution this milestone has. Shared by set-font/set-math-font.
fn resolve_font_abbrev(abbrev: &str) -> FontKey {
    let lower = abbrev.to_ascii_lowercase();
    if lower.contains("bold") {
        FONT_BOLD
    } else if lower.contains("it") || lower.contains("obl") || lower.contains("slant") {
        FONT_OBLIQUE
    } else {
        FONT_REGULAR
    }
}

/// `set-math-font : string -> context -> context` (vminst.ml:1495) —
/// `abbrev` resolves through the font metrics provider's registry first
/// (D1a, same upgrade as `set-font`), falling back to the milestone-1
/// 3-face name heuristic. A math OTF configured under any abbrev (not just
/// the CLI regular face) can now be selected. Per-abbrev math-file
/// selection beyond that = MATH-table slice (already landed).
fn prim_set_math_font(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let abbrev = as_str(args.pop().unwrap())?;
    let math_font = interp
        .metrics
        .resolve_font_abbrev(&abbrev)
        .unwrap_or_else(|| resolve_font_abbrev(&abbrev));
    Ok(Value::Context(Box::new(Context { math_font, ..ctx })))
}

/// `space-between-maths : context -> math -> math -> inline-boxes option`
/// (vminst.ml:173) — STAND-IN: the real inter-atom glue is the full
/// `space_between_math_kinds` table (`math.ml:319-410`, phase A.4,
/// roadmap); always returns `None` (no extra glue), used by `math.satyh`'s
/// `+align` — never invoked eagerly (that binding is a `let-block` closure).
fn prim_space_between_maths(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let _m2 = args.pop().unwrap();
    let _m1 = args.pop().unwrap();
    let _ctx = as_context(args.pop().unwrap())?;
    Ok(Value::Ctor("None".to_string(), None))
}

/// `raise-inline : length -> inline-boxes -> inline-boxes` — STAND-IN: the
/// line model has no per-box vertical-offset wrapper outside
/// `PureHorzBox::Math`'s own per-glyph `dy` (`docs/plans/math-engine.md`'s
/// "structural difference" note); returns the boxes unshifted (used by
/// `math.satyh`'s `\cases`, never invoked eagerly).
fn prim_raise_inline(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ib = as_inline_boxes(args.pop().unwrap())?;
    let _len = as_length(args.pop().unwrap())?;
    Ok(Value::InlineBoxes(ib))
}

/// `embed-block-breakable : context -> block-boxes -> inline-boxes`
/// (vminst.ml:973) — STAND-IN: no nested page-breakable block-in-inline box
/// exists yet (`hbox.rs` has no `HorzEmbeddedVertBreakable` analog; roadmap
/// E, see `docs/plans/math-engine.md` §E). Renders as an empty box rather
/// than erroring (matching `inline-frame-outer`'s existing "typed
/// faithfully, stand-in body never renders the real thing" precedent) —
/// used by `math.satyh`'s `+math`'s `\eqn`/`\math-list`/`\align`, never
/// invoked eagerly.
fn prim_embed_block_breakable(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let _bb = as_block_boxes(args.pop().unwrap())?;
    let _ctx = as_context(args.pop().unwrap())?;
    Ok(Value::InlineBoxes(Vec::new()))
}

/// `unite-path : path -> path -> path` — FAITHFUL: `path` is upstream's
/// `path list` (a list of independently-closed subpaths — see
/// `graphics.rs`'s `Path` doc comment), so uniting two is a plain
/// subpath-list append. Used by `math.satyh`'s `\norm` (two parallel bars).
fn prim_unite_path(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let p2 = as_path(args.pop().unwrap())?;
    let p1 = as_path(args.pop().unwrap())?;
    let mut subpaths = p1.subpaths;
    subpaths.extend(p2.subpaths);
    Ok(Value::Path(Path { subpaths }))
}

/// `set-min-gap-of-lines : length -> context -> context` (vminst.ml:1291) —
/// STAND-IN: no separate `min_gap_of_lines` field on `Context` yet (see
/// `set-leading`'s own comment on why IT, not this, is the baseline-distance
/// setter); accepted and dropped. Used by `math.satyh`'s `+math-list`, never
/// invoked eagerly.
fn prim_set_min_gap_of_lines(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let _len = as_length(args.pop().unwrap())?;
    Ok(Value::Context(Box::new(ctx)))
}

/// `embed-math : context -> math -> inline-boxes` (vminst.ml:520) — the
/// bridge to the page: the faithful, primitive-driven analog of `read_math`
/// (Slice 1's `MathElem`-walking lowering), operating on a `Value::Math`
/// tree instead. FAITHFUL for the atoms Slice 1 already draws (plain/kerned/
/// variant chars, groups, sup/sub); the structural forms only phases C/D/E
/// add real layout for (fraction/radical/paren/limits/pull-in-scripts/
/// embedded-text) get a deliberately cheap, documented stand-in rendering
/// rather than an error, so `${…}`-shaped math built through these
/// primitives is never *unusable*, just not yet typographically faithful.
fn prim_embed_math(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let m = args.pop().unwrap();
    let ctx = as_context(args.pop().unwrap())?;
    let elems = as_math(interp, m)?;
    let boxed = layout_math_value(interp, &ctx, &elems)?;
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(boxed)]))
}

/// Lay out a faithful `&[Math]` run into one `PureHorzBox::Math`, mirroring
/// `read_math`'s glyph-emission shape (fixed-constant super/subscript
/// shift/scale, the same minimal `Bin`/`Rel` spacer) but keyed on each
/// atom's own EXPLICIT class (from `math-char`/`math-group`/…) rather than
/// `ascii_math_kind`'s inference.
fn layout_math_value(
    interp: &mut Interp,
    ctx: &Context,
    elems: &[Math],
) -> Result<PureHorzBox, EvalError> {
    let (glyphs, rules, width, _left, _right) =
        layout_math_list(interp, ctx, elems, ctx.font_size)?;
    let mut height = Length::ZERO;
    let mut depth = Length::ZERO;
    for g in &glyphs {
        height = height.max(g.dy + g.height);
        depth = depth.max(g.depth - g.dy);
    }
    // §B2: a fraction bar/radical sign is a `Fill` with no `MathGlyph`
    // backing it at all, so the glyph-only aggregation above would silently
    // undercount a run whose bar/sign extends above every glyph's own ink
    // (e.g. `${\sqrt{2}}`'s `l_extra` ascender). Fold every rule's own
    // (y-up, box-local — same frame as `MathGlyph::dy`) bounding box in too.
    for r in &rules {
        let ((_, min_y), (_, max_y)) = graphics_bbox(r);
        height = height.max(max_y);
        depth = depth.max(-min_y);
    }
    Ok(PureHorzBox::Math {
        width,
        height,
        depth,
        glyphs,
        rules,
    })
}

/// Lay out a flat `&[Math]` list at `size`, threading inter-atom spacing
/// (`space_before`, Slice 1's minimal spacer) and returning the glyphs (at
/// LOCAL coordinates starting at `x = 0`), any graphics `rules` an atom
/// pushed (§B2, `docs/plans/math-engine.md`; shifted horizontally by the
/// same running `x` a glyph gets — `layout_math_list` never shifts an
/// atom vertically, only `shift_and_append`'s callers do), the total width,
/// and the boundary classes on either end (needed by a `Group` ancestor,
/// which can present different left/right classes — see `Math::Group`'s doc
/// comment).
fn layout_math_list(
    interp: &mut Interp,
    ctx: &Context,
    elems: &[Math],
    size: Length,
) -> Result<(Vec<MathGlyph>, Vec<GraphicsElem>, Length, MathKind, MathKind), EvalError> {
    let mut glyphs = Vec::new();
    let mut rules = Vec::new();
    let mut x = Length::ZERO;
    let mut last_kind: Option<MathKind> = None;
    let mut first_kind: Option<MathKind> = None;
    for atom in elems {
        let (atom_glyphs, atom_rules, atom_width, left, right) =
            layout_math_atom(interp, ctx, atom, size)?;
        if let Some(prev) = last_kind {
            x += space_before(prev, left, ctx.font_size);
        }
        first_kind.get_or_insert(left);
        let base_x = x;
        for mut g in atom_glyphs {
            g.dx = base_x + g.dx;
            glyphs.push(g);
        }
        for r in &atom_rules {
            rules.push(shift_graphics((base_x, Length::ZERO), r));
        }
        x = base_x + atom_width;
        last_kind = Some(right);
    }
    let left = first_kind.unwrap_or(MathKind::Ord);
    let right = last_kind.unwrap_or(MathKind::Ord);
    Ok((glyphs, rules, x, left, right))
}

/// Upstream `check_subscript` (math.ml:682-699): if a superscript base's
/// LAST element is itself a `Sub`, strip it — returning `(subscript script,
/// new base)` where the new base is the preceding elements followed by the
/// inner `Sub`'s own base, so `{x_1}^2` becomes one base carrying both a
/// sub and a sup. Recurses through `ChangeColor`/`ChangeCharClass`.
fn check_subscript(base: &[Math]) -> Option<(Vec<Math>, Vec<Math>)> {
    let (last, head) = base.split_last()?;
    match last {
        Math::Sub(inner_base, sub_script) => {
            let mut new_base = head.to_vec();
            new_base.extend(inner_base.iter().cloned());
            Some((sub_script.clone(), new_base))
        }
        Math::ChangeColor(color, inner) => {
            let (sub_script, inner_new) = check_subscript(inner)?;
            let mut new_base = head.to_vec();
            new_base.push(Math::ChangeColor(color.clone(), inner_new));
            Some((vec![Math::ChangeColor(color.clone(), sub_script)], new_base))
        }
        Math::ChangeCharClass(cls, inner) => {
            let (sub_script, inner_new) = check_subscript(inner)?;
            let mut new_base = head.to_vec();
            new_base.push(Math::ChangeCharClass(cls.clone(), inner_new));
            Some((vec![Math::ChangeCharClass(cls.clone(), sub_script)], new_base))
        }
        _ => None,
    }
}

/// Upstream `invoke_pull_in_scripts` (math.ml:957-966): call a
/// `math-pull-in-scripts` resolver with the actual pulled-in scripts —
/// `resolver : math option -> math option -> math`, SUBSCRIPT option first,
/// SUPERSCRIPT second — then splice the returned math after the remaining
/// base as ONE `Group(cls1, cls2, …)` atom and lay the whole list out.
#[allow(clippy::too_many_arguments)]
fn layout_pull_in_scripts(
    interp: &mut Interp,
    ctx: &Context,
    head: &[Math],
    cls1: MathKind,
    cls2: MathKind,
    resolver: &Value,
    sub: Option<&[Math]>,
    sup: Option<&[Math]>,
    size: Length,
) -> Result<(Vec<MathGlyph>, Vec<GraphicsElem>, Length, MathKind, MathKind), EvalError> {
    let opt_math = |o: Option<&[Math]>| match o {
        Some(m) => Value::Ctor(
            "Some".to_string(),
            Some(Box::new(Value::Math(Rc::new(m.to_vec())))),
        ),
        None => Value::Ctor("None".to_string(), None),
    };
    let partial = interp.apply(resolver.clone(), opt_math(sub))?;
    let result = interp.apply(partial, opt_math(sup))?;
    let resolved = as_math(interp, result)?;
    let mut items: Vec<Math> = head.to_vec();
    items.push(Math::Group(cls1, cls2, (*resolved).clone()));
    layout_math_list(interp, ctx, &items, size)
}

/// Gap 5's metrics-probe fallback policy: resolve `c` under `ctx`'s current
/// `math_char_class` (checking the runtime override map first, then the
/// built-in `default_math_variant_char` table), but only actually EMIT the
/// remapped codepoint if the current font can render it
/// (`interp.metrics.advance` returns `Some`) — otherwise fall back to the
/// source char `c` (its class, from `Context::math_class_map`/
/// `ascii_math_kind`-style inference, is kept regardless). This is what
/// keeps base-14/WinAnsi documents byte-identical (`Base14Metrics` returns
/// `None` outside ASCII 32-126) while a math-capable TTF, or a permissive
/// test stub, gets the real Mathematical-Alphanumeric glyph automatically.
fn resolve_variant_char(interp: &Interp, ctx: &Context, c: char, size: Length) -> char {
    let mapped = ctx
        .math_variant_char_map
        .get(&(c, ctx.math_char_class))
        .copied()
        .or_else(|| default_math_variant_char(ctx.math_char_class, c));
    match mapped {
        Some(m) if math_char_available(interp, ctx, m, size) => m,
        _ => c,
    }
}

/// §B3b-2: invoke ONE `paren` closure (`math.satyh`'s `paren-left`/
/// `paren-right`/`abs-left`/`brace-left`/…) exactly the way upstream's
/// `make_paren` does (`math.ml:644-649`): 5 CURRIED args in order — inner
/// height `h_in` (≥0), inner depth SIGNED (≤0, hence `-d_in` — this port
/// carries depths as non-negative magnitudes, see this function's `d_in`
/// param doc below), the axis height at the local size, the local
/// (script-scaled) size, and the current text color — then unpack the
/// returned `(inline-boxes, length -> length)` 2-tuple and harvest the
/// boxes' glyphs/rules/width via `math_boxes_of_inline_boxes` (the
/// graphics-harvesting sibling of `math_glyphs_of_inline_boxes`, since a
/// closure's delimiter is drawn `Fill`/`Stroke` ink via `inline-graphics`,
/// not a font glyph). The kernf itself is returned un-invoked (callers
/// re-derive/discard it as `math.ml:923` does for `ParenWithMiddle`'s own
/// middle).
///
/// `d_in`: this port's non-negative ink-depth MAGNITUDE (`inner_ink_extent`'s
/// second component). Upstream's own box depths are non-positive internally
/// (`convert_to_low`'s `dC` folds via `Length.min`, always ≤ `Length.zero`),
/// and `half-length` (`math.satyh:1023-1026`) computes the below-axis need
/// as `hgtaxis +' dpt` on that SIGNED value — so passing the magnitude
/// directly would OVERSIZE every delimiter below the axis (double-counts
/// the depth on the wrong side). Negating here is what keeps the closure's
/// own arithmetic faithful without changing this port's magnitude
/// convention everywhere else.
fn make_paren_run(
    interp: &mut Interp,
    ctx: &Context,
    paren: &Value,
    h_in: Length,
    d_in: Length,
    axis: Length,
    size: Length,
) -> Result<(Vec<MathGlyph>, Vec<GraphicsElem>, Length, Value), EvalError> {
    let mut v = paren.clone();
    let args = [
        Value::Length(h_in),
        Value::Length(-d_in),
        Value::Length(axis),
        Value::Length(size),
        make_color_value(ctx.text_color),
    ];
    for a in args {
        v = interp.apply(v, a)?;
    }
    let (boxes_v, kernf) = match v {
        Value::Tuple(mut items) if items.len() == 2 => {
            let kernf = items.pop().unwrap();
            (items.pop().unwrap(), kernf)
        }
        other => {
            return eval_error(format!(
                "math-paren: a paren closure must return (inline-boxes, length -> length), got {}",
                other.type_name()
            ))
        }
    };
    let boxes = as_inline_boxes(boxes_v)?;
    let (glyphs, rules, width) = math_boxes_of_inline_boxes(&boxes);
    Ok((glyphs, rules, width, kernf))
}

/// §B3b(i)'s original MATH-native stretchy-delimiter body, extracted
/// verbatim as the fallback `Math::Paren`/`Math::ParenWithMiddle` now take
/// when the closure route (`make_paren_run`, primary — upstream-faithful)
/// errors: every delimiter renders as a correctly-SIZED `(`/`)`/`|`
/// regardless of the requested paren kind (identity-wrong, matching this
/// port's behavior before §B3b-2 for any closure that can't be run — a
/// synthetic/ill-shaped test closure, or a real error from a malformed
/// user-supplied one).
fn paren_variant_fallback(
    interp: &mut Interp,
    ctx: &Context,
    parts: Vec<(Vec<MathGlyph>, Vec<GraphicsElem>, Length)>,
    h_in: Length,
    d_in: Length,
    axis: Length,
    size: Length,
) -> Result<(Vec<MathGlyph>, Vec<GraphicsElem>, Length, MathKind, MathKind), EvalError> {
    let target = (h_in - axis).max(axis + d_in) * 2.0;
    let mut glyphs = Vec::new();
    let mut rules = Vec::new();
    let mut x = Length::ZERO;
    push_delimiter_glyph(interp, ctx, '(', size, target, axis, &mut glyphs, &mut x)?;
    for (i, (pg, pr, pw)) in parts.into_iter().enumerate() {
        if i > 0 {
            push_delimiter_glyph(interp, ctx, '|', size, target, axis, &mut glyphs, &mut x)?;
        }
        append_at(&mut glyphs, &mut rules, &mut x, pg, pr, pw);
    }
    push_delimiter_glyph(interp, ctx, ')', size, target, axis, &mut glyphs, &mut x)?;
    Ok((glyphs, rules, x, MathKind::Open, MathKind::Close))
}

/// §B3b-2: re-derive a paren base's TRAILING (right) delimiter's dense math
/// kern function by re-invoking its closure at the script-attachment site
/// (`superscript_kern`'s glyph-corner sampling doesn't apply to a paren
/// base — it has no single "last glyph" to sample italic-correction/corner
/// kerns off; the closure itself IS the source of truth for how much a
/// script should tuck into it, exactly upstream's `lp_math_kern_scheme`,
/// `math.ml:906`/`922`). Closures are pure (`math.satyh`'s bundled ones
/// have no side effects), so re-invoking with the SAME `(h_in, d_in, axis,
/// size)` the original `Math::Paren`/`ParenWithMiddle` layout used yields
/// the identical `kernf` value. Returns `None` when `base`'s last atom
/// isn't a paren, or when re-running its closure(s) errors (the B3b(i)
/// fallback path carries no math-kern scheme at all — `dense_kern`'s
/// caller then falls back to zero, matching that stand-in's own
/// `kerninfo _ = 0pt` shape).
fn paren_trailing_kernf(
    interp: &mut Interp,
    ctx: &Context,
    base: &[Math],
    size: Length,
) -> Option<Value> {
    let (r, h_in, d_in) = match base.last()? {
        Math::Paren(_, r, inner) => {
            let (g, ru, ..) = layout_math_list(interp, ctx, inner, size).ok()?;
            let (h, d) = inner_ink_extent(&g, &ru);
            (r, h, d)
        }
        Math::ParenWithMiddle(_, r, _, parts) => {
            let mut h = Length::ZERO;
            let mut d = Length::ZERO;
            for p in parts {
                let (g, ru, ..) = layout_math_list(interp, ctx, p, size).ok()?;
                let (ph, pd) = inner_ink_extent(&g, &ru);
                h = h.max(ph);
                d = d.max(pd);
            }
            (r, h, d)
        }
        _ => return None,
    };
    let mc = MathC::of(interp, ctx.math_font);
    let axis = mc.axis(size);
    let (_, _, _, kernf) = make_paren_run(interp, ctx, r, h_in, d_in, axis, size).ok()?;
    Some(kernf)
}

/// `fontInfo.ml:361`'s `DenseMathKern` branch: `Length.negate (kernf
/// corrhgt)` — the closure returns a POSITIVE tuck amount (how far to slide
/// the script INTO the delimiter's hollow), and the engine negates it into
/// a kern (negative = closer to the previous glyph, `get_math_kern`'s own
/// doc comment). Any failure (wrong-shaped return, closure error) collapses
/// to `Length::ZERO` — no kern, not a layout error; matches
/// `paren_trailing_kernf`'s own `None`-on-error contract.
fn dense_kern(interp: &mut Interp, kernf: &Value, corrhgt: Length) -> Length {
    match interp.apply(kernf.clone(), Value::Length(corrhgt)) {
        Ok(Value::Length(l)) => -l,
        _ => Length::ZERO,
    }
}

/// Lay out one `Math` atom at `size` (LOCAL coordinates, `x` starting at
/// 0), returning its glyphs, any graphics `rules` it pushed (§B2 — only the
/// `Fraction`/`Radical` arms produce any; every other arm forwards its
/// children's), width, and left/right boundary class.
fn layout_math_atom(
    interp: &mut Interp,
    ctx: &Context,
    atom: &Math,
    size: Length,
) -> Result<(Vec<MathGlyph>, Vec<GraphicsElem>, Length, MathKind, MathKind), EvalError> {
    match atom {
        Math::Pure(MathElement::Char { class, big, chars })
        | Math::Pure(MathElement::CharWithKern { class, big, chars, .. }) => {
            let mut glyphs = Vec::new();
            let mut x = Length::ZERO;
            for c in chars.chars() {
                if *big {
                    push_big_char_glyph(interp, ctx, c, size, &mut glyphs, &mut x)?;
                } else {
                    push_char_glyph(interp, ctx, c, size, &mut glyphs, &mut x)?;
                }
            }
            Ok((glyphs, Vec::new(), x, *class, *class))
        }
        Math::Pure(MathElement::VariantChar { class, style, .. }) => {
            // Select the target codepoints by the CURRENT restyling
            // (`Context::math_char_class`, set by `ChangeCharClass`'s
            // layout arm below) rather than always `style.italic` — these
            // are explicit per-style codepoints the caller built
            // (`math-variant-char`), so no metrics-probe fallback (unlike
            // `resolve_variant_char`): `push_char_glyph` errors like any
            // other explicit-codepoint atom if the font can't render it.
            let text = match ctx.math_char_class {
                MathCharClass::Italic => &style.italic,
                MathCharClass::BoldItalic => &style.bold_italic,
                MathCharClass::Roman => &style.roman,
                MathCharClass::BoldRoman => &style.bold_roman,
                MathCharClass::Script => &style.script,
                MathCharClass::BoldScript => &style.bold_script,
                MathCharClass::Fraktur => &style.fraktur,
                MathCharClass::BoldFraktur => &style.bold_fraktur,
                MathCharClass::DoubleStruck => &style.double_struck,
            };
            let mut glyphs = Vec::new();
            let mut x = Length::ZERO;
            for c in text.chars() {
                push_char_glyph(interp, ctx, c, size, &mut glyphs, &mut x)?;
            }
            Ok((glyphs, Vec::new(), x, *class, *class))
        }
        Math::Pure(MathElement::VariantCharPending(s)) => {
            // One MATHCHAR token, resolved now that `ctx` (font +
            // math_char_class + both override maps) is available: first
            // try the whole-TOKEN class map (`=`, `-`, `,`, … ->
            // (replacement, MathKind)); if the token isn't there, fall back
            // to a per-char variant remap (gap 5's metrics-probe policy)
            // with `MathKind::Ord`.
            let mut glyphs = Vec::new();
            let mut x = Length::ZERO;
            if let Some((target, kind)) = ctx.math_class_map.get(s.as_str()) {
                let kind = *kind;
                let all_renderable = target
                    .chars()
                    .all(|c| math_char_available(interp, ctx, c, size));
                let chosen = if all_renderable { target.clone() } else { s.clone() };
                for c in chosen.chars() {
                    push_char_glyph(interp, ctx, c, size, &mut glyphs, &mut x)?;
                }
                return Ok((glyphs, Vec::new(), x, kind, kind));
            }
            for c in s.chars() {
                let chosen = resolve_variant_char(interp, ctx, c, size);
                push_char_glyph(interp, ctx, chosen, size, &mut glyphs, &mut x)?;
            }
            Ok((glyphs, Vec::new(), x, MathKind::Ord, MathKind::Ord))
        }
        Math::Pure(MathElement::EmbeddedText { class, body }) => {
            let v = interp.apply((**body).clone(), Value::Context(Box::new(ctx.clone())))?;
            let boxes = as_inline_boxes(v)?;
            let (glyphs, width) = math_glyphs_of_inline_boxes(&boxes);
            Ok((glyphs, Vec::new(), width, *class, *class))
        }
        Math::Group(cls1, cls2, inner) => {
            let (glyphs, rules, width, _, _) = layout_math_list(interp, ctx, inner, size)?;
            Ok((glyphs, rules, width, *cls1, *cls2))
        }
        Math::Sup(base, script) => {
            // Upstream MathSuperscript: (1) check_subscript merges a
            // base-tail `Sub` into one base + (sub, sup) pair;
            // (2) check_pull_in hands the script(s) to a base-tail
            // `PullInScripts` resolver.
            if let Some((sub_script, new_base)) = check_subscript(base) {
                if let Some((Math::PullInScripts(cls1, cls2, resolver), head)) =
                    new_base.split_last()
                {
                    return layout_pull_in_scripts(
                        interp,
                        ctx,
                        head,
                        *cls1,
                        *cls2,
                        resolver,
                        Some(&sub_script),
                        Some(script),
                        size,
                    );
                }
                // No pull-in (`{x_1}^2`): one sub+sup pair on the same base.
                let (mut glyphs, mut rules, base_width, left, _) =
                    layout_math_list(interp, ctx, &new_base, size)?;
                let mc = MathC::of(interp, ctx.math_font);
                let script_size = size * mc.script_scale();
                // §B3b-2 Edit C: re-derive ONCE (a paren base's dense math
                // kern function, if `new_base`'s trailing atom is a paren —
                // `paren_trailing_kernf`'s doc comment) and reuse it for
                // BOTH the sup and sub kerns below, mirroring
                // `lp_math_kern_scheme`'s single scheme feeding both corner
                // attachments upstream.
                let paren_kernf = paren_trailing_kernf(interp, ctx, &new_base, size);
                let (sub_glyphs, sub_rules, sub_width, _, _) =
                    layout_math_list(interp, ctx, &sub_script, script_size)?;
                let (sup_glyphs, sup_rules, sup_width, _, _) =
                    layout_math_list(interp, ctx, script, script_size)?;
                let (h_base, d_base) = glyphs_extent(&glyphs);
                let (_, d_sup) = glyphs_extent(&sup_glyphs);
                let (h_sub, _) = glyphs_extent(&sub_glyphs);
                let sup_shift_raw = mc.sup_shift_clamped(ctx.font_size, h_base, d_sup);
                let sub_shift_raw = mc.sub_shift_clamped(ctx.font_size, d_base, h_sub);
                let (sup_shift, sub_shift) =
                    mc.correct_script_gap(ctx.font_size, d_sup, h_sub, sup_shift_raw, sub_shift_raw);
                let kern = match &paren_kernf {
                    Some(kf) => dense_kern(interp, kf, sup_shift - d_sup),
                    None => superscript_kern(
                        interp, ctx, size, script_size, &glyphs, &sup_glyphs, sup_shift, h_base, d_sup,
                    ),
                };
                let sub_kern = paren_kernf
                    .as_ref()
                    .map(|kf| dense_kern(interp, kf, h_sub - d_base))
                    .unwrap_or(Length::ZERO);
                shift_and_append(
                    &mut glyphs, &mut rules, sub_glyphs, sub_rules, base_width + sub_kern, -sub_shift,
                );
                shift_and_append(
                    &mut glyphs, &mut rules, sup_glyphs, sup_rules, base_width + kern, sup_shift,
                );
                return Ok((
                    glyphs,
                    rules,
                    base_width + (sub_kern + sub_width).max(kern + sup_width),
                    left,
                    MathKind::Ord,
                ));
            }
            if let Some((Math::PullInScripts(cls1, cls2, resolver), head)) = base.split_last() {
                return layout_pull_in_scripts(
                    interp, ctx, head, *cls1, *cls2, resolver, None, Some(script), size,
                );
            }
            let (mut glyphs, mut rules, base_width, left, _) =
                layout_math_list(interp, ctx, base, size)?;
            let mc = MathC::of(interp, ctx.math_font);
            let script_size = size * mc.script_scale();
            let (script_glyphs, script_rules, script_width, _, _) =
                layout_math_list(interp, ctx, script, script_size)?;
            let (h_base, _) = glyphs_extent(&glyphs);
            let (_, d_sup) = glyphs_extent(&script_glyphs);
            let sup_shift = mc.sup_shift_clamped(ctx.font_size, h_base, d_sup);
            // §B3b-2 Edit A: a paren base has no italic correction / glyph
            // corner kern to sample (`superscript_kern`'s own last-glyph
            // sampling would hit the INNER run's last glyph, not the
            // delimiter) — its closure's dense kern REPLACES
            // `superscript_kern` outright rather than adding to it.
            let kern = match paren_trailing_kernf(interp, ctx, base, size) {
                Some(kf) => dense_kern(interp, &kf, sup_shift - d_sup),
                None => superscript_kern(
                    interp, ctx, size, script_size, &glyphs, &script_glyphs, sup_shift, h_base, d_sup,
                ),
            };
            shift_and_append(
                &mut glyphs, &mut rules, script_glyphs, script_rules, base_width + kern, sup_shift,
            );
            Ok((glyphs, rules, base_width + kern + script_width, left, MathKind::Ord))
        }
        Math::Sub(base, script) => {
            // Upstream MathSubscript: a `PullInScripts` at the base list's
            // TAIL receives the subscript itself instead of a corner script.
            if let Some((Math::PullInScripts(cls1, cls2, resolver), head)) = base.split_last() {
                return layout_pull_in_scripts(
                    interp,
                    ctx,
                    head,
                    *cls1,
                    *cls2,
                    resolver,
                    Some(script),
                    None,
                    size,
                );
            }
            let (mut glyphs, mut rules, base_width, left, _) =
                layout_math_list(interp, ctx, base, size)?;
            let mc = MathC::of(interp, ctx.math_font);
            let script_size = size * mc.script_scale();
            let (script_glyphs, script_rules, script_width, _, _) =
                layout_math_list(interp, ctx, script, script_size)?;
            let (_, d_base) = glyphs_extent(&glyphs);
            let (h_sub, _) = glyphs_extent(&script_glyphs);
            let sub_shift = mc.sub_shift_clamped(ctx.font_size, d_base, h_sub);
            // §B3b-2 Edit B: no non-paren subscript kern existed before this
            // slice (`kern = Length::ZERO` implicitly) — a paren base's
            // closure now supplies one; every other base keeps the old
            // zero-kern behavior via `paren_trailing_kernf`'s `None` arm.
            let kern = match paren_trailing_kernf(interp, ctx, base, size) {
                Some(kf) => dense_kern(interp, &kf, h_sub - d_base),
                None => Length::ZERO,
            };
            shift_and_append(
                &mut glyphs, &mut rules, script_glyphs, script_rules, base_width + kern, -sub_shift,
            );
            Ok((glyphs, rules, base_width + kern + script_width, left, MathKind::Ord))
        }
        Math::ChangeColor(_, inner) => {
            // stand-in: color restyling doesn't affect Slice-1 glyph
            // rendering yet (roadmap B) — just render the content.
            let (glyphs, rules, width, left, right) = layout_math_list(interp, ctx, inner, size)?;
            Ok((glyphs, rules, width, left, right))
        }
        Math::ChangeCharClass(cls, inner) => {
            // gap 5: no longer a layout no-op — lay `inner` out under a
            // context with `math_char_class` set to `cls`, which is what
            // `VariantCharPending`/`VariantChar`'s arms above consult.
            let ctx2 = Context {
                math_char_class: *cls,
                ..ctx.clone()
            };
            let (glyphs, rules, width, left, right) = layout_math_list(interp, &ctx2, inner, size)?;
            Ok((glyphs, rules, width, left, right))
        }
        Math::Fraction(num, den) => {
            // §B2 (`docs/plans/math-engine.md`): real numerator/denominator
            // placement (`math.ml:574-594` `numerator_baseline_height`/
            // `denominator_baseline_depth`) plus a bar `Fill` — replaces the
            // ASCII "num / den" stand-in. `num`/`den` are laid out at the
            // SAME `size` as this atom (no script-scale reduction — a
            // fraction's own numerator/denominator aren't scripts, matching
            // upstream's `convert_to_low` call with the ambient `mathctx`
            // unchanged).
            let (num_glyphs, num_rules, num_w, ..) = layout_math_list(interp, ctx, num, size)?;
            let (den_glyphs, den_rules, den_w, ..) = layout_math_list(interp, ctx, den, size)?;
            let w = num_w.max(den_w);
            // Center the narrower of the two over/under the wider
            // (`math.ml:1140-1155`'s symmetric padding).
            let num_dx = (w - num_w) * 0.5;
            let den_dx = (w - den_w) * 0.5;
            let (_, d_numer) = glyphs_extent(&num_glyphs);
            let (h_denom, _) = glyphs_extent(&den_glyphs);
            let mc = MathC::of(interp, ctx.math_font);
            let numer_shift = mc.frac_numer_shift(size, d_numer);
            let denom_shift = mc.frac_denom_shift(size, h_denom);
            let axis = mc.axis(size);
            let rule = mc.frac_rule(size);
            let mut glyphs = Vec::new();
            let mut rules = Vec::new();
            // `num dy>0` (raised above the axis), `den dy<0` (`frac_denom_
            // shift` is already signed negative — see that method's doc
            // comment) — both applied via the SAME up-positive `dy_shift`
            // `shift_and_append` uses for Sup/Sub.
            shift_and_append(
                &mut glyphs, &mut rules, num_glyphs, num_rules, num_dx, numer_shift,
            );
            shift_and_append(
                &mut glyphs, &mut rules, den_glyphs, den_rules, den_dx, denom_shift,
            );
            // The bar itself: `rect x∈[0,w], y∈[axis·s, axis·s+rule·s]`
            // (§B2's test-plan shape — a deliberate simplification of
            // upstream's own `Rectangle((xpos, ypos+h_bar+t_bar/2), (wid,
            // t_bar))`, which centers the rule on its OWN half-thickness
            // rather than sitting flush on the axis; this port picks the
            // simpler flush-on-axis placement instead).
            rules.push(GraphicsElem::Fill(
                ctx.text_color,
                rect_path((Length::ZERO, axis), (w, rule)),
            ));
            Ok((glyphs, rules, w, MathKind::Inner, MathKind::Inner))
        }
        Math::Radical(_degree, inner) => {
            // §B2: real bar metrics (`math.ml:620-626` `radical_bar_
            // metrics`) plus a ported `default_radical` checkmark `Fill`
            // (`primitives.cppo.ml:311-355`) and an overbar rect `Fill` —
            // replaces the U+221A stand-in. `RadicalWithDegree` (`_degree =
            // Some(..)`, `\sqrt[n]{..}`) stays unimplemented exactly as
            // before this slice — the degree is carried faithfully in the
            // `Math` value but silently NOT drawn, matching upstream's own
            // parity note (`math.ml:886-899`'s `failwith "unsupported"` is
            // upstream's harder failure mode; this port's own stand-in
            // policy, predating §B2, already chose "render the radicand
            // without the degree" over erroring — §B2 doesn't change that).
            let (inner_glyphs, inner_rules, inner_w, ..) =
                layout_math_list(interp, ctx, inner, size)?;
            let (h_cont, d_cont) = glyphs_extent(&inner_glyphs);
            let mc = MathC::of(interp, ctx.math_font);
            let (h_bar, t_bar, l_extra) = mc.radical_bar_metrics(size, h_cont);
            // `_nonnegdpt` (the sign's own, slightly deeper, ink extent —
            // `default_radical`'s downward checkmark stroke pads `d_cont` by
            // `size*0.1`, upstream's own `nonnegdpt`) isn't threaded into
            // this atom's reported `depth` directly: unlike upstream's own
            // `d_whole = d_cont` (`math.ml:884`, a "temporary" simplification
            // per its own comment there), this port's `layout_math_value`
            // folds every rule's `graphics_bbox` into the OUTER box's
            // height/depth (§B2's correctness fix, `PureHorzBox::Math`'s doc
            // comment), so the sign's real ink depth reaches the top-level
            // box automatically THROUGH the drawn `Fill` — no separate
            // manual accounting needed here.
            let (sign_path, sign_w, _nonnegdpt) = radical_sign_geometry(size, h_bar, t_bar, d_cont);
            let mut rules = vec![GraphicsElem::Fill(ctx.text_color, sign_path)];
            // Overbar + radicand share the same x-range right after the
            // sign (`math.ml:1163-1176`'s `hbbar`/`hbback`/`hblstC`); the
            // radicand itself stays at `dy = 0` (its own baseline), exactly
            // upstream — `h_bar` already clears it via the vertical-gap add
            // in `radical_bar_metrics`, so no raise is needed here.
            rules.push(GraphicsElem::Fill(
                ctx.text_color,
                rect_path((sign_w, h_bar), (inner_w, t_bar)),
            ));
            // `l_extra`: the extra ascender ABOVE the bar this run reports
            // to its container (upstream `h_whole = h_rad +% l_extra`,
            // `math.ml:882`) — no ink of its own, just headroom, so there's
            // no glyph/fill shape to naturally carry it. A single-point
            // "extent marker" `Fill` (a subpath with a `move_to` and no
            // further segments paints nothing — PDF's `f` on a degenerate
            // zero-length path is a no-op) reports it through the SAME
            // `graphics_bbox` fold `layout_math_value` already does for
            // every rule, without adding a new return channel just for this
            // one field.
            rules.push(GraphicsElem::Fill(
                ctx.text_color,
                Path {
                    subpaths: vec![Subpath {
                        start: (Length::ZERO, h_bar + t_bar + l_extra),
                        segs: Vec::new(),
                        closing: Closing::Open,
                    }],
                },
            ));
            let mut glyphs = Vec::new();
            for mut g in inner_glyphs {
                g.dx = sign_w + g.dx;
                glyphs.push(g);
            }
            for r in &inner_rules {
                rules.push(shift_graphics((sign_w, Length::ZERO), r));
            }
            Ok((glyphs, rules, sign_w + inner_w, MathKind::Inner, MathKind::Inner))
        }
        Math::Paren(l, r, inner) => {
            // §B3b-2: PRIMARY route is upstream's own `make_paren` closure
            // invocation (`math.ml:644-649`, `make_paren_run` above) —
            // identity (a `\paren` drawing round parens vs. an `\abs`
            // drawing vertical bars, etc.) lives ENTIRELY in the `l`/`r`
            // closures (`math.satyh`'s `paren-left`/`abs-left`/…), so
            // running them is what makes different delimiter kinds actually
            // look different. Falls back to §B3b(i)'s MATH-native
            // stretchy-variant stand-in (`paren_variant_fallback`) only if
            // either closure errors (synthetic/ill-shaped test closures, or
            // a real user error) — this fallback's own delimiter kind is
            // always `(`/`)` regardless of what was requested, exactly as
            // before this slice. Inner is laid out OUTSIDE the closure
            // route so an inner layout error still propagates normally
            // (only closure-route errors trigger the fallback); splice
            // order `lg ++ inner ++ rg` matches upstream's own
            // `LowMathParen(lpL, lpR, lmC)` (`math.ml:909`).
            let (inner_glyphs, inner_rules, inner_w, ..) =
                layout_math_list(interp, ctx, inner, size)?;
            let (h_in, d_in) = inner_ink_extent(&inner_glyphs, &inner_rules);
            let mc = MathC::of(interp, ctx.math_font);
            let axis = mc.axis(size);
            let closure_route = make_paren_run(interp, ctx, l, h_in, d_in, axis, size).and_then(
                |left| {
                    let right = make_paren_run(interp, ctx, r, h_in, d_in, axis, size)?;
                    Ok((left, right))
                },
            );
            match closure_route {
                Ok(((lg, lr, lw, _), (rg, rr, rw, _))) => {
                    let mut glyphs = Vec::new();
                    let mut rules = Vec::new();
                    let mut x = Length::ZERO;
                    append_at(&mut glyphs, &mut rules, &mut x, lg, lr, lw);
                    append_at(&mut glyphs, &mut rules, &mut x, inner_glyphs, inner_rules, inner_w);
                    append_at(&mut glyphs, &mut rules, &mut x, rg, rr, rw);
                    Ok((glyphs, rules, x, MathKind::Open, MathKind::Close))
                }
                Err(_) => paren_variant_fallback(
                    interp,
                    ctx,
                    vec![(inner_glyphs, inner_rules, inner_w)],
                    h_in,
                    d_in,
                    axis,
                    size,
                ),
            }
        }
        Math::ParenWithMiddle(l, r, m, mlstlst) => {
            // Same closure-primary/§B3b(i)-fallback policy as `Math::Paren`,
            // but ONE shared `(h_in, d_in)` over every part (the tallest
            // part's ink drives the size of every delimiter, including the
            // middle separator(s)) — mirrors upstream's own
            // `MathParenWithMiddle` fold (`math.ml:912-916`). The middle
            // closure's own kernf is DISCARDED (`math.ml:923`: `let
            // (hblstmiddle, _) = make_paren mathctx middle hC dC in ...`) —
            // a separator never tucks a script into itself.
            let mut parts = Vec::with_capacity(mlstlst.len());
            let mut h_in = Length::ZERO;
            let mut d_in = Length::ZERO;
            for part in mlstlst {
                let (part_glyphs, part_rules, part_w, ..) =
                    layout_math_list(interp, ctx, part, size)?;
                let (h, d) = inner_ink_extent(&part_glyphs, &part_rules);
                h_in = h_in.max(h);
                d_in = d_in.max(d);
                parts.push((part_glyphs, part_rules, part_w));
            }
            let mc = MathC::of(interp, ctx.math_font);
            let axis = mc.axis(size);
            let closure_route = make_paren_run(interp, ctx, l, h_in, d_in, axis, size).and_then(
                |left| {
                    let right = make_paren_run(interp, ctx, r, h_in, d_in, axis, size)?;
                    let middle = make_paren_run(interp, ctx, m, h_in, d_in, axis, size)?;
                    Ok((left, right, middle))
                },
            );
            match closure_route {
                Ok(((lg, lr, lw, _), (rg, rr, rw, _), (mg, mr, mw, _))) => {
                    let mut glyphs = Vec::new();
                    let mut rules = Vec::new();
                    let mut x = Length::ZERO;
                    append_at(&mut glyphs, &mut rules, &mut x, lg, lr, lw);
                    for (i, (part_glyphs, part_rules, part_w)) in parts.into_iter().enumerate() {
                        if i > 0 {
                            append_at(&mut glyphs, &mut rules, &mut x, mg.clone(), mr.clone(), mw);
                        }
                        append_at(&mut glyphs, &mut rules, &mut x, part_glyphs, part_rules, part_w);
                    }
                    append_at(&mut glyphs, &mut rules, &mut x, rg, rr, rw);
                    Ok((glyphs, rules, x, MathKind::Open, MathKind::Close))
                }
                Err(_) => paren_variant_fallback(interp, ctx, parts, h_in, d_in, axis, size),
            }
        }
        Math::UpperLimit(base, upper) => {
            let (mut glyphs, mut rules, base_width, left, right) =
                layout_math_list(interp, ctx, base, size)?;
            let mc = MathC::of(interp, ctx.math_font);
            let script_size = size * mc.script_scale();
            let (script_glyphs, script_rules, script_width, _, _) =
                layout_math_list(interp, ctx, upper, script_size)?;
            let (h_base, _) = glyphs_extent(&glyphs);
            let (_, d_up) = glyphs_extent(&script_glyphs);
            let up_shift = mc.upper_limit_shift(ctx.font_size, h_base, d_up);
            shift_and_append(
                &mut glyphs, &mut rules, script_glyphs, script_rules, base_width, up_shift,
            );
            Ok((glyphs, rules, base_width + script_width, left, right))
        }
        Math::LowerLimit(base, lower) => {
            let (mut glyphs, mut rules, base_width, left, right) =
                layout_math_list(interp, ctx, base, size)?;
            let mc = MathC::of(interp, ctx.math_font);
            let script_size = size * mc.script_scale();
            let (script_glyphs, script_rules, script_width, _, _) =
                layout_math_list(interp, ctx, lower, script_size)?;
            let (_, d_base) = glyphs_extent(&glyphs);
            let (h_low, _) = glyphs_extent(&script_glyphs);
            let low_shift = mc.lower_limit_shift(ctx.font_size, d_base, h_low);
            shift_and_append(
                &mut glyphs, &mut rules, script_glyphs, script_rules, base_width, -low_shift,
            );
            Ok((glyphs, rules, base_width + script_width, left, right))
        }
        Math::PullInScripts(cls1, cls2, resolver) => {
            // Not consumed by an enclosing Sub/Sup (bare `\sum` with no
            // scripts): resolver gets (None, None).
            layout_pull_in_scripts(interp, ctx, &[], *cls1, *cls2, resolver, None, None, size)
        }
    }
}

/// Append `glyphs`/`rules` (already at LOCAL coordinates relative to their
/// own run) onto `out_glyphs`/`out_rules` at the running `*x`, advancing `*x`
/// past them — the no-spacing-adjustment sibling of `layout_math_list`'s
/// per-atom loop, used by the structural stand-ins above (paren) that
/// concatenate sub-runs directly rather than through the spacing table.
/// `rules` shifts horizontally only (`shift_graphics` with a zero `dy` —
/// `append_at`'s callers never raise/lower a sub-run, only `dx`-place it;
/// contrast `shift_and_append` below, which does both).
fn append_at(
    out_glyphs: &mut Vec<MathGlyph>,
    out_rules: &mut Vec<GraphicsElem>,
    x: &mut Length,
    glyphs: Vec<MathGlyph>,
    rules: Vec<GraphicsElem>,
    width: Length,
) {
    let base_x = *x;
    for mut g in glyphs {
        g.dx = base_x + g.dx;
        out_glyphs.push(g);
    }
    for r in &rules {
        out_rules.push(shift_graphics((base_x, Length::ZERO), r));
    }
    *x = base_x + width;
}

/// Append `glyphs`/`rules` (LOCAL coordinates, from an isolated
/// `layout_math_list` call) onto `out_glyphs`/`out_rules`, shifting every
/// glyph/rule right by `dx_shift` (its base's own width — placing the
/// script/numerator/denominator/radicand right after the preceding content)
/// and up/down by `dy_shift` (`> 0` raises, `< 0` lowers) — the `Math`-atom
/// analog of Slice 1's `place_script`, which instead threads a single
/// running `x` across a flat `MathElem` list. `rules` go through the SAME
/// `shift_graphics` (`docs/plans/graphics-subsystem.md`) a standalone
/// `inline-graphics` box's `shift-graphics` primitive uses — box-local,
/// y-**up** coordinates, exactly `MathGlyph::dy`'s sign convention (§B2's
/// critical correctness note: get this sign wrong and a fraction bar/
/// radical mirrors instead of landing at the axis).
fn shift_and_append(
    out_glyphs: &mut Vec<MathGlyph>,
    out_rules: &mut Vec<GraphicsElem>,
    glyphs: Vec<MathGlyph>,
    rules: Vec<GraphicsElem>,
    dx_shift: Length,
    dy_shift: Length,
) {
    for mut g in glyphs {
        g.dx = dx_shift + g.dx;
        g.dy = g.dy + dy_shift;
        out_glyphs.push(g);
    }
    for r in &rules {
        out_rules.push(shift_graphics((dx_shift, dy_shift), r));
    }
}

/// An axis-aligned rectangle `Fill` path, box-local (y-**up**): bottom-left
/// corner `origin`, extending `size.0` right and `size.1` up. Shared by the
/// fraction bar and the radical overbar (§B2) — both are exactly this
/// shape, just at different `y`/width.
fn rect_path(origin: Point, size: (Length, Length)) -> Path {
    let (x, y) = origin;
    let (w, h) = size;
    Path {
        subpaths: vec![Subpath {
            start: (x, y),
            segs: vec![
                PathSeg::Line((x + w, y)),
                PathSeg::Line((x + w, y + h)),
                PathSeg::Line((x, y + h)),
            ],
            closing: Closing::Line,
        }],
    }
}

/// Port of `default_radical` (`primitives.cppo.ml:311-355`): the radical
/// checkmark's `GeneralPath`, plus its own natural advance (`wid`, upstream's
/// `PHGFixedGraphics`'s declared width) and `nonnegdpt` (its own depth
/// extent, upstream's declared `depth` — returned for completeness though
/// §B2's overall `Math::Radical` depth uses `d_cont` directly, matching
/// upstream's own "temporary" simplification, see that arm's call site).
/// `size` is the ambient LOCAL nesting size (upstream `fontsize`); `hgt_bar`/
/// `t_bar` come from `MathC::radical_bar_metrics`; `dpt` is the radicand's
/// own depth (a NON-NEGATIVE magnitude, this port's convention — see
/// `sup_shift_clamped`'s doc comment; upstream's signed `Length.negate dpt`
/// becomes a plain ADD of `dpt` here).
///
/// Box-local origin `(0, 0)` = this atom's own baseline-left corner (where
/// upstream's `graphics (xpos, ypos)` closure is finally called with the
/// box's placed anchor — every point below is relative to that same origin,
/// matching `PathSeg`/`Subpath`'s y-**up** convention).
fn radical_sign_geometry(
    size: Length,
    hgt_bar: Length,
    t_bar: Length,
    dpt: Length,
) -> (Path, Length, Length) {
    let w_m = size * 0.02;
    let w1 = size * 0.1;
    let w2 = size * 0.15;
    let w3 = size * 0.4;
    let w_a = size * 0.18;
    let h1 = size * 0.3;
    let h2 = size * 0.375;

    let nonnegdpt = dpt + size * 0.1;
    let l_r = hgt_bar + nonnegdpt;

    let wid = w_m + w1 + w2 + w3;
    let a1 = (h2 - h1) / w1;
    let a2 = h2 / w2;
    let a3 = l_r / w3;
    let t1 = t_bar * (1.0 + a1 * a1).sqrt();
    let t3 = t_bar * (((1.0 + a3 * a3).sqrt() - 1.0) / a3);
    let h_a = h1 + t1 + w_a * a1;
    let w_b = (l_r + t_bar - h_a - (w1 + w2 + w3 - t3 - w_a) * a3) * (-1.0 / (a2 + a3));
    let h_b = h_a - w_b * a2;

    let path = Path {
        subpaths: vec![Subpath {
            start: (wid, hgt_bar),
            segs: vec![
                PathSeg::Line((w_m + w1 + w2, -nonnegdpt)),
                PathSeg::Line((w_m + w1, -nonnegdpt + h2)),
                PathSeg::Line((w_m, -nonnegdpt + h1)),
                PathSeg::Line((w_m, -nonnegdpt + h1 + t1)),
                PathSeg::Line((w_m + w_a, -nonnegdpt + h_a)),
                PathSeg::Line((w_m + w_a + w_b, -nonnegdpt + h_b)),
                PathSeg::Line((wid - t3, hgt_bar + t_bar)),
                PathSeg::Line((wid, hgt_bar + t_bar)),
            ],
            closing: Closing::Line,
        }],
    };
    (path, wid, nonnegdpt)
}

/// Gap 6: flatten `text-in-math`'s embedded `inline-boxes` (already laid out
/// by `read_inline` against the math atom's own context) into `MathGlyph`s
/// nestable in a math run — the box-in-math bridge `layout_math_atom`'s
/// `EmbeddedText` arm needs. Mirrors `linebreak.rs`'s `natural_metrics`
/// exhaustive `PureHorzBox` walk EXACTLY (same variant list, same "what
/// advances `x`" choice per variant) so an added/renamed `PureHorzBox`
/// variant can't silently drop content here without also breaking that
/// walk. Caveats (faithful to what's actually renderable at Slice 1): only
/// `InnerString`/nested `Math` boxes contribute real glyphs (hence height/
/// depth, computed by the caller from the returned glyphs); every other
/// box kind (`Image`/`Graphics`/`Tabular`/`EmbeddedBlock`/…) keeps its
/// horizontal space but contributes no ink; text run at full (non-script)
/// size regardless of the math run's own `size` (upstream-faithful — a
/// `text-in-math` body is laid out once, by `read_inline`, before this
/// function ever sees it).
fn math_glyphs_of_inline_boxes(boxes: &[HorzBox]) -> (Vec<MathGlyph>, Length) {
    fn go(pure: &PureHorzBox, out: &mut Vec<MathGlyph>, x: &mut Length) {
        match pure {
            PureHorzBox::InnerString {
                info,
                text,
                width,
                height,
                depth,
            } => {
                out.push(MathGlyph {
                    info: info.clone(),
                    text: text.clone(),
                    gid: None,
                    dx: *x,
                    dy: Length::ZERO,
                    width: *width,
                    height: *height,
                    depth: *depth,
                });
                *x += *width;
            }
            PureHorzBox::OuterEmpty { natural, .. } => *x += *natural,
            PureHorzBox::OuterFil => {}
            PureHorzBox::FixedEmpty { width } => *x += *width,
            PureHorzBox::Image { width, .. } => *x += *width,
            PureHorzBox::Discretionary { no_break, .. } => {
                for p in no_break {
                    go(p, out, x);
                }
            }
            PureHorzBox::Graphics { width, .. } => *x += *width,
            // An unresolved `inline-graphics-outer` marker has zero width
            // (fil semantics, see the variant's doc comment) and no glyph
            // representation this walk can extract — advance past it like
            // `Image`/`Tabular` (a resolved one is an ordinary `Graphics`,
            // handled by the arm above).
            PureHorzBox::GraphicsOuter { width, .. } => *x += *width,
            PureHorzBox::Math { width, glyphs, .. } => {
                for g in glyphs {
                    let mut g = g.clone();
                    g.dx = *x + g.dx;
                    out.push(g);
                }
                *x += *width;
            }
            PureHorzBox::HookPageBreak { .. } => {}
            PureHorzBox::Tabular(tab) => *x += tab.width,
            PureHorzBox::EmbeddedBlock { width, .. } => *x += *width,
            // A frame in a math context has no glyph representation this
            // walk can extract — advance past it like `Image`/`Tabular`.
            PureHorzBox::Frame { width, .. } => *x += *width,
            PureHorzBox::FrameMarker { .. } => {}
            // Zero-width marker; no glyph representation. Same treatment
            // as `HookPageBreak`.
            PureHorzBox::Footnote { .. } => {}
        }
    }
    let mut glyphs = Vec::new();
    let mut x = Length::ZERO;
    for HorzBox::Pure(p) in boxes {
        go(p, &mut glyphs, &mut x);
    }
    (glyphs, x)
}

/// §B3b-2: `math_glyphs_of_inline_boxes`'s graphics-harvesting sibling — the
/// shape a `make_paren` closure's result needs, since a delimiter drawn via
/// `inline-graphics` (`math.satyh`'s `paren-left`/`abs-left`/…, `fill`/
/// `stroke` a path) carries its ink as a `PureHorzBox::Graphics` box, not a
/// `MathGlyph`. Same exhaustive walk as `math_glyphs_of_inline_boxes` (do
/// NOT modify that function — every OTHER caller still wants glyphs-only,
/// e.g. `EmbeddedText`), but additionally harvests `Graphics::elems` (`dx`-
/// shifted via `shift_graphics`, the box's own local-origin convention —
/// see `PureHorzBox::Graphics`'s doc comment) and forwards BOTH the glyphs
/// AND `rules` out of any nested `PureHorzBox::Math` box (a paren closure
/// could, in principle, embed one via `text-in-math`/`embed-math`).
fn math_boxes_of_inline_boxes(boxes: &[HorzBox]) -> (Vec<MathGlyph>, Vec<GraphicsElem>, Length) {
    fn go(pure: &PureHorzBox, out: &mut Vec<MathGlyph>, rules: &mut Vec<GraphicsElem>, x: &mut Length) {
        match pure {
            PureHorzBox::InnerString {
                info,
                text,
                width,
                height,
                depth,
            } => {
                out.push(MathGlyph {
                    info: info.clone(),
                    text: text.clone(),
                    gid: None,
                    dx: *x,
                    dy: Length::ZERO,
                    width: *width,
                    height: *height,
                    depth: *depth,
                });
                *x += *width;
            }
            PureHorzBox::OuterEmpty { natural, .. } => *x += *natural,
            PureHorzBox::OuterFil => {}
            PureHorzBox::FixedEmpty { width } => *x += *width,
            PureHorzBox::Image { width, .. } => *x += *width,
            PureHorzBox::Discretionary { no_break, .. } => {
                for p in no_break {
                    go(p, out, rules, x);
                }
            }
            PureHorzBox::Graphics { width, elems, .. } => {
                for e in elems {
                    rules.push(shift_graphics((*x, Length::ZERO), e));
                }
                *x += *width;
            }
            // See `math_glyphs_of_inline_boxes`'s matching arm.
            PureHorzBox::GraphicsOuter { width, .. } => *x += *width,
            PureHorzBox::Math {
                width,
                glyphs,
                rules: inner_rules,
                ..
            } => {
                for g in glyphs {
                    let mut g = g.clone();
                    g.dx = *x + g.dx;
                    out.push(g);
                }
                for r in inner_rules {
                    rules.push(shift_graphics((*x, Length::ZERO), r));
                }
                *x += *width;
            }
            PureHorzBox::HookPageBreak { .. } => {}
            PureHorzBox::Tabular(tab) => *x += tab.width,
            PureHorzBox::EmbeddedBlock { width, .. } => *x += *width,
            // See `math_glyphs_of_inline_boxes`'s matching arm.
            PureHorzBox::Frame { width, .. } => *x += *width,
            PureHorzBox::FrameMarker { .. } => {}
            // Zero-width marker; no glyph/graphics representation.
            PureHorzBox::Footnote { .. } => {}
        }
    }
    let mut glyphs = Vec::new();
    let mut rules = Vec::new();
    let mut x = Length::ZERO;
    for HorzBox::Pure(p) in boxes {
        go(p, &mut glyphs, &mut rules, &mut x);
    }
    (glyphs, rules, x)
}

// ============================================================================
// docs/plans/context-box-prims.md §Slice 1 (rows 1-10): the context-setter
// + box-combinator prims `code.satyh`/`itemize.satyh` need.
// ============================================================================

/// The inverse of `as_color` (mirrors `evalUtil.ml:124`'s `get_color` the
/// other way) — `get-text-color`'s result, which `itemize.satyh` feeds
/// straight into `fill`, so the tag/payload shape must match `as_color`
/// exactly (see that primitive's doc comment).
fn make_color_value(c: Color) -> Value {
    match c {
        Color::Gray(g) => Value::Ctor("Gray".to_string(), Some(Box::new(Value::Float(g)))),
        Color::Rgb(r, g, b) => Value::Ctor(
            "RGB".to_string(),
            Some(Box::new(Value::Tuple(vec![
                Value::Float(r),
                Value::Float(g),
                Value::Float(b),
            ]))),
        ),
        Color::Cmyk(c, m, y, k) => Value::Ctor(
            "CMYK".to_string(),
            Some(Box::new(Value::Tuple(vec![
                Value::Float(c),
                Value::Float(m),
                Value::Float(y),
                Value::Float(k),
            ]))),
        ),
    }
}

/// `font` = `Value::Tuple([string, float, float])` in `(abbrev, size_ratio,
/// rising_ratio)` order (vminst.ml's `tFONT`) — `set-font`'s second argument.
fn as_font(v: Value) -> Result<(String, f64, f64), EvalError> {
    match v {
        Value::Tuple(vs) if vs.len() == 3 => {
            let mut it = vs.into_iter();
            let abbrev = as_str(it.next().unwrap())?;
            let size_ratio = as_float(it.next().unwrap())?;
            let rising_ratio = as_float(it.next().unwrap())?;
            Ok((abbrev, size_ratio, rising_ratio))
        }
        other => eval_error(format!(
            "expected a font (string * float * float), got {}",
            other.type_name()
        )),
    }
}

/// `set-text-color : color -> context -> context` (vminst.ml:1603) —
/// FAITHFUL store (`Context::text_color`, the `set-font-size` shape); glyph-
/// color *rendering* (emitting `rg`/`g` before `Tj` in both PDF writers) is
/// a small follow-on, not needed for the field to round-trip.
fn prim_set_text_color(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let color = as_color(args.pop().unwrap())?;
    Ok(Value::Context(Box::new(Context {
        text_color: color,
        ..ctx
    })))
}

/// `get-text-color : context -> color` (vminst.ml:1618) — FAITHFUL and
/// load-bearing: `itemize.satyh`'s `make-bullet` feeds this straight into
/// `fill color (Gr.circle …)`, so it must round-trip exactly what
/// `set-text-color` stored (see `make_color_value`'s doc comment).
fn prim_get_text_color(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    Ok(make_color_value(ctx.text_color))
}

/// `set-hyphen-penalty : int -> context -> context` (vminst.ml:1692) —
/// FAITHFUL store (`Context::hyphen_badness`); no consumer yet (this port
/// has no hyphenation — `code.satyh` sets this to `100000` specifically to
/// *disable* hyphenation, already this port's behavior regardless).
fn prim_set_hyphen_penalty(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let n = as_int(args.pop().unwrap())?;
    Ok(Value::Context(Box::new(Context {
        hyphen_badness: n,
        ..ctx
    })))
}

/// `set-space-ratio : float -> float -> float -> context -> context`
/// (vminst.ml:1309), params `(natural, shrink, stretch)` — FAITHFUL store
/// (`Context::space_natural`/`space_shrink`/`space_stretch`, clamped to
/// `>= 0.0` like upstream); interword-glue *sizing* still uses the line
/// breaker's own fixed ratios until `docs/plans/text-rendering.md` wires
/// these in (see `context.rs`'s field doc comments).
fn prim_set_space_ratio(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let stretch = as_float(args.pop().unwrap())?.max(0.0);
    let shrink = as_float(args.pop().unwrap())?.max(0.0);
    let natural = as_float(args.pop().unwrap())?.max(0.0);
    Ok(Value::Context(Box::new(Context {
        space_natural: natural,
        space_shrink: shrink,
        space_stretch: stretch,
        ..ctx
    })))
}

/// `split-into-lines : string -> (int * string) list` (vminst.ml:2269) —
/// FAITHFUL: splits on `'\n'` and, per line, counts the leading ASCII spaces
/// `i` and returns `(i, rest_after_indent)` — exactly `evalUtil.ml:36`'s
/// `chop_space_indent`. Pure string op: no context, no box, no new type.
fn prim_split_into_lines(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let s = as_str(args.pop().unwrap())?;
    let mut out = Vec::new();
    for line in s.split('\n') {
        let indent = line.chars().take_while(|c| *c == ' ').count();
        let rest: String = line.chars().skip(indent).collect();
        out.push(Value::Tuple(vec![
            Value::Int(indent as i64),
            Value::Str(rest),
        ]));
    }
    Ok(Value::List(out))
}

/// Shift every content box in `block`'s `Line`s right by `pad_l`
/// (`block-frame-breakable`'s left-indent, `docs/plans/context-box-prims.md`
/// §4 point 4) — the simplest of the two options that section names:
/// adjusting each box's own `x` offset directly rather than prepending an
/// extra `FixedEmpty` box (`Skip`s carry no `x` offsets to shift, so they
/// pass through unchanged).
fn indent_left(block: Vec<VertBox>, pad_l: Length) -> Vec<VertBox> {
    block
        .into_iter()
        .map(|vb| match vb {
            VertBox::Line {
                height,
                depth,
                leading,
                contents,
            } => VertBox::Line {
                height,
                depth,
                leading,
                contents: contents.into_iter().map(|(x, bx)| (x + pad_l, bx)).collect(),
            },
            // `Skip`/`ClearPage`/`HookPageBreak` carry no `x` offsets to shift.
            other => other,
        })
        .collect()
}

/// `block-frame-breakable : context -> paddings -> deco-set -> (context ->
/// block-boxes) -> block-boxes` (vminst.ml:1090) — the `inline-frame-outer`
/// playbook, one dimension up (`docs/plans/context-box-prims.md` §4 +
/// hooks-annotations-crossref.md §D): `paddingL`/`paddingR` shrink the inner
/// `reducef` closure's context width, and the result is indented and
/// top/bottom-padded with plain `Skip`s (unchanged from before this slice),
/// now bracketed by a `FrameStart(id)`/`FrameEnd(id)` marker pair — the
/// frame's pads/width/deco-set are interned into `interp.decos` under `id`
/// (`DecoEntry::Block`), and `fire_hooks`'s block-fragment pass fires
/// `decoS` once the frame's whole single-page fragment is placed (§D's first
/// cut: multi-page fragments/`decoH`/`decoM`/`decoT` are a documented
/// follow-up, see `fire_hooks`'s doc comment).
fn prim_block_frame_breakable(
    interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let k = args.pop().unwrap();
    let decoset = as_decoset(args.pop().unwrap())?;
    let (pad_l, pad_r, pad_t, pad_b) = as_paddings(args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    let id = DecoId(interp.decos.len());
    interp.decos.push(DecoEntry::Block {
        pads: Paddings { l: pad_l, r: pad_r, t: pad_t, b: pad_b },
        width: ctx.paragraph_width,
        decoset,
    });
    let inner_ctx = Context {
        paragraph_width: ctx.paragraph_width - pad_l - pad_r,
        ..ctx
    };
    let inner = as_block_boxes(interp.apply(k, Value::Context(Box::new(inner_ctx)))?)?;
    let indented = indent_left(inner, pad_l);
    let mut out = Vec::with_capacity(indented.len() + 4);
    out.push(VertBox::FrameStart(id));
    out.push(VertBox::Skip(pad_t));
    out.extend(indented);
    out.push(VertBox::Skip(pad_b));
    out.push(VertBox::FrameEnd(id));
    Ok(Value::BlockBoxes(out))
}

/// `embed-block-top : context -> length -> (context -> block-boxes) ->
/// inline-boxes` (vminst.ml:1145) — STAND-IN: the `reducef` closure is
/// applied at a sub-context whose `paragraph_width` is the given `wid`
/// (upstream's own width, faithfully), and the resulting block is wrapped in
/// a `PureHorzBox::EmbeddedBlock` sized by `measure_block`
/// (`satysfi-backend`) — top-aligned (upstream's exact first-line-baseline
/// `adjust_to_first_line` is a roadmap refinement; see `satysfi-pdf`'s
/// `place_embedded_block` for the rendering side of this approximation).
fn prim_embed_block_top(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let k = args.pop().unwrap();
    let wid = as_length(args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    let inner_ctx = Context {
        paragraph_width: wid,
        ..ctx
    };
    let block = as_block_boxes(interp.apply(k, Value::Context(Box::new(inner_ctx)))?)?;
    let (height, depth) = measure_block(&block);
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::EmbeddedBlock {
            width: wid,
            height,
            depth,
            block,
        },
    )]))
}

/// `embed-block-bottom : context -> length -> (context -> block-boxes) ->
/// inline-boxes` (vminst.ml:1185) — the `embed-block-top` sibling upstream
/// distinguishes only by which end of the solidified block its baseline
/// aligns to (`adjust_to_first_line` vs. `adjust_to_last_line`); since
/// `prim_embed_block_top` above is already a STAND-IN that skips that
/// distinction (both ends fold into one `measure_block` sum), this is the
/// exact same construction — kept as its own function/registration entry
/// only to keep the name and arity faithful to v0.0.6, not because the
/// bodies differ.
fn prim_embed_block_bottom(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let k = args.pop().unwrap();
    let wid = as_length(args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    let inner_ctx = Context {
        paragraph_width: wid,
        ..ctx
    };
    let block = as_block_boxes(interp.apply(k, Value::Context(Box::new(inner_ctx)))?)?;
    let (height, depth) = measure_block(&block);
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::EmbeddedBlock {
            width: wid,
            height,
            depth,
            block,
        },
    )]))
}

/// `line-stack-bottom : inline-boxes list -> inline-boxes` (vminst.ml:1229,
/// `evalUtil.ml`'s `make_line_stack`) — FAITHFUL: each `inline-boxes` in the
/// list becomes exactly one line, fit (not broken) to the widest line's
/// natural width via `fit_cell` (this port's `LineBreak.fit`, already used
/// by the tabular grid solver — same "no `Context`, `natural_metrics`
/// height/depth" fallback upstream's `make_line_stack` needs since it too
/// has no context to lean on). Lines are stacked with zero extra margin
/// (upstream's `VertParagraph`s all have `margin_top`/`margin_bottom =
/// None`): each line's `leading` is set to the previous line's depth plus
/// this line's height, so consecutive baselines sit exactly
/// `prev_depth + this_height` apart (see `pagebreak.rs`'s
/// `leading.max(height)` placement formula — this choice makes that `max`
/// always resolve to our computed `leading`). Like `embed-block-top`, the
/// upstream `-top`/`-bottom` split (`adjust_to_first_line` vs.
/// `adjust_to_last_line`) collapses to one shape here; `line-stack-top`
/// isn't registered because nothing in this sweep's target packages calls
/// it, but it would be this same construction too.
fn prim_line_stack_bottom(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let hblstlst = as_list(args.pop().unwrap())?
        .into_iter()
        .map(as_inline_boxes)
        .collect::<Result<Vec<_>, _>>()?;
    let wid = hblstlst
        .iter()
        .map(|hbs| natural_metrics(hbs).0)
        .fold(Length::ZERO, |acc, w| if w > acc { w } else { acc });
    let mut block = Vec::with_capacity(hblstlst.len());
    let mut prev_depth = Length::ZERO;
    for (idx, hbs) in hblstlst.into_iter().enumerate() {
        let (contents, height, depth) = fit_cell(hbs, wid);
        let leading = if idx == 0 {
            height + depth
        } else {
            prev_depth + height
        };
        block.push(VertBox::Line {
            height,
            depth,
            leading,
            contents,
        });
        prev_depth = depth;
    }
    let (height, depth) = measure_block(&block);
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::EmbeddedBlock {
            width: wid,
            height,
            depth,
            block,
        },
    )]))
}

/// `add-footnote : block-boxes -> inline-boxes` (vminst.ml:1130
/// `BackendAddFootnote`) — FAITHFUL: wraps the block in a zero-metric
/// `PureHorzBox::Footnote` marker (upstream `PHGFootnote`,
/// vminstdef.yaml:1034-1044; the upstream body's `PageBreak.solidify` is a
/// no-op here because this port's block-boxes are already solid
/// `Vec<VertBox>`). `chop_page` (satysfi-backend) extracts the marker when
/// its line is committed to a page, reserves the stack's height at the
/// column bottom, and places the block bottom-aligned there — see that
/// function's doc comment. The cross-trial `changed`-flag protocol
/// `footnote-scheme.satyh` layers on top rides the already-landed crossref
/// fixpoint (docs/plans/hooks-annotations-crossref.md §F).
fn prim_add_footnote(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let block = as_block_boxes(args.pop().unwrap())?;
    Ok(Value::InlineBoxes(vec![HorzBox::Pure(
        PureHorzBox::Footnote { block },
    )]))
}

/// `set-font : script -> font -> context -> context` (vminst.ml:1463) —
/// D1b (`docs/plans/text-rendering.md` §1b): real per-script wiring.
/// `abbrev` resolves through the font metrics provider's registry first
/// (`FontMetrics::resolve_font_abbrev` — a real `TtfFontStore` built from
/// `fonts.satysfi-hash`, D1a), falling back to the milestone-1 3-face name
/// heuristic (`resolve_font_abbrev` free fn) when the provider has no
/// registry entry for it (every pre-D1 provider, or an abbrev the config
/// doesn't name) — never an error, matching this port's existing
/// accept-and-degrade stance on unresolvable font names.
///
/// **Resolution rule (back-compat critical).** `Latin`-script text keeps
/// reading `Context::font` directly rather than `font_scheme[Latin]` (see
/// that field's doc comment) — `set-font Latin f` therefore writes BOTH so
/// the two stay in sync, but `set-font` on any OTHER script only touches
/// `font_scheme`, leaving `ctx.font` (and hence `set-font-key`/`\bold`/
/// `\emph`, which only ever read `ctx.font`) untouched.
fn prim_set_font(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let mut ctx = as_context(args.pop().unwrap())?;
    let (abbrev, size_ratio, rising_ratio) = as_font(args.pop().unwrap())?;
    let script = as_script(args.pop().unwrap())?;
    let font = interp
        .metrics
        .resolve_font_abbrev(&abbrev)
        .unwrap_or_else(|| resolve_font_abbrev(&abbrev));
    ctx.font_scheme[script as usize] = ScriptFont {
        font,
        ratio: size_ratio,
        rising: rising_ratio,
    };
    if script == Script::Latin {
        ctx.font = font;
    }
    Ok(Value::Context(Box::new(ctx)))
}

/// `set-code-text-command : [string] inline-cmd -> context -> context`
/// (`stdja:116`; orphan #4, `docs/plans/build-order-to-stdja.md` — no
/// vminst.ml entry to cite). STAND-IN, same shape as `set-math-command`/
/// `set-math-font` above: `(command \cmd)` (`docs/plans/class-signature-
/// lang-gaps.md` gap 1, already landed) means a real program CAN now build
/// a `[string] inline-cmd` value to pass here — but `Context`
/// (`satysfi-backend`) still cannot hold an arbitrary lang-side `Value`
/// without a reverse crate dependency, and the one seam this codebase uses
/// for that indirection (`Interp::hooks`'s ID-table, `eval.rs`) sits outside
/// this slice's file boundary — so the command argument is accepted (to
/// keep the arity/signature faithful) and dropped.
fn prim_set_code_text_command(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let _cmd = args.pop().unwrap();
    Ok(Value::Context(Box::new(ctx)))
}

/// `get-natural-length : block-boxes -> length` (vminst.ml:2040) —
/// FAITHFUL: `get-natural-width`'s block sibling (`get-natural-width` itself
/// is a `pervasives.satyh` wrapper over `get-natural-metrics`, not a
/// primitive). A block's own "natural length" is its total vertical extent
/// — `measure_block`'s two components (height above the nominal top, depth
/// of the last line) summed into one length.
fn prim_get_natural_length(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let bb = as_block_boxes(args.pop().unwrap())?;
    let (height, depth) = measure_block(&bb);
    Ok(Value::Length(height + depth))
}

/// `set-dominant-wide-script : script -> context -> context`
/// (vminst.ml:1511 `PrimitiveSetDominantWideScript`) — FAITHFUL store
/// (context-box-prims.md §C landed): consumed by `get-dominant-wide-script`
/// now, by CJK script normalization (group D) later.
fn prim_set_dominant_wide_script(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let dominant_wide_script = as_script(args.pop().unwrap())?;
    Ok(Value::Context(Box::new(Context { dominant_wide_script, ..ctx })))
}

/// `set-dominant-narrow-script : script -> context -> context`
/// (vminst.ml:1539) — FAITHFUL store, mirror of the wide setter.
fn prim_set_dominant_narrow_script(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let dominant_narrow_script = as_script(args.pop().unwrap())?;
    Ok(Value::Context(Box::new(Context { dominant_narrow_script, ..ctx })))
}

/// `set-language : script -> language -> context -> context`
/// (vminst.ml:1568 `PrimitiveSetLangSys`) — FAITHFUL per-script map insert
/// (`langsys_scheme |> ScriptSchemeMap.add script langsys` upstream; a
/// 4-slot array write here).
fn prim_set_language(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let langsys = as_language(args.pop().unwrap())?;
    let script = as_script(args.pop().unwrap())?;
    let mut langsys_scheme = ctx.langsys_scheme;
    langsys_scheme[script as usize] = langsys;
    Ok(Value::Context(Box::new(Context { langsys_scheme, ..ctx })))
}

/// `get-dominant-wide-script : context -> script` (vminst.ml:1526) — FAITHFUL.
fn prim_get_dominant_wide_script(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    Ok(make_script_value(ctx.dominant_wide_script))
}

/// `get-dominant-narrow-script : context -> script` (vminst.ml:1555) — FAITHFUL.
fn prim_get_dominant_narrow_script(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    Ok(make_script_value(ctx.dominant_narrow_script))
}

/// `get-language : script -> context -> language` (vminst.ml:1587
/// `PrimitiveGetLangSys`) — FAITHFUL. Upstream routes through
/// `get_language_system`, whose `normalize_script` step is the identity on
/// every script a VALUE can carry (only the char-decoder-internal
/// CommonNarrow/CommonWide/Inherited normalize, horzBox.ml:470-479), so
/// this is a plain indexed read; absent-entry default `NoLanguageSystem`
/// is baked into the array's initial value.
fn prim_get_language(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let script = as_script(args.pop().unwrap())?;
    Ok(make_language_value(ctx.langsys_scheme[script as usize]))
}

/// `set-every-word-break : inline-boxes -> inline-boxes -> context -> context`
/// (vminst.ml:3007 `PrimitiveSetEveryWordBreak`) — sets the inline-boxes
/// inserted before/after every inter-word break (mdja.satyh uses it for a
/// CJK word-break strut). STAND-IN: accepted and dropped (no per-context
/// every-word-break state yet), same pattern as `prim_set_language` above.
fn prim_set_every_word_break(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let ctx = as_context(args.pop().unwrap())?;
    let _after = args.pop().unwrap();
    let _before = args.pop().unwrap();
    Ok(Value::Context(Box::new(ctx)))
}

/// `register-outline : (int * string * string * bool) list -> unit`
/// (vminstdef.yaml:2794 `BackendRegisterOutline`) — FAITHFUL: upstream
/// REPLACES the whole registered list (`outline.ml`: `registered_outline :=
/// ol`), it does not append; and it is callable anywhere (no
/// during-page-break gate — upstream's `Outline.register` has no `State`
/// check). Keys resolve through [`Interp::dest_name`] (upstream
/// `make_entry`'s `NamedDest.get key`).
fn prim_register_outline(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let entries = as_list(args.pop().unwrap())?;
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        let Value::Tuple(vs) = e else {
            return eval_error("register-outline expects a list of (int * string * string * bool)");
        };
        if vs.len() != 4 {
            return eval_error("register-outline expects 4-tuples (level, text, key, is-open)");
        }
        let mut it = vs.into_iter();
        let level = as_int(it.next().unwrap())?;
        let text = as_str(it.next().unwrap())?;
        let key = as_str(it.next().unwrap())?;
        let is_open = as_bool(it.next().unwrap())?;
        let dest_name = interp.dest_name(&key);
        out.push(OutlineEntry { level, text, dest_name, is_open });
    }
    interp.outline = out; // replace, not extend
    Ok(Value::Unit)
}

/// Recursive `extract_one` helper for [`prim_extract_string`] — mirrors
/// `horzBox.ml`'s `extract_string`'s `extract_one`: an `InnerString`
/// contributes its own text, a `Discretionary` recurses into `no_break`
/// (the "not yet broken" reading), every other box contributes nothing.
/// This port's box vocabulary has no separate Rising/Frame/ScriptGuard
/// wrapper (`inline-frame-breakable` et al. already flatten their padding
/// into the same flat `Vec<HorzBox>` — see `prim_inline_frame_breakable`),
/// so there is nothing else to recurse into.
fn extract_string_pure_one(phb: &PureHorzBox) -> String {
    match phb {
        PureHorzBox::InnerString { text, .. } => text.clone(),
        PureHorzBox::Discretionary { no_break, .. } => {
            no_break.iter().map(extract_string_pure_one).collect()
        }
        // Upstream `extract_string` recurses into frames.
        PureHorzBox::Frame { contents, .. } => {
            contents.iter().map(|(_, b)| extract_string_pure_one(b)).collect()
        }
        _ => String::new(),
    }
}

fn extract_string_one(hb: &HorzBox) -> String {
    match hb {
        HorzBox::Pure(phb) => extract_string_pure_one(phb),
    }
}

/// `extract-string : inline-boxes -> string` (vminstdef.yaml:1565
/// `PrimitiveExtract`) — FAITHFUL (see [`extract_string_one`]).
fn prim_extract_string(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let boxes = as_inline_boxes(args.pop().unwrap())?;
    let s: String = boxes.iter().map(extract_string_one).collect();
    Ok(Value::Str(s))
}

/// `get-initial-text-info : unit -> text-info` (vminst.ml:953
/// `TextGetInitialTextModeContext`) — FAITHFUL:
/// `TextBackend.get_initial_text_mode_context` is `{ indent = 0;
/// escape_list = [] }` (textBackend.ml:9-12); escape_list is omitted from
/// the port's `TextInfo` (see its doc comment).
fn prim_get_initial_text_info(
    _interp: &mut Interp,
    mut args: Vec<Value>,
) -> Result<Value, EvalError> {
    let _unit = args.pop().unwrap();
    Ok(Value::TextInfo(TextInfo { indent: 0 }))
}

/// `deepen-indent : int -> text-info -> text-info` (vminst.ml:921
/// `TextDeepenIndent`) — FAITHFUL: `indent + max i 0`
/// (`TextBackend.deepen_indent`, textBackend.ml:15-16 — the INCREMENT is
/// clamped, not the total).
fn prim_deepen_indent(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let tinfo = as_text_info(args.pop().unwrap())?;
    let i = as_int(args.pop().unwrap())?;
    Ok(Value::TextInfo(TextInfo { indent: tinfo.indent + i.max(0) }))
}

/// `break : text-info -> string` (vminst.ml:935 `TextBreak`) — FAITHFUL:
/// `"\n" ^ String.make indent ' '` (`TextBackend.get_indent`).
fn prim_break(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let tinfo = as_text_info(args.pop().unwrap())?;
    let mut s = String::with_capacity(1 + tinfo.indent as usize);
    s.push('\n');
    for _ in 0..tinfo.indent {
        s.push(' ');
    }
    Ok(Value::Str(s))
}

// ============================================================================
// docs/plans/document-page-model.md §Slice 1 unit tests: `as_page` (every
// paper-size ctor), `read_content_scheme`/`read_parts_scheme` (field
// extraction + missing-field errors). These extractors are private, so the
// tests live in-module rather than in `tests/`, same pattern as
// `crossref.rs`'s own `#[cfg(test)] mod tests`.
// ============================================================================
#[cfg(test)]
mod page_model_tests {
    use super::*;

    #[test]
    fn as_page_maps_every_nullary_ctor_to_the_right_paper_size() {
        let cases: &[(&str, PaperSize)] = &[
            ("A0Paper", PaperSize::A0),
            ("A1Paper", PaperSize::A1),
            ("A2Paper", PaperSize::A2),
            ("A3Paper", PaperSize::A3),
            ("A4Paper", PaperSize::A4),
            ("A5Paper", PaperSize::A5),
            ("USLetter", PaperSize::USLetter),
            ("USLegal", PaperSize::USLegal),
        ];
        for (name, expected) in cases {
            let v = Value::Ctor((*name).to_string(), None);
            assert_eq!(as_page(v).unwrap(), *expected, "ctor {name}");
        }
    }

    #[test]
    fn as_page_unwraps_user_defined_papers_tuple_payload() {
        let v = Value::Ctor(
            "UserDefinedPaper".to_string(),
            Some(Box::new(Value::Tuple(vec![
                Value::Length(Length::pt(100.0)),
                Value::Length(Length::pt(200.0)),
            ]))),
        );
        assert_eq!(
            as_page(v).unwrap(),
            PaperSize::UserDefined(Length::pt(100.0), Length::pt(200.0))
        );
    }

    #[test]
    fn a4_paper_dims_are_595_by_842_points() {
        let (w, h) = PaperSize::A4.dims();
        assert!((w.0 - 595.0).abs() < 1.0, "width: {}", w.0);
        assert!((h.0 - 842.0).abs() < 1.0, "height: {}", h.0);
    }

    #[test]
    fn read_content_scheme_extracts_origin_and_height() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "text-origin".to_string(),
            Value::Tuple(vec![
                Value::Length(Length::pt(10.0)),
                Value::Length(Length::pt(20.0)),
            ]),
        );
        fields.insert("text-height".to_string(), Value::Length(Length::pt(300.0)));
        let (origin, height) = read_content_scheme(Value::Record(fields)).unwrap();
        assert_eq!(origin, (Length::pt(10.0), Length::pt(20.0)));
        assert_eq!(height, Length::pt(300.0));
    }

    #[test]
    fn read_content_scheme_errors_on_a_missing_field() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "text-origin".to_string(),
            Value::Tuple(vec![Value::Length(Length::ZERO), Value::Length(Length::ZERO)]),
        );
        let err = read_content_scheme(Value::Record(fields)).unwrap_err();
        assert!(
            err.msg.contains("text-height"),
            "error should name the missing field: {}",
            err.msg
        );
    }

    #[test]
    fn read_parts_scheme_extracts_all_four_fields() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "header-origin".to_string(),
            Value::Tuple(vec![Value::Length(Length::ZERO), Value::Length(Length::ZERO)]),
        );
        fields.insert("header-content".to_string(), Value::BlockBoxes(Vec::new()));
        fields.insert(
            "footer-origin".to_string(),
            Value::Tuple(vec![
                Value::Length(Length::pt(1.0)),
                Value::Length(Length::pt(2.0)),
            ]),
        );
        fields.insert("footer-content".to_string(), Value::BlockBoxes(Vec::new()));
        let (horg, hbb, forg, fbb) = read_parts_scheme(Value::Record(fields)).unwrap();
        assert_eq!(horg, (Length::ZERO, Length::ZERO));
        assert!(hbb.is_empty());
        assert_eq!(forg, (Length::pt(1.0), Length::pt(2.0)));
        assert!(fbb.is_empty());
    }

    #[test]
    fn read_parts_scheme_errors_on_a_missing_field() {
        let err = read_parts_scheme(Value::Record(BTreeMap::new())).unwrap_err();
        assert!(
            err.msg.contains("header-origin"),
            "error should name the missing field: {}",
            err.msg
        );
    }
}
