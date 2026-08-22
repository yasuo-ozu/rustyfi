//! A [`FontMetrics`] provider backed by real TrueType/OpenType font files.
//! Loads up to three faces — regular, bold, oblique —
//! mapped onto the existing `FontKey(0/1/2)` convention from `base14`, and
//! measures through `ttf-parser`'s `cmap`/`hmtx`/`hhea`/`OS/2` tables instead
//! of hardcoded AFM widths.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rustyfi_backend::{
    FontKey, FontMetrics, Length, MathConstants, MathCorner, MathVariantGlyph, Script,
    VertVariantPolicy,
};
use ttf_parser::Face;

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("failed to read font file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse font {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ttf_parser::FaceParsingError,
    },
}

/// Owns the raw bytes of every distinct font file that was loaded, plus a
/// `FontKey(0/1/2) -> file` lookup that lets bold/oblique fall back to the
/// regular face without duplicating its bytes in memory (and, in the PDF
/// writer, without embedding the same font file twice).
///
/// Face ownership: rather than caching a `ttf_parser::Face<'a>` alongside the
/// `Vec<u8>` it borrows from (which needs either `unsafe` self-referential
/// storage or a crate like `owned-ttf-parser`), each accessor reparses a
/// `Face` on demand from the stored bytes. `Face::parse` only walks the sfnt
/// table directory and a few small required tables (`head`, `hhea`, `maxp`,
/// `OS/2`, ...); it does not touch glyph outlines, so its cost does not scale
/// with document size and is cheap at the milestone's scale (a handful of
/// pages, one parse per glyph lookup). This keeps `TtfFontStore` a plain,
/// safe struct.
pub struct TtfFontStore {
    files: Vec<Vec<u8>>,
    /// `FontKey(0)=regular, 1=bold, 2=oblique, 3.. = registry abbrevs` ->
    /// index into `files`. Missing bold/oblique share the regular slot
    /// (index 0). `FontRegistry::build_store` allocates one slot per
    /// configured abbrev beyond the three seeded defaults; `slots[0..3]`
    /// always stay regular/bold/oblique, so a bare `TtfFontStore::load`
    /// store has exactly 3.
    slots: Vec<usize>,
    /// Registry abbrev ("ipaexm", "Junicode-b", ...) -> the `FontKey`
    /// allocated for it by `FontRegistry::build_store`. Empty for a bare
    /// `TtfFontStore::load` (no registry involved) — `resolve_font_abbrev`
    /// then returns `None` and callers fall back to the 3-face name
    /// heuristic (`resolve_font_abbrev` free fn, rustyfi-lang).
    abbrevs: BTreeMap<String, FontKey>,
    /// The configured default `(font, ratio, rising)` per `Script`
    /// (`context::Script` as `usize`), from `default-font.satysfi-hash`'s
    /// optional `scripts` block. `None` per-slot (the default) means
    /// "no script scheme configured" — callers overlay `(ctx.font, 1.0,
    /// 0.0)` themselves, keeping today's single-font behavior.
    script_defaults: [Option<(FontKey, f64, f64)>; 4],
    /// The `FontKey` allocated for `default-font.satysfi-hash`'s optional
    /// `"math"` abbrev. `None` for a bare `TtfFontStore::load` or
    /// a registry with no `"math"` entry — `get-initial-context` then leaves
    /// `Context::math_font` at its `Context::initial` seed.
    math_default: Option<FontKey>,
}

impl TtfFontStore {
    /// Load up to three faces. `bold`/`oblique` fall back to `regular` when
    /// not given.
    pub fn load(
        regular: &Path,
        bold: Option<&Path>,
        oblique: Option<&Path>,
    ) -> Result<Self, FontError> {
        let mut files = vec![Self::read_and_validate(regular)?];
        let mut slots = vec![0usize, 0, 0];

        if let Some(path) = bold {
            files.push(Self::read_and_validate(path)?);
            slots[1] = files.len() - 1;
        }
        if let Some(path) = oblique {
            files.push(Self::read_and_validate(path)?);
            slots[2] = files.len() - 1;
        }

        Ok(TtfFontStore {
            files,
            slots,
            abbrevs: BTreeMap::new(),
            script_defaults: [None; 4],
            math_default: None,
        })
    }

