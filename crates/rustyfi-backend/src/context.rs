use crate::font::FontKey;
use crate::graphics::Color;
use crate::length::Length;
use crate::math::{default_math_class_map, MathCharClass, MathKind};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Opaque handle to the context's installed math command (v0.0.6
/// `context_main.math_command`). The closure VALUE lives lang-side in
/// `Interp::math_commands` — this crate cannot depend on
/// `rustyfi_lang::Value`, so this is the same id-into-an-`Interp`-table
/// seam as `ImageId`/`HookId` (hbox.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MathCmdId(pub usize);

/// v0.0.6 `CharBasis.script`, SURFACE subset: the four constructors a
/// `script` VALUE can carry (`get_script`, evalUtil.ml:235-241; the port's
/// `script` variant decl, prim_types.rs `script_decl`). Upstream's
/// internal-only `CommonNarrow`/`CommonWide`/`Inherited` (charBasis.ml:11-13)
/// arise solely inside the char decoder; context storage never sees them.
/// Discriminants index `langsys_scheme`.
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

/// The set of Knuth–Liang hyphenation dictionaries a `Context` may have
/// installed. Deliberately a `Copy` tag with **no dependency on the
/// `hyphenation` crate** — `Context` is cloned constantly, so it cannot own a
/// `hyphenation::Standard` dictionary (~89 KiB, not cheaply clonable). The
/// dictionaries live in a process-global load-once cache in
/// `crates/rustyfi-lang/src/hyphenation.rs`, keyed by this tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HyphenLang {
    EnglishUS,
    /// en-GB. Maps to `hyphenation::Language::EnglishGB`.
    EnglishGB,
}

/// SATySFi 0.1's `math_script_level` (`dev-0-1-0 src/backend/horzBox.ml:
/// 139-142`) — how many script-nesting levels deep the current math reading
/// context sits. V0_0 never reads it (its script-size shrink is a fixed
/// per-call constant, not context-carried).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathScriptLevel {
    Base,
    Script,
    ScriptScript,
}

/// One script's font selection within a `Context::font_scheme`:
/// upstream `font_with_ratio` (`horzBox.ml`) folded into a plain struct.
/// `ratio` scales `ctx.font_size` for this script's glyphs; `rising` is a
/// further fraction-of-size baseline raise (`fontInfo.ml`'s
/// `get_font_with_ratio`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScriptFont {
    pub font: FontKey,
    pub ratio: f64,
    pub rising: f64,
}

