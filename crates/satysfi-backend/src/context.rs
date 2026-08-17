use crate::font::FontKey;
use crate::graphics::Color;
use crate::length::Length;
use crate::math::{default_math_class_map, MathCharClass, MathKind};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Opaque handle to the context's installed math command
/// (`get-initial-context`'s second argument / `set-math-command`; v0.0.6
/// `context_main.math_command`). The closure VALUE lives lang-side in
/// `Interp::math_commands` — this crate cannot depend on
/// `satysfi_lang::Value`, so this is the same id-into-an-`Interp`-table
/// seam as `ImageId`/`HookId` (hbox.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathCmdId(pub usize);

/// v0.0.6 `CharBasis.script`, SURFACE subset: the four constructors a
/// `script` VALUE can carry (`get_script`, evalUtil.ml:235-241; the port's
/// `script` variant decl, prim_types.rs `script_decl`). Upstream's
/// internal-only `CommonNarrow`/`CommonWide`/`Inherited` (charBasis.ml:11-13)
/// arise solely inside the char decoder — group D (text-rendering.md Slice
/// 2) adds them there if its `normalize_script` port needs them; context
/// storage never sees them. Discriminants index `langsys_scheme`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    HanIdeographic = 0,
    /// Upstream internal name `HiraganaOrKatakana`; surface ctor `Kana`.
    Kana = 1,
    Latin = 2,
    OtherScript = 3,
}

/// v0.0.6 `CharBasis.language_system` (the port's `language` variant decl).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Japanese,
    English,
    NoLanguageSystem,
}

/// SATySFi 0.1's `math_script_level` (`dev-0-1-0 src/backend/horzBox.ml:
/// 139-142`) — how many script-nesting levels deep the current math reading
/// context sits, consulted by [`Context::math_script_level`]. V0_0_6 never
/// reads this field (its script-size shrink is a fixed per-call constant,
/// not context-carried); it exists purely for V0_1's `read-math`/`enter-
/// script` (`satysfi-lang`'s `enter_script`, math-split spec §3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathScriptLevel {
    Base,
    Script,
    ScriptScript,
}

/// One script's font selection within a `Context::font_scheme` (D1b,
/// `docs/plans/text-rendering.md` §1): upstream `font_with_ratio`
/// (`horzBox.ml`) folded into a plain struct. `ratio` scales `ctx.font_size`
/// for this script's glyphs (e.g. a CJK face set at 0.88 of the Latin size,
/// stdja's `ipaexm` convention); `rising` is a further fraction-of-size
/// baseline raise (`fontInfo.ml`'s `get_font_with_ratio`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptFont {
    pub font: FontKey,
    pub ratio: f64,
    pub rising: f64,
}