    /// Builder used only by [`crate::fonts::FontRegistry::build_store`]:
    /// construct a store with the three default slots already
    /// loaded (via [`Self::load`]) plus every other configured abbrev's
    /// file appended as its own slot (deduped by canonical path against
    /// files already loaded), and the abbrev -> `FontKey` map that
    /// `resolve_font_abbrev` consults.
    pub(crate) fn from_parts(
        files: Vec<Vec<u8>>,
        slots: Vec<usize>,
        abbrevs: BTreeMap<String, FontKey>,
        script_defaults: [Option<(FontKey, f64, f64)>; 4],
        math_default: Option<FontKey>,
    ) -> Self {
        TtfFontStore {
            files,
            slots,
            abbrevs,
            script_defaults,
            math_default,
        }
    }

    pub(crate) fn read_and_validate(path: &Path) -> Result<Vec<u8>, FontError> {
        let bytes = fs::read(path).map_err(|source| FontError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        // Fail fast at load time rather than the first metrics/embedding call.
        Face::parse(&bytes, 0).map_err(|source| FontError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(bytes)
    }

    /// Clamp an arbitrary `FontKey` onto the known slots, mirroring
    /// `base14::Base14Metrics`'s treatment of out-of-range keys.
    fn key_slot(&self, font: FontKey) -> usize {
        (font.0 as usize).min(self.slots.len() - 1)
    }

    /// The physical-file index backing `font` (after bold/oblique fallback).
    /// Used by the CID embedder to dedup: two `FontKey`s that resolve to the
    /// same file are embedded (and their Type0 font object shared) once.
    pub(crate) fn file_index(&self, font: FontKey) -> usize {
        self.slots[self.key_slot(font)]
    }

    pub fn num_files(&self) -> usize {
        self.files.len()
    }

    /// Number of allocated `FontKey` slots (3 for a bare `load`; 3 + one
    /// per extra configured abbrev for a registry-built store).
    ///
    /// Only a test consumer remains (`fonts.rs`'s in-src unit tests) since
    /// the font registry landed, so this is `cfg(test)`-gated rather than a live
    /// `pub(crate)` accessor with no non-test caller.
    #[cfg(test)]
    pub(crate) fn num_slots(&self) -> usize {
        self.slots.len()
    }

    /// Raw bytes of a physical file, for `FontFile2` embedding.
    pub fn file_bytes(&self, file_index: usize) -> &[u8] {
        &self.files[file_index]
    }

    /// Resolve a registry abbrev ("ipaexm", "Junicode-b", ...) to its
    /// allocated `FontKey`, or `None` if the store has no such abbrev
    /// (either it wasn't configured, or the store came from a bare `load`).
    pub fn abbrev_key(&self, abbrev: &str) -> Option<FontKey> {
        self.abbrevs.get(abbrev).copied()
    }

    /// See the `script_defaults` field doc.
    pub fn script_default(&self, script: usize) -> Option<(FontKey, f64, f64)> {
        self.script_defaults.get(script).copied().flatten()
    }

    /// See the `math_default` field doc.
    pub(crate) fn math_font_default(&self) -> Option<FontKey> {
        self.math_default
    }

    /// Parse the face for a given font key. See the struct doc for why this
    /// reparses on every call instead of caching a `Face`.
    pub fn face(&self, font: FontKey) -> Option<Face<'_>> {
        self.face_by_file(self.file_index(font))
    }

    pub(crate) fn face_by_file(&self, file_index: usize) -> Option<Face<'_>> {
        Face::parse(self.files.get(file_index)?, 0).ok()
    }
}

impl FontMetrics for TtfFontStore {
    fn advance(&self, font: FontKey, c: char, size: Length) -> Option<Length> {
        let face = self.face(font)?;
        let gid = face.glyph_index(c)?;
        let advance = face.glyph_hor_advance(gid)? as f64;
        let units_per_em = face.units_per_em() as f64;
        Some(size * (advance / units_per_em))
    }

