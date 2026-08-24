//! CSS `font-family` resolution for the reflow backend: the stack a run's
//! physical font file becomes, naming the face the document was typeset in
//! and falling back to generics.

/// The CSS `font-family` VALUE (a whole stack, not one name) for a physical
/// font file whose declared family is `family`, naming the face the document
/// was typeset in and falling back to generics — with nothing embedded.
///
/// **Why nothing is embedded.** Inlining each referenced font file as base64
/// would be right for a layout-faithful backend, which positions every glyph
/// run at a coordinate this port's own metrics produced, so the browser must
/// have exactly that face or the text drifts out of its boxes. A reflowed
/// document makes the opposite bargain — the browser re-breaks every line —
/// so the exact face buys nothing, while the bytes cost everything: with the
/// bundled Japanese faces, the `latexcmds` manual came to **20 MB**, of which
/// 20 MB was fonts. Naming the family instead gives the reader the real face
/// when they have it, a sensible one when they do not, and a 120 KB file
/// either way.
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
    if is_monospace_family(family) {
        format!("'{safe}', 'DejaVu Sans Mono', Menlo, Consolas, monospace")
    } else if SANS_MARKERS.iter().any(|m| lower.contains(m)) {
        format!("'{safe}', 'Noto Sans CJK JP', 'Hiragino Sans', sans-serif")
    } else {
        format!("'{safe}', 'Noto Serif CJK JP', 'Hiragino Mincho ProN', Georgia, serif")
    }
}

/// Whether a family name reads as a fixed-pitch face.
///
/// Checked BEFORE the serif/sans split, because the markers overlap: `code.
/// satyh` sets `lmmono`, and "DejaVu Sans Mono" would otherwise be classified
/// by its "sans". Getting this wrong is not a fallback nicety — a code block
/// set in a proportional serif is unreadable as code, and this is also what
/// tells `block.rs` that a line boundary inside the run is a HARD break.
pub(super) fn is_monospace_family(family: &str) -> bool {
    const MONO_MARKERS: [&str; 6] = ["mono", "courier", "consol", "menlo", "typewriter", "teletype"];
    let lower = family.to_ascii_lowercase();
    MONO_MARKERS.iter().any(|m| lower.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_fixed_pitch_face_ends_in_monospace_and_beats_the_sans_marker() {
        // `code.satyh`'s own face, which used to land in the serif stack.
        let lm = reflow_font_stack("LMMono10");
        assert!(lm.starts_with("'LMMono10',"), "{lm}");
        assert!(lm.ends_with("monospace"), "{lm}");
        // "Sans" appears in the name; "Mono" has to win.
        assert!(reflow_font_stack("DejaVu Sans Mono").ends_with("monospace"));
        assert!(!is_monospace_family("Junicode"));
        assert!(!is_monospace_family("IPAexGothic"));
    }
}
