//! HTML output backend, Slice 3 (: "real fonts + math", THE
//! font-metric-fidelity fix). Emits one `@font-face` rule per physical
//! font FILE the document actually referenced, embedding that file's raw
//! bytes as a base64 `data:font/ttf;base64,...` URI — so the browser lays
//! text out in the SAME face whose metrics the layout engine measured
//! with, closing the design doc's §Risks "font-metric fidelity" gap.
//! Mirrors the CID PDF writer's own per-FILE dedup (`cid.rs`'s
//! `FontUsage`/`usage: BTreeMap <usize, FontUsage>`, keyed by
//! [`rustyfi_pdf::TtfFontStore::file_index`] rather than by `FontKey`,
//! since bold/oblique with no configured face fall back to sharing the
//! regular file — one `@font-face` per FILE, not per `FontKey` slot,
//! avoids embedding the same bytes twice).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use rustyfi_pdf::TtfFontStore;

use super::base64;

/// The CSS `font-family` name for physical font file `file_idx` — used both
/// by the `@font-face` rule's `font-family` and by every run's inline
/// `font-family` style (`html.rs`'s `Ctx::font_family_for`), which must
/// agree exactly for the browser to pick up the embedded face.
pub(super) fn font_family_name(file_idx: usize) -> String {
    format!("rustyfi-html-font-{file_idx}")
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

/// The reflow backend's answer to the same question, and a deliberately
/// different one: the CSS `font-family` VALUE (a whole stack, not one name)
/// for physical file `file_idx`, naming the face the document was typeset in
/// and falling back to generics — with nothing embedded.
///
/// **Why the reflow backend does not embed.** [`font_face_rules`] above
/// inlines each referenced font file as base64, which is right for the
/// faithful backend: it positions every glyph run at a coordinate this
/// port's own metrics produced, so the browser must have exactly that face
/// or the text drifts out of its boxes. A reflowed document makes the
/// opposite bargain — the browser re-breaks every line — so the exact face
/// buys nothing, while the bytes cost everything: with the bundled Japanese
/// faces, the `latexcmds` manual came to **20 MB**, of which 20 MB was
/// fonts. Naming the family instead gives the reader the real face when they
/// have it, a sensible one when they do not, and a 120 KB file either way.
///
/// The serif/sans split is read off the family NAME. That is a heuristic and
/// is labelled as one; it is also the only signal available without a
/// PANOSE/OS-2 classification pass, it decides nothing but which generic the
/// browser falls back to when the named face is absent, and the CJK families
/// this port bundles name themselves unambiguously (`IPAexGothic` /
/// `IPAexMincho`).
/// Family names are SINGLE-quoted, and any single quote or backslash inside
/// one is stripped. The stack goes into an inline `style="…"` attribute as
/// well as into the stylesheet, and a double quote there would terminate the
/// attribute — which is exactly what happened: every table cell set in
/// Junicode emitted `style="font-family:"Junicode", …"` and the browser read
/// the rest of the declaration as bare attributes.
pub(super) fn reflow_font_stack(family: &str) -> String {
    const SANS_MARKERS: [&str; 6] = [
        "gothic",
        "sans",
        "grotesk",
        "grotesque",
        "helvetica",
        "arial",
    ];
    let lower = family.to_ascii_lowercase();
    let safe: String = family
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\' | '<' | '>'))
        .collect();
    if SANS_MARKERS.iter().any(|m| lower.contains(m)) {
        format!("'{safe}', 'Noto Sans CJK JP', 'Hiragino Sans', sans-serif")
    } else {
        format!("'{safe}', 'Noto Serif CJK JP', 'Hiragino Mincho ProN', Georgia, serif")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_names_are_distinct_per_file_index() {
        assert_ne!(font_family_name(0), font_family_name(1));
    }

    #[test]
    fn the_reflow_stack_names_the_face_first_and_always_ends_in_a_generic() {
        let mincho = reflow_font_stack("IPAexMincho");
        assert!(mincho.starts_with("'IPAexMincho',"), "{mincho}");
        assert!(mincho.ends_with("serif"), "{mincho}");
        // Never a double quote: the stack also goes into a `style="…"`.
        assert!(!mincho.contains('"'), "{mincho}");
        assert!(!reflow_font_stack("Od\"d'Name").contains('"'));
        let gothic = reflow_font_stack("IPAexGothic");
        assert!(gothic.ends_with("sans-serif"), "{gothic}");
        // A name with no marker either way falls back to serif — the safer
        // default for running prose.
        assert!(reflow_font_stack("Junicode").ends_with("serif"));
        assert!(!reflow_font_stack("Junicode").ends_with("sans-serif"));
    }
}
