//! Which SATySFi generation a buffer is analysed under.
//!
//! Getting this wrong is the loudest possible failure: 0.1's grammar is a
//! different language, so a 0.1 file read with the 0.0.6 parser yields a
//! parse error on essentially its first binding, and the editor shows a red
//! squiggle on a file that compiles perfectly.

use rustyfi_syntax::RustyfiVersion;

/// Pick the generation to analyse `source` under, mirroring the CLI's Axis-A
/// ladder in `rustyfi`'s `resolve_version_and_mode`:
///
/// 1. an explicit override (`rustyfi lsp --lang 0.1`, the server's
///    `initializationOptions.lang`) wins outright;
/// 2. otherwise [`rustyfi_syntax::sniff_version`]'s verdict — a `use`-shaped
///    header or a `val` head pins 0.1, a `@stage:` header or a hyphenated
///    `let-*` head pins 0.0;
/// 3. otherwise [`RustyfiVersion::DEFAULT`] (0.0).
///
/// The one thing the CLI does that this does not is *warn* when the override
/// contradicts the sniff. A language server has no channel for that which is
/// worth the noise — `window/showMessage` on every keystroke of a file the
/// user has already told us the version of would be worse than silence.
///
/// This is on purpose the CLI's rule and not the loader's per-file rule.
/// `rustyfi_loader`'s detector answers a different question — "which
/// generation is this *dependency* of an already-pinned entry?" — and its
/// answer depends on the parent that reached it (`import_parent_version`) and
/// on whether the path sits in a frozen corpus directory. An open buffer is
/// an entry, not a dependency: nothing has reached it, so only the entry rule
/// applies.
pub fn detect_version(source: &str, override_lang: Option<RustyfiVersion>) -> RustyfiVersion {
    if let Some(v) = override_lang {
        return v;
    }
    rustyfi_syntax::sniff_version(source).unwrap_or(RustyfiVersion::DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_wins_even_against_a_contradicting_sniff() {
        // `let-rec` sniffs 0.0; the override still decides.
        assert_eq!(
            detect_version("let-rec f x = x", Some(RustyfiVersion::V0_1)),
            RustyfiVersion::V0_1
        );
        assert_eq!(
            detect_version("use Foo", Some(RustyfiVersion::V0_0)),
            RustyfiVersion::V0_0
        );
    }

    #[test]
    fn use_and_val_heads_select_0_1() {
        assert_eq!(
            detect_version("use package foo\nval x = 1", None),
            RustyfiVersion::V0_1
        );
        assert_eq!(
            detect_version("@require: pervasives\nval x = 1", None),
            RustyfiVersion::V0_1
        );
    }

    #[test]
    fn stage_headers_and_hyphenated_lets_select_0_0() {
        assert_eq!(
            detect_version("@stage: 0\nlet x = 1 in x", None),
            RustyfiVersion::V0_0
        );
        assert_eq!(
            detect_version("let-inline ctx \\e x = x", None),
            RustyfiVersion::V0_0
        );
    }

    #[test]
    fn an_ambiguous_buffer_falls_back_to_the_default() {
        for src in ["", "@require: stdjabook\nlet x = 1 in x", "% nothing here"] {
            assert_eq!(detect_version(src, None), RustyfiVersion::DEFAULT, "{src:?}");
        }
    }
}