/// The typesetting context (a milestone-1 subset of `context_main` in
/// horzBox.ml). Grows field by field as primitives need them.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub font: FontKey,
    /// The dedicated math font (v0.0.6 context_main.math_font; set-math-font).
    /// Math layout measures/emits glyphs under THIS key, falling back to `font`
    /// per-glyph when it has no glyph (see primitives::math_glyph_font). Seeded
    /// FontKey(0) — same as font — so a math-OTF regular face renders styled
    /// math with no further setup, base-14/non-math degrades to today. The
    /// OpenType MATH-table slice keys its lookups on this same FontKey.
    pub math_font: FontKey,
    pub font_size: Length,
    /// Baseline-to-baseline distance.
    pub leading: Length,
    /// Wrap width for paragraphs.
    pub paragraph_width: Length,
    /// Extra vertical skip inserted above a paragraph
    /// (`set-paragraph-margin`'s first argument; v0.0.6
    /// `context_main.paragraph_top`, horzBox.ml:227). Not wired into any
    /// box-producing primitive yet — a future `+p` is expected to turn this
    /// into a leading `VertBox::Skip`.
    pub paragraph_top: Length,
    /// Extra vertical skip inserted below a paragraph
    /// (`set-paragraph-margin`'s second argument; v0.0.6
    /// `context_main.paragraph_bottom`, horzBox.ml:228). Same "not wired in
    /// yet" status as `paragraph_top`.
    pub paragraph_bottom: Length,
    /// A manual vertical shift applied to text set under this context
    /// (`set-manual-rising`'s argument; v0.0.6 `context_main.manual_rising`,
    /// horzBox.ml:232). Like `paragraph_top`/`paragraph_bottom`, not yet
    /// wired into any box-producing primitive (upstream's `PHGRising` box
    /// has no analogue here yet) — `pervasives.satyh`'s `\SATySFi`/`\LaTeX`/
    /// `\TeX` set it, but this port has nothing downstream that reads it.
    pub manual_rising: Length,
    /// `set-dominant-wide-script` (v0.0.6 `context_main.dominant_wide_script`,
    /// horzBox.ml:218). FAITHFUL storage; round-trips via
    /// `get-dominant-wide-script`. The layout consumer (`normalize_script`'s
    /// CommonWide arm → CJK font selection) is group D, text-rendering.md
    /// Slice 2.
    pub dominant_wide_script: Script,
    /// `set-dominant-narrow-script` (horzBox.ml:219). Same status.
    pub dominant_narrow_script: Script,
    /// `set-language`/`get-language` (v0.0.6 `context_main.langsys_scheme`,
    /// horzBox.ml:216 — a script→language_system map). Stored as a dense
    /// 4-slot array indexed by `Script`'s discriminant; upstream's "absent
    /// from map" IS `NoLanguageSystem` (`get_language_system`'s default,
    /// horzBox.ml:483-487), so the empty map and `[NoLanguageSystem; 4]`
    /// are indistinguishable — no Option needed. Consumer: OpenType
    /// language-system shaping, group D.
    pub langsys_scheme: [Language; 4],
    /// `set-font`'s per-script font/ratio/rising scheme (D1b; v0.0.6
    /// `context_main.font_scheme`, horzBox.ml:214), indexed by `Script`'s
    /// discriminant (`char_script`'s classification of a run). Resolution
    /// rule (back-compat critical, see `set-font`'s doc in satysfi-lang):
    /// `Latin`-script text keeps reading `Context::font` directly, NOT this
    /// scheme's `Latin` slot — `set-font Latin f` writes BOTH so the two
    /// stay in sync, but every pre-D1 `set-font-key`/`\bold`/`\emph` call
    /// (which only ever touches `font`) is unaffected. Seeded to `{ font:
    /// FontKey(0), ratio: 1.0, rising: 0.0 }` for all four scripts —
    /// identical to today's single-font behavior until a `set-font` call or
    /// a configured `default-font.satysfi-hash` `scripts` block overlays it.
    pub font_scheme: [ScriptFont; 4],
    /// `set-text-color`/`get-text-color` (docs/plans/context-box-prims.md
    /// §Slice 1 row 1-2; v0.0.6 `context_main.text_color`). FAITHFUL storage
    /// — round-trips exactly (`itemize.satyh`'s bullet `fill` color depends
    /// on it) — but not yet *consumed* by either PDF writer's glyph
    /// emission, which still always draws in black.
    pub text_color: Color,
    /// `set-hyphen-penalty` (row 3; v0.0.6 `context_main.hyphen_badness`).
    /// Stored faithfully; no consumer yet (this port has no hyphenation).
    pub hyphen_badness: i64,
    /// `set-space-ratio`'s three fields (row 4; v0.0.6
    /// `context_main.space_natural`/`space_shrink`/`space_stretch`). Stored
    /// faithfully; interword-glue sizing still uses the line breaker's own
    /// fixed ratios until `docs/plans/text-rendering.md` wires these in.
    pub space_natural: f64,
    pub space_shrink: f64,
    pub space_stretch: f64,
    /// The installed `[math] inline-cmd` applied to bare `${…}` in inline
    /// text (v0.0.6 `context_main.math_command`). `None` only for contexts
    /// built by `Context::initial` directly (unit tests) — the
    /// `get-initial-context` primitive always installs its second argument.
    pub math_command: Option<MathCmdId>,
    /// `\mathrm`/`\bm`/… restyling target (v0.0.6 `context_main.
    /// math_char_class`, `docs/plans/math-engine.md` §F): which Mathematical-
    /// Alphanumeric style block a plain `${…}` letter resolves to. Set by
    /// `Math::ChangeCharClass`'s layout arm (`primitives.rs`), consulted by
    /// `resolve_variant_char`. Defaults to `Italic` — v0.0.6's own default
    /// (plain math letters are italic unless restyled).
    pub math_char_class: MathCharClass,
    /// Upstream `default_math_class_map` (`primitives.cppo.ml:465-480`):
    /// whole-TOKEN entries (`=`, `-`, `,`, …) consulted BEFORE the per-char
    /// variant lookup below. `Arc` (not per-`Context` `BTreeMap`) since every
    /// `Context` shares the same default table unless a future primitive
    /// overrides it; cloning a `Context` (routine — every `..ctx` spread)
    /// stays a cheap refcount bump.
    pub math_class_map: Arc<BTreeMap<String, (String, MathKind)>>,
    /// `set-math-variant-char`'s runtime override table (gap 7,
    /// `docs/plans/math-engine.md` §F): `(source char, style) -> replacement
    /// char`, consulted BEFORE `default_math_variant_char`'s built-in
    /// Mathematical-Alphanumeric remap. Empty by default. `Arc` for the same
    /// cheap-clone reason as `math_class_map`; `set-math-variant-char`
    /// copy-on-writes it via `Arc::make_mut`.
    pub math_variant_char_map: Arc<BTreeMap<(char, MathCharClass), char>>,
    /// V0_1-only: how many script-nesting levels deep this reading context
    /// sits (`math-split` spec §3.3's `enter_script`, port of `dev-0-1-0
    /// src/frontend/context.ml:52-68`). `Base` under V0_0_6 always — no
    /// V0_0_6 code path ever reads or bumps this field.
    pub math_script_level: MathScriptLevel,
}

