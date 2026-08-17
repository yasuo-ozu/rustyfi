//! A [`FontMetrics`] provider backed by real TrueType/OpenType font files
//! (phase 5, first slice). Loads up to three faces — regular, bold, oblique —
//! mapped onto the existing `FontKey(0/1/2)` convention from `base14`, and
//! measures through `ttf-parser`'s `cmap`/`hmtx`/`hhea`/`OS/2` tables instead
//! of hardcoded AFM widths.

use std::fs;
use std::path::{Path, PathBuf};

use satysfi_backend::{FontKey, FontMetrics, Length};
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
}
