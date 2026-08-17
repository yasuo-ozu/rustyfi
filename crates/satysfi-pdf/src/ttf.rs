//! A [`FontMetrics`] provider backed by real TrueType/OpenType font files
//! (phase 5, first slice). Loads up to three faces — regular, bold, oblique —
//! mapped onto the existing `FontKey(0/1/2)` convention from `base14`, and
//! measures through `ttf-parser`'s `cmap`/`hmtx`/`hhea`/`OS/2` tables instead
//! of hardcoded AFM widths.

use std::fs;
use std::path::{Path, PathBuf};

use satysfi_backend::{
    FontKey, FontMetrics, Length, MathConstants, MathCorner, MathVariantGlyph, VertVariantPolicy,
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
    /// One entry per physical font file that was actually loaded.
    files: Vec<Vec<u8>>,
    /// `FontKey(0)=regular, 1=bold, 2=oblique` -> index into `files`. Missing
    /// bold/oblique share the regular slot (index 0).
    slot: [usize; 3],
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
        let mut slot = [0usize; 3];

        if let Some(path) = bold {
            files.push(Self::read_and_validate(path)?);
            slot[1] = files.len() - 1;
        }
        if let Some(path) = oblique {
            files.push(Self::read_and_validate(path)?);
            slot[2] = files.len() - 1;
        }

        Ok(TtfFontStore { files, slot })
    }

    fn read_and_validate(path: &Path) -> Result<Vec<u8>, FontError> {
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

    /// Clamp an arbitrary `FontKey` onto the three known slots, mirroring
    /// `base14::Base14Metrics`'s treatment of out-of-range keys.
    fn key_slot(font: FontKey) -> usize {
        (font.0 as usize).min(2)
    }

    /// The physical-file index backing `font` (after bold/oblique fallback).
    /// Used by the CID embedder to dedup: two `FontKey`s that resolve to the
    /// same file are embedded (and their Type0 font object shared) once.
    pub fn file_index(&self, font: FontKey) -> usize {
        self.slot[Self::key_slot(font)]
    }

    /// Number of distinct font files backing this store (1..=3).
    pub fn num_files(&self) -> usize {
        self.files.len()
    }

    /// Raw bytes of a physical file, for `FontFile2` embedding.
    pub fn file_bytes(&self, file_index: usize) -> &[u8] {
        &self.files[file_index]
    }

    /// Parse the face for a given font key. See the struct doc for why this
    /// reparses on every call instead of caching a `Face`.
    pub fn face(&self, font: FontKey) -> Option<Face<'_>> {
        self.face_by_file(self.file_index(font))
    }

    /// Parse the face for a given physical file index directly.
    pub fn face_by_file(&self, file_index: usize) -> Option<Face<'_>> {
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

    // ---- OpenType MATH table (docs/plans/math-engine.md §B1/§B3) ---------
    //
    // Every accessor below was checked against the installed ttf-parser
    // 0.25.1 `tables::math` module (`~/.cargo/registry/.../ttf-parser-0.25.1/
    // src/tables/math.rs`): `Face::tables().math -> Option<math::Table>`
    // with `.constants: Option<Constants>` / `.glyph_info: Option<GlyphInfo>`
    // / `.variants: Option<Variants>`; every `Constants` accessor except the
    // two percent-scale-downs returns `MathValue { value: i16, device }` (a
    // struct, not a plain integer) — hence the `mv.value` field access in
    // `r(...)` below. `GlyphInfo.italic_corrections`/`.kern_infos` are
    // `Option<MathValues>`/`Option<KernInfos>` fields (not methods), each
    // with a `.get(GlyphId) -> Option<...>` accessor. `KernInfo`'s four
    // corners are `Option<Kern>` fields. `Kern::count()/height(i)/kern(i)`
    // all matched the spec's assumed shape exactly — no deviations found.
    //
    // §B3 (`math_vertical_variant`, below) consumes `Variants` itself:
    // `Variants { min_connector_overlap: u16, vertical_constructions:
    // GlyphConstructions, horizontal_constructions: GlyphConstructions }`;
    // `GlyphConstructions::get(GlyphId) -> Option<GlyphConstruction>`;
    // `GlyphConstruction { assembly: Option<GlyphAssembly>, variants:
    // LazyArray16<GlyphVariant> }`; `GlyphVariant { variant_glyph: GlyphId,
    // advance_measurement: u16 }`. `LazyArray16::len() -> u16` /
    // `::get(index: u16) -> Option<T>` (both confirmed against `parser.rs`)
    // — no deviation from the spec's assumed shape.

    fn math_constants(&self, font: FontKey) -> Option<MathConstants> {
        let face = self.face(font)?;
        let c = face.tables().math?.constants?;
        let upem = face.units_per_em() as f64;
        let r = |mv: ttf_parser::math::MathValue| mv.value as f64 / upem;
        Some(MathConstants {
            axis_height: r(c.axis_height()),
            superscript_bottom_min: r(c.superscript_bottom_min()),
            superscript_shift_up: r(c.superscript_shift_up()),
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

    /// §B3 (`MathVariants`): pick a vertically-grown variant of `c` per
    /// `policy` and report its real per-glyph ink metrics at `size`.
    /// Assembly-only constructions (`variants.len() == 0`, big enough
    /// stretchy delimiters in some fonts) are out of this slice's scope —
    /// `GlyphAssembly` is a documented follow-up (§B3b-2 area).
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
}