    fn ascender(&self, font: FontKey, size: Length) -> Length {
        let Some(face) = self.face(font) else {
            return Length::ZERO;
        };
        // `Face::ascender` already prefers the OS/2 typographic ascender over
        // hhea's when the face's `fsSelection` USE_TYPO_METRICS bit is set
        // (falling back to hhea, then to OS/2's Win ascender otherwise) —
        // the same resolution order FreeType uses. We rely on that rather
        // than re-deriving it, since it is exactly "prefer typographic
        // OS/2 values when present".
        let units_per_em = face.units_per_em() as f64;
        size * (face.ascender() as f64 / units_per_em)
    }

    fn descender(&self, font: FontKey, size: Length) -> Length {
        let Some(face) = self.face(font) else {
            return Length::ZERO;
        };
        let units_per_em = face.units_per_em() as f64;
        // ttf-parser's descender (hhea/typographic OS/2, same resolution
        // order as `ascender`) is negative — depth below the baseline —
        // while `FontMetrics::descender` wants a positive depth.
        size * (-(face.descender() as f64) / units_per_em)
    }

    fn glyph_vextent(&self, font: FontKey, c: char, size: Length) -> Option<(Length, Length)> {
        let face = self.face(font)?;
        let gid = face.glyph_index(c)?;
        // Actual glyph ink box — SATySFi's `get_glyph_metrics` (fontFormat.ml):
        // `hgt = ymax`, `dpt = ymin`. A blank glyph (space) has no bbox and
        // contributes nothing to the run's extent.
        let bbox = face.glyph_bounding_box(gid)?;
        let units_per_em = face.units_per_em() as f64;
        let height = size * (bbox.y_max as f64 / units_per_em);
        let depth = size * (-(bbox.y_min as f64) / units_per_em);
        Some((height, depth))
    }

    // ---- OpenType MATH table ---------
    //
    // Read through ttf-parser 0.25.1's `tables::math`:
    // `Face::tables().math -> Option<math::Table>` with `.constants` /
    // `.glyph_info` / `.variants`. Every `Constants` accessor except the two
    // percent-scale-downs returns a `MathValue { value: i16, device }`
    // struct, not a plain integer — hence the `mv.value` field access in
    // `r(...)` below. `GlyphInfo.italic_corrections`/`.kern_infos` are
    // fields, not methods, each with a `.get(GlyphId)` accessor;
    // `KernInfo`'s four corners are `Option<Kern>` fields.
    //
    // `math_vertical_variant`, below, consumes `Variants` itself:
    // `Variants { min_connector_overlap: u16, vertical_constructions,
    // horizontal_constructions }`; `GlyphConstruction { assembly:
    // Option<GlyphAssembly>, variants: LazyArray16<GlyphVariant> }`;
    // `GlyphVariant { variant_glyph: GlyphId, advance_measurement: u16 }`.

    fn math_constants(&self, font: FontKey) -> Option<MathConstants> {
        let face = self.face(font)?;
        let c = face.tables().math?.constants?;
        let upem = face.units_per_em() as f64;
        let r = |mv: ttf_parser::math::MathValue| mv.value as f64 / upem;
        Some(MathConstants {
            axis_height: r(c.axis_height()),
            superscript_bottom_min: r(c.superscript_bottom_min()),
            superscript_shift_up: r(c.superscript_shift_up()),
            superscript_shift_up_cramped: r(c.superscript_shift_up_cramped()),
            superscript_baseline_drop_max: r(c.superscript_baseline_drop_max()),
            subscript_top_max: r(c.subscript_top_max()),
            subscript_shift_down: r(c.subscript_shift_down()),
            subscript_baseline_drop_min: r(c.subscript_baseline_drop_min()),
            script_scale_down: c.script_percent_scale_down() as f64 / 100.0,
            script_script_scale_down: c.script_script_percent_scale_down() as f64 / 100.0,
            space_after_script: r(c.space_after_script()),
            sub_superscript_gap_min: r(c.sub_superscript_gap_min()),
            fraction_rule_thickness: r(c.fraction_rule_thickness()),
            fraction_numer_shift_up: r(c.fraction_numerator_display_style_shift_up()),
            fraction_numer_gap_min: r(c.fraction_num_display_style_gap_min()),
            fraction_denom_shift_down: r(c.fraction_denominator_display_style_shift_down()),
            fraction_denom_gap_min: r(c.fraction_denom_display_style_gap_min()),
            radical_extra_ascender: r(c.radical_extra_ascender()),
            radical_rule_thickness: r(c.radical_rule_thickness()),
            radical_vertical_gap: r(c.radical_display_style_vertical_gap()),
            upper_limit_gap_min: r(c.upper_limit_gap_min()),
            upper_limit_baseline_rise_min: r(c.upper_limit_baseline_rise_min()),
            lower_limit_gap_min: r(c.lower_limit_gap_min()),
            lower_limit_baseline_drop_min: r(c.lower_limit_baseline_drop_min()),
        })
    }