/// The typesetting context (a subset of `context_main` in horzBox.ml).
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub font: FontKey,
    /// The dedicated math font (v0.0.6 context_main.math_font; set-math-font).
    /// Math layout measures/emits glyphs under THIS key, falling back to `font`
    /// per-glyph when it has no glyph (see primitives::math_glyph_font). The
    /// OpenType MATH-table lookups key on this same FontKey.
    pub math_font: FontKey,
    pub font_size: Length,
    /// Baseline-to-baseline distance.
    pub leading: Length,
    /// Wrap width for paragraphs.
    pub paragraph_width: Length,
    /// Extra vertical skip inserted above a paragraph
    /// (`set-paragraph-margin`'s first argument; v0.0.6
    /// `context_main.paragraph_top`, horzBox.ml:227). Emitted by
    /// `prim_line_break` as a leading `VertBox::Skip`. A skip at the
    /// very top of a page/column is discarded by `chop_page` (upstream's
    /// page-top glue suppression), so this adds no space above a page's first
    /// paragraph.
    pub paragraph_top: Length,
    /// Extra vertical skip inserted below a paragraph
    /// (`set-paragraph-margin`'s second argument; v0.0.6
    /// `context_main.paragraph_bottom`, horzBox.ml:228) — a trailing
    /// `VertBox::Skip`.
    pub paragraph_bottom: Length,
    /// A manual vertical shift applied to text set under this context
    /// (`set-manual-rising`'s argument; v0.0.6 `context_main.manual_rising`,
    /// horzBox.ml:232). Stored only: nothing downstream reads it (upstream's
    /// `PHGRising` box has no analogue here).
    pub manual_rising: Length,
    /// `set-dominant-wide-script` (v0.0.6 `context_main.dominant_wide_script`,
    /// horzBox.ml:218). Storage only; no layout consumer yet.
    pub dominant_wide_script: Script,
    /// `set-dominant-narrow-script` (horzBox.ml:219). Same status.
    pub dominant_narrow_script: Script,
    /// `set-language`/`get-language` (v0.0.6 `context_main.langsys_scheme`,
    /// horzBox.ml:216 — a script→language_system map). Stored as a dense
    /// 4-slot array indexed by `Script`'s discriminant; upstream's "absent
    /// from map" IS `NoLanguageSystem` (`get_language_system`'s default,
    /// horzBox.ml:483-487), so the empty map and `[NoLanguageSystem; 4]`
    /// are indistinguishable — no Option needed.
    pub langsys_scheme: [Language; 4],
    /// `set-font`'s per-script font/ratio/rising scheme (v0.0.6
    /// `context_main.font_scheme`, horzBox.ml:214), indexed by `Script`'s
    /// discriminant. Resolution rule (back-compat critical, see `set-font`'s
    /// doc in rustyfi-lang): `Latin`-script text reads `Context::font`
    /// directly, NOT this scheme's `Latin` slot — `set-font Latin f` writes
    /// BOTH so the two stay in sync, leaving a bare
    /// `set-font-key`/`\bold`/`\emph` (which only touches `font`) unaffected.
    pub font_scheme: [ScriptFont; 4],
    /// `set-space-ratio-between-scripts` (v0.0.6
    /// `context_main.script_space_map`, horzBox.ml:222) — the space inserted
    /// between two adjacent runs of DIFFERENT scripts, as a ratio of
    /// `font_size`. Indexed `[left script][right script]` by `Script`'s
    /// discriminant, so the map is directional: upstream keys
    /// `ScriptSpaceMap` on the ordered pair and registers the four Latin↔CJK
    /// directions separately (`primitives.cppo.ml:491-494`).
    ///
    /// **Only the NATURAL ratio is stored, because only it is observable.**
    /// The primitive takes three (natural, shrink, stretch), and upstream's
    /// `ScriptSpaceMap` really does hold all three — but
    /// `pure_space_between_scripts` spends them as
    /// `LBAtom((natural (size *% r0), size *% r1, size *% r2), _)`, whose
    /// first field is `metrics = length_info * length * length`, i.e. *(width
    /// info, height, depth)*. `r1` and `r2` land in the height and depth slots
    /// and never reach the glue's elasticity; see `primitives.rs`'s
    /// `interscript_glue` for the full argument. Keeping the two dead ratios
    /// here would imply a stretch this glue does not have.
    ///
    /// Upstream's map is SPARSE and a miss falls through to the JLreq class
    /// table and then to `adjacent_space`; this dense array cannot distinguish
    /// "absent" from "present and 0.0". That costs nothing, because the port
    /// only ever consults it at a `is_latin_cjk_boundary`, where upstream's
    /// fall-through is already unreachable — the four Latin↔CJK keys are
    /// exactly the ones the default map fills. A 0.0 entry still emits a
    /// zero-width glue rather than nothing, keeping the break opportunity
    /// upstream's `discretionary_if_breakable` wrapper grants regardless of
    /// the box's width.
    pub script_space_map: [[f64; 4]; 4],
    /// `set-text-color`/`get-text-color` (row 1-2; v0.0.6
    /// `context_main.text_color`). Copied into each run's
    /// `HorzStringInfo::color` at box-construction time — which is what both
    /// PDF writers emit their fill-color op from.
    pub text_color: Color,
    /// `set-hyphen-penalty` (row 3; v0.0.6 `context_main.hyphen_badness`).
    /// Consumed by `rustyfi-lang`'s `flush_word` injection as each injected
    /// `Discretionary`'s `penalty`, but only when `hyphen_dictionary` is
    /// `Some(_)`; with no dictionary installed it has no layout effect.
    pub hyphen_badness: i64,
    /// The installed hyphenation dictionary
    /// (`set-hyphenation-dictionary`/`load-hyphenation-dictionary`; v0.0.6
    /// `context_main.hyphen_dictionary`). `flush_word` (`rustyfi-lang`) runs
    /// the hyphenation branch **iff** this is `Some(tag)` and the run's
    /// script is `Latin`.
    pub hyphen_dictionary: Option<HyphenLang>,
    /// Minimum number of chars that must PRECEDE an accepted hyphenation
    /// break (`set-hyphen-min`'s first argument; v0.0.6
    /// `context_main.left_hyphen_min`). Default 3.
    pub left_hyphen_min: i64,
    /// Minimum number of chars that must FOLLOW it (`set-hyphen-min`'s
    /// second argument). Default 2.
    pub right_hyphen_min: i64,
    /// `set-space-ratio`'s three fields (row 4; v0.0.6
    /// `context_main.space_natural`/`space_shrink`/`space_stretch`). Each is
    /// a ratio of `font_size` DIRECTLY, not of the natural width:
    /// `text_to_boxes`'s interword glue is natural = `font_size *
    /// space_natural`, shrink/stretch likewise.
    pub space_natural: f64,
    pub space_shrink: f64,
    pub space_stretch: f64,
    /// `set-adjacent-stretch` (v0.0.6 `context_main.adjacent_stretch`) — the
    /// stretch, as a ratio of `font_size`, of the glue SATySFi puts between
    /// two DIRECTLY ADJACENT CJK characters (`convertText.ml:101`
    /// `adjacent_space`: natural 0, shrink 0, stretch `font_size * ratio`).
    /// This is what lets a Japanese line fill its column: unspaced CJK has no
    /// interword glue, so without it a CJK line's only elasticity is whatever
    /// incidental Latin spaces it happens to contain.
    pub adjacent_stretch: f64,
    /// The installed `[math] inline-cmd` applied to bare `${…}` in inline
    /// text (v0.0.6 `context_main.math_command`). `None` only for contexts
    /// built by `Context::initial` directly (unit tests) — the
    /// `get-initial-context` primitive always installs its second argument.
    pub math_command: Option<MathCmdId>,
    /// `set-code-text-command` (v0.0.6 `context_main.code_text_command`) — the
    /// `[string] inline-cmd` a backtick literal inside inline text is handed
    /// to. `None` is upstream's `DefaultCodeTextCommand`: the literal is set as
    /// ordinary text. Same id-into-`Interp` handle as `math_command`.
    pub code_text_command: Option<MathCmdId>,
    /// `\mathrm`/`\bm`/… restyling target (v0.0.6 `context_main.
    /// math_char_class`): which Mathematical- Alphanumeric style block a
    /// plain `${…}` letter resolves to. Set by `Math::ChangeCharClass`'s
    /// layout arm (`primitives.rs`), consulted by `resolve_variant_char`.
    /// Defaults to `Italic`, v0.0.6's own default.
    pub math_char_class: MathCharClass,
    /// Upstream `default_math_class_map` (`primitives.cppo.ml:465-480`):
    /// whole-TOKEN entries (`=`, `-`, `,`, …) consulted BEFORE the per-char
    /// variant lookup below. `Arc` so that cloning a `Context` (every `..ctx`
    /// spread) stays a refcount bump.
    pub math_class_map: Arc<BTreeMap<String, (String, MathKind)>>,
    /// `set-math-variant-char`'s runtime override table: `(source
    /// char, style) -> replacement char`, consulted BEFORE
    /// `default_math_variant_char`'s built-in Mathematical-Alphanumeric
    /// remap. Empty by default; copy-on-written via `Arc::make_mut`.
    pub math_variant_char_map: Arc<BTreeMap<(char, MathCharClass), char>>,
    /// V0_1-only (`enter_script`, port of `dev-0-1-0
    /// src/frontend/context.ml:52-68`). `Base` under V0_0 always.
    pub math_script_level: MathScriptLevel,
    /// Whether the current math sub-formula is laid out "cramped" (TeXbook
    /// Appendix G): set on the recursive layout `Context` for a radical's
    /// radicand, a fraction's denominator, and any subscript. Read by BOTH
    /// V0_0 and V0_1 (the bit rides the shared layout-recursion clone, not a
    /// version-gated primitive). Only consumed by `sup_shift_clamped`'s
    /// superscript shift-up formula.
    pub math_cramped: bool,
}