impl Context {
    /// The default context `get-initial-context` hands to `document`.
    pub fn initial(paragraph_width: Length) -> Context {
        Context {
            font: FontKey(0),
            math_font: FontKey(0),
            font_size: Length::pt(12.0),
            leading: Length::pt(18.0),
            paragraph_width,
            // v0.0.6's `get_pdf_mode_initial_context`
            // (primitives.cppo.ml:514-515) defaults both to 18pt.
            paragraph_top: Length::pt(18.0),
            paragraph_bottom: Length::pt(18.0),
            manual_rising: Length::ZERO,
            dominant_wide_script: Script::OtherScript,
            dominant_narrow_script: Script::OtherScript,
            langsys_scheme: [Language::NoLanguageSystem; 4],
            font_scheme: [ScriptFont {
                font: FontKey(0),
                ratio: 1.0,
                rising: 0.0,
            }; 4],
            // v0.0.6's `get_pdf_mode_initial_context` (primitives.cppo.ml):
            // `text_color = DeviceGray 0.`, `hyphen_badness = 100`,
            // `space_natural = 0.33`, `space_shrink = 0.08`,
            // `space_stretch = 0.16`.
            text_color: Color::Gray(0.0),
            hyphen_badness: 100,
            space_natural: 0.33,
            space_shrink: 0.08,
            space_stretch: 0.16,
            math_command: None,
            math_char_class: MathCharClass::Italic,
            math_class_map: Arc::new(default_math_class_map()),
            math_variant_char_map: Arc::new(BTreeMap::new()),
            math_script_level: MathScriptLevel::Base,
        }
    }
}

/// Page geometry (A4 with even margins by default).
#[derive(Clone, Debug, PartialEq)]
pub struct PageGeometry {
    pub paper_width: Length,
    pub paper_height: Length,
    /// Top-left corner of the text area.
    pub text_origin: (Length, Length),
    pub text_width: Length,
    pub text_height: Length,
}

impl Default for PageGeometry {
    fn default() -> Self {
        let paper_width = Length::from_unit(210.0, "mm").unwrap();
        let paper_height = Length::from_unit(297.0, "mm").unwrap();
        let margin = Length::from_unit(25.0, "mm").unwrap();
        PageGeometry {
            paper_width,
            paper_height,
            text_origin: (margin, margin),
            text_width: paper_width - margin - margin,
            text_height: paper_height - margin - margin,
        }
    }
}

impl PageGeometry {
    /// Build a geometry from `page`'s paper dimensions
    /// (docs/plans/document-page-model.md §"`page` -> dimensions"). Only
    /// `paper_width`/`paper_height` are read by the PDF writer; the
    /// `text_*` fields are vestigial here — each page's real text area
    /// lives in its `PlacedLine` coordinates, set per page by the
    /// content scheme (`chop_page`'s caller), not by this geometry.
    pub fn for_paper(paper_width: Length, paper_height: Length) -> PageGeometry {
        PageGeometry {
            paper_width,
            paper_height,
            text_origin: (Length::ZERO, Length::ZERO),
            text_width: paper_width,
            text_height: paper_height,
        }
    }
}

/// v0.0.6's `page_size` (`primitives.cppo.ml:203-212`) — the paper-size
/// constant set `page-break`'s first argument selects from, the port of
/// `get_pdf_paper` (`handlePdf.ml:406`, `Pdfpaper.t` dims from
/// `pdfpaper.ml`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaperSize {
    A0,
    A1,
    A2,
    A3,
    A4,
    A5,
    USLetter,
    USLegal,
    UserDefined(Length, Length),
}

impl PaperSize {
    /// `(width, height)` in points.
    pub fn dims(&self) -> (Length, Length) {
        fn mm(w: f64, h: f64) -> (Length, Length) {
            (
                Length::from_unit(w, "mm").unwrap(),
                Length::from_unit(h, "mm").unwrap(),
            )
        }
        fn inch(w: f64, h: f64) -> (Length, Length) {
            (
                Length::from_unit(w, "inch").unwrap(),
                Length::from_unit(h, "inch").unwrap(),
            )
        }
        match *self {
            PaperSize::A0 => mm(841.0, 1189.0),
            PaperSize::A1 => mm(594.0, 841.0),
            PaperSize::A2 => mm(420.0, 594.0),
            PaperSize::A3 => mm(297.0, 420.0),
            PaperSize::A4 => mm(210.0, 297.0),
            PaperSize::A5 => mm(148.0, 210.0),
            PaperSize::USLetter => inch(8.5, 11.0),
            PaperSize::USLegal => inch(8.5, 14.0),
            PaperSize::UserDefined(w, h) => (w, h),
        }
    }
}