    fn italic_correction(&self, font: FontKey, c: char, size: Length) -> Option<Length> {
        let face = self.face(font)?;
        let gid = face.glyph_index(c)?;
        let mv = face.tables().math?.glyph_info?.italic_corrections?.get(gid)?;
        Some(size * (mv.value as f64 / face.units_per_em() as f64))
    }

    fn math_kern(
        &self,
        font: FontKey,
        c: char,
        size: Length,
        corner: MathCorner,
        corr: Length,
    ) -> Option<Length> {
        let face = self.face(font)?;
        let gid = face.glyph_index(c)?;
        let ki = face.tables().math?.glyph_info?.kern_infos?.get(gid)?;
        let kern = match corner {
            MathCorner::TopRight => ki.top_right,
            MathCorner::TopLeft => ki.top_left,
            MathCorner::BottomRight => ki.bottom_right,
            MathCorner::BottomLeft => ki.bottom_left,
        }?;
        let upem = face.units_per_em() as f64;
        let corr_du = (corr.0 / size.0) * upem;
        let n = kern.count();
        let mut idx = n; // default = last kern (kfinal)
        for i in 0..n {
            if corr_du < kern.height(i)?.value as f64 {
                idx = i;
                break;
            }
        }
        Some(size * (kern.kern(idx)?.value as f64 / upem))
    }

    /// Pick a vertically-grown MATH variant (`MathVariants`) of `c` per
    /// `policy` and report its real per-glyph ink metrics at `size`.
    /// Assembly-only constructions (`variants.len() == 0`, big enough
    /// stretchy delimiters in some fonts) return `None` here — they are
    /// `math_vertical_assembly`'s job.
    fn math_vertical_variant(
        &self,
        font: FontKey,
        c: char,
        size: Length,
        policy: VertVariantPolicy,
    ) -> Option<MathVariantGlyph> {
        let face = self.face(font)?;
        let gid = face.glyph_index(c)?;
        let construction = face
            .tables()
            .math?
            .variants?
            .vertical_constructions
            .get(gid)?;
        let n = construction.variants.len();
        if n == 0 {
            return None;
        }
        let upem = face.units_per_em() as f64;
        let rec = match policy {
            VertVariantPolicy::BigOp => {
                construction.variants.get(if n >= 2 { 1 } else { 0 })?
            }
            VertVariantPolicy::AtLeast(min) => {
                let min_du = (min.0 / size.0) * upem;
                let mut chosen = construction.variants.get(n - 1)?; // largest fallback
                for i in 0..n {
                    let v = construction.variants.get(i)?;
                    if v.advance_measurement as f64 >= min_du {
                        chosen = v;
                        break;
                    }
                }
                chosen
            }
        };
        let vgid = rec.variant_glyph;
        let advance = face.glyph_hor_advance(vgid)? as f64;
        let bbox = face.glyph_bounding_box(vgid)?;
        Some(MathVariantGlyph {
            gid: vgid.0,
            advance: size * (advance / upem),
            height: size * (bbox.y_max.max(0) as f64 / upem),
            depth: size * ((-(bbox.y_min.min(0) as i32)) as f64 / upem),
        })
    }