/// `default_script_space_map` (`primitives.cppo.ml:487-494`): the four
/// Latin↔CJK directions carry `space_latin_cjk`, every other ordered pair is
/// unset. Only the natural ratio survives into layout — see
/// [`Context::script_space_map`].
pub fn default_script_space_map() -> [[f64; 4]; 4] {
    // `let space_latin_cjk = (0.24, 0.08, 0.16)` — the 0.08 and 0.16 are the
    // shrink and stretch upstream never actually applies.
    const LATIN_CJK_NATURAL: f64 = 0.24;
    let mut m = [[0.0; 4]; 4];
    for cjk in [Script::HanIdeographic, Script::Kana] {
        m[Script::Latin as usize][cjk as usize] = LATIN_CJK_NATURAL;
        m[cjk as usize][Script::Latin as usize] = LATIN_CJK_NATURAL;
    }
    m
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
            // Default paragraph margin. Measured against the reference SATySFi
            // 0.0.11 (flake.nix): a body paragraph boundary advances 27pt =
            // leading(18) + 9, i.e. the effective paragraph margin is 9pt
            // (= font_size 12 × 0.75), NOT the 18pt an older reading of
            // primitives.cppo.ml assumed. Matching it makes the port's page
            // breaks coincide with SATySFi's across the prose corpus.
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
            script_space_map: default_script_space_map(),
            // v0.0.6's `get_pdf_mode_initial_context` (primitives.cppo.ml):
            // `text_color = DeviceGray 0.`, `hyphen_badness = 100`,
            // `space_natural = 0.33`, `space_shrink = 0.08`,
            // `space_stretch = 0.16`, `adjacent_stretch = 0.025`.
            text_color: Color::Gray(0.0),
            hyphen_badness: 100,
            // Upstream loads `dist/hyph/english.satysfi-hyph` into
            // `default_hyphen_dictionary` at startup and hands it to EVERY
            // initial context (`primitives.cppo.ml:500,607`).
            hyphen_dictionary: Some(HyphenLang::EnglishUS),
            left_hyphen_min: 3,
            right_hyphen_min: 2,
            space_natural: 0.33,
            space_shrink: 0.08,
            space_stretch: 0.16,
            // `convertText.ml:103` — inter-CJK glue stretch ratio.
            adjacent_stretch: 0.025,
            math_command: None,
            code_text_command: None,
            math_char_class: MathCharClass::Italic,
            math_class_map: Arc::new(default_math_class_map()),
            math_variant_char_map: Arc::new(BTreeMap::new()),
            math_script_level: MathScriptLevel::Base,
            math_cramped: false,
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
    /// Build a geometry from `page`'s paper dimensions. Only
    /// `paper_width`/`paper_height` are read by the PDF writer; the `text_*`
    /// fields are vestigial here — each page's real text area lives in its
    /// `PlacedLine` coordinates, set per page by the content scheme
    /// (`chop_page`'s caller).
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
