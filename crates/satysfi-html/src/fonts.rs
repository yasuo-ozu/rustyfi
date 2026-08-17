//! HTML output backend, Slice 3 (`docs/plans/design-html-output.md` §Slice
//! 3: "real fonts + math", THE font-metric-fidelity fix). Emits one
//! `@font-face` rule per physical font FILE the document actually
//! referenced, embedding that file's raw bytes as a base64
//! `data:font/ttf;base64,...` URI — so the browser lays text out in the
//! SAME face whose metrics the layout engine measured with, closing the
//! design doc's §Risks "font-metric fidelity" gap. Mirrors the CID PDF
//! writer's own per-FILE dedup (`cid.rs`'s `FontUsage`/`usage: BTreeMap
//! <usize, FontUsage>`, keyed by [`satysfi_pdf::TtfFontStore::file_index`]
//! rather than by `FontKey`, since bold/oblique with no configured face
//! fall back to sharing the regular file — one `@font-face` per FILE, not
//! per `FontKey` slot, avoids embedding the same bytes twice).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use satysfi_pdf::TtfFontStore;

use super::base64;

/// The CSS `font-family` name for physical font file `file_idx` — used both
/// by the `@font-face` rule's `font-family` and by every run's inline
/// `font-family` style (`html.rs`'s `Ctx::font_family_for`), which must
/// agree exactly for the browser to pick up the embedded face.
pub(super) fn font_family_name(file_idx: usize) -> String {
    format!("satysfi-html-font-{file_idx}")
}

/// One `@font-face` rule per file index in `used`, each embedding that
/// file's raw bytes verbatim (no subsetting — unlike the PDF CID writer's
/// D5 optimization, this slice keeps Slice-3 simple; subsetting the
/// `@font-face` payload is a documented, non-blocking follow-up, the same
/// size-optimization-only status D5 has for the PDF path). Iterates `used`
/// (a `BTreeSet`) rather than the store's own file list, so a document
/// that references only some of the store's configured faces (e.g. only
/// the regular slot, never bold/oblique/a math face) emits `@font-face`
/// only for what actually appears on a page — mirroring `write_font`
/// (`cid.rs`) only being called for `usage.keys()`, never every file the
/// store happens to hold.
pub(super) fn font_face_rules(store: &TtfFontStore, used: &BTreeSet<usize>) -> String {
    let mut out = String::new();
    for &file_idx in used {
        let b64 = base64::encode(store.file_bytes(file_idx));
        let family = font_family_name(file_idx);
        let _ = write!(
            out,
            "@font-face {{ font-family: \"{family}\"; \
             src: url(data:font/ttf;base64,{b64}) format(\"truetype\"); }}\n",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_names_are_distinct_per_file_index() {
        assert_ne!(font_family_name(0), font_family_name(1));
    }
}