    /// Stretch `c` (via OpenType MATH `GlyphAssembly`) beyond the largest discrete
    /// `MathVariants` record by stacking the assembly's `GlyphPart`s
    /// vertically, repeating `extender` parts to reach `target`. Faithful to
    /// the OpenType "assembling glyphs" recipe (and `math.ml`'s
    /// `MathVariants`/`GlyphConstruction` reader): parts are listed
    /// bottom-to-top; every non-extender part is placed exactly once, and all
    /// extender parts are repeated the same number of times `r` (the smallest
    /// `r` whose stacked extent, at the minimum `min_connector_overlap`
    /// overlap, covers `target`). Each connection overlaps by exactly
    /// `min_connector_overlap` design units (the smallest legal overlap, which
    /// yields the LONGEST assembly for a given part count — so the result
    /// always covers `target`). Returns `(gid, dy, advance)` per placed part
    /// with `dy` the y-up box-local baseline offset (bottom part at `dy = 0`,
    /// each next part raised by the previous part's advance minus the
    /// overlap) and `advance` the part's `full_advance` scaled to `size`.
    fn math_vertical_assembly(
        &self,
        font: FontKey,
        c: char,
        size: Length,
        target: Length,
    ) -> Option<Vec<(u16, Length, Length)>> {
        let face = self.face(font)?;
        let gid = face.glyph_index(c)?;
        let variants = face.tables().math?.variants?;
        let construction = variants.vertical_constructions.get(gid)?;
        let assembly = construction.assembly?;
        let parts: Vec<ttf_parser::math::GlyphPart> = assembly.parts.into_iter().collect();
        if parts.is_empty() {
            return None;
        }
        let upem = face.units_per_em() as f64;
        let overlap_du = variants.min_connector_overlap as f64;
        // The extent of an ordered part list, in design units, at the minimum
        // (`min_connector_overlap`) overlap on every connection — i.e. the
        // longest the list can stack. `sum(full_advance) - overlap *
        // (count - 1)`.
        let extent_du = |seq: &[&ttf_parser::math::GlyphPart]| -> f64 {
            if seq.is_empty() {
                return 0.0;
            }
            let sum: f64 = seq.iter().map(|p| p.full_advance as f64).sum();
            sum - overlap_du * (seq.len() as f64 - 1.0)
        };
        let target_du = (target.0 / size.0) * upem;
        // Grow the extender repeat count `r` until the stack covers `target`
        // (or a hard cap keeps a pathological/degenerate assembly from
        // looping forever — 256 repeats is far past any real delimiter).
        let build = |r: usize| -> Vec<&ttf_parser::math::GlyphPart> {
            let mut seq: Vec<&ttf_parser::math::GlyphPart> = Vec::new();
            for p in &parts {
                let times = if p.part_flags.extender() { r } else { 1 };
                for _ in 0..times {
                    seq.push(p);
                }
            }
            seq
        };
        let has_extender = parts.iter().any(|p| p.part_flags.extender());
        let mut r = if has_extender { 1 } else { 0 };
        let mut seq = build(r);
        while has_extender && extent_du(&seq) < target_du && r < 256 {
            r += 1;
            seq = build(r);
        }
        if seq.is_empty() {
            return None;
        }
        let overlap_scaled = size * (overlap_du / upem);
        let mut out = Vec::with_capacity(seq.len());
        let mut cursor = Length::ZERO;
        for p in &seq {
            let advance = size * (p.full_advance as f64 / upem);
            out.push((p.glyph_id.0, cursor, advance));
            cursor += advance - overlap_scaled;
        }
        Some(out)
    }

    // ---- Registry-abbrev resolution --------------------------------------------------------------------

    fn resolve_font_abbrev(&self, abbrev: &str) -> Option<FontKey> {
        self.abbrev_key(abbrev)
    }

    /// Reverse scan of `abbrevs`. Linear, but that map holds one row per
    /// configured font (tens at most) and `get-font` is called a handful of
    /// times per document, so a second index would cost more than it saves.
    fn font_abbrev(&self, key: FontKey) -> Option<String> {
        self.abbrevs
            .iter()
            .find(|(_, k)| **k == key)
            .map(|(abbrev, _)| abbrev.clone())
    }

    fn default_script_font(&self, script: Script) -> Option<(FontKey, f64, f64)> {
        self.script_default(script as usize)
    }

    fn default_math_font(&self) -> Option<FontKey> {
        self.math_font_default()
    }
}
