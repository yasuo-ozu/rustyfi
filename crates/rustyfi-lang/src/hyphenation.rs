//! Knuth–Liang hyphenation engine.
//!
//! Confines the `hyphenation` crate dependency to this module: `Context`
//! (in `rustyfi-backend`) only stores a lightweight `HyphenLang` tag —
//! the actual `hyphenation::Standard` dictionaries (each ~89 KiB of decoded
//! bincode, not cheaply clonable) live in a process-global, load-once cache
//! here, keyed by that tag.

use hyphenation::{Hyphenator, Language, Load, Standard};
use rustyfi_backend::HyphenLang;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, Mutex, OnceLock};

/// Map the port's crate-independent tag (`rustyfi_backend::HyphenLang`) to
/// the `hyphenation` crate's own language enum — the one place this crate's
/// dependency on `hyphenation` and the backend's independence meet.
fn language_of(tag: HyphenLang) -> Language {
    match tag {
        HyphenLang::EnglishUS => Language::EnglishUS,
        HyphenLang::EnglishGB => Language::EnglishGB,
    }
}

/// The en-GB `Standard` dictionary, vendored directly from the `hyphenation`
/// crate's own `dictionaries/en-gb.standard.bincode` (v0.8.4; same
/// Apache-2.0/MIT `hyph-utf8`-derived "standard" tier as the embedded en-US
/// data — NOT one of the two non-permissively-licensed *extended* pattern
/// sets, `hyph-hu.ext`/`hyph-ca.ext`, the crate's README calls out separately).
///
/// Vendored instead of using a Cargo feature: `hyphenation` 0.8.4 only
/// exposes `embed_en-us` (en-US only) and `embed_all` (all 82 languages,
/// ~3 MiB, including the non-permissive `.ext` files) — no per-language
/// `embed_en-gb`. The crate's README names "shipping a `.bincode` and using
/// `from_path`" as the alternative; this is that, via `include_bytes!` +
/// `from_reader` instead of a runtime path, so it still needs no disk I/O.
/// `Cargo.toml`'s `hyphenation` feature list, and therefore the root
/// `Cargo.lock`, stays untouched.
const EN_GB_STANDARD_BINCODE: &[u8] = include_bytes!("hyph-data/en-gb.standard.bincode");

fn cache() -> &'static Mutex<HashMap<HyphenLang, Arc<Standard>>> {
    static DICTS: OnceLock<Mutex<HashMap<HyphenLang, Arc<Standard>>>> = OnceLock::new();
    DICTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Load-once, process-global: the underlying `hyphenation` load call runs at
/// most once per language per process, after which every caller shares the
/// same `Arc`. `None` only if the dictionary data is somehow unreadable
/// (never expected in practice — both dictionaries are baked into the
/// binary at compile time, no disk I/O).
fn dict(tag: HyphenLang) -> Option<Arc<Standard>> {
    let mut map = cache().lock().unwrap();
    if let Some(d) = map.get(&tag) {
        return Some(Arc::clone(d));
    }
    let standard = match tag {
        HyphenLang::EnglishUS => Standard::from_embedded(language_of(tag)).ok()?,
        HyphenLang::EnglishGB => {
            Standard::from_reader(language_of(tag), &mut Cursor::new(EN_GB_STANDARD_BINCODE))
                .ok()?
        }
    };
    let arc = Arc::new(standard);
    map.insert(tag, Arc::clone(&arc));
    Some(arc)
}

/// Char-index break offsets into `word` — Knuth–Liang candidates from the
/// installed dictionary, converted from the crate's byte offsets and
/// filtered so that each accepted break leaves at least `left_min` chars
/// before it and `right_min` chars after it (matches upstream's
/// `uchar_segment`-counted minimums).
///
/// Returns an empty `Vec` when the dictionary is unavailable, the word is
/// empty, the dictionary found no breaks, or every candidate break was
/// filtered out by the min-fragment constraints — in every such case the
/// caller (`primitives::text_to_boxes`'s `flush_word`) emits the word
/// exactly as it would with no dictionary installed, which is what keeps
/// the byte-identity gate provable.
pub fn hyphenate_word(
    tag: HyphenLang,
    word: &str,
    left_min: usize,
    right_min: usize,
) -> Vec<usize> {
    let Some(dict) = dict(tag) else {
        return Vec::new();
    };
    if word.is_empty() {
        return Vec::new();
    }
    let hyphenated = dict.hyphenate(word);
    if hyphenated.breaks.is_empty() {
        return Vec::new();
    }

    // The crate's `breaks` are BYTE offsets, guaranteed to land on char
    // boundaries. Build the table of char-boundary byte offsets (index =
    // char count before that boundary) and look each break up in it to
    // convert byte offset -> char offset.
    let mut char_offsets: Vec<usize> = word.char_indices().map(|(b, _)| b).collect();
    char_offsets.push(word.len());
    let n_chars = char_offsets.len() - 1;

    hyphenated
        .breaks
        .iter()
        .filter_map(|&b| char_offsets.iter().position(|&x| x == b))
        .filter(|&char_idx| char_idx >= left_min && n_chars - char_idx >= right_min)
        .collect()
}

/// Explicit soft hyphens (U+00AD) embedded in `word`, split out into (a) the
/// word with every one of them removed and (b) the char-index break points
/// into that *cleaned* word where each one sat.
///
/// The `hyphenation` crate's own `Standard::hyphenate` already special-cases
/// this (soft hyphens take priority over dictionary hyphenation and are
/// never filtered by the min-fragment machinery — an explicit soft hyphen is
/// authored intent, not a pattern-derived candidate), but its returned
/// `Word` keeps the soft hyphen character *still embedded* and reports each
/// break as that character's own byte offset — callers must know to drop it
/// or it leaks into a rendered fragment as a literal glyph-less character.
/// Implementing the equivalent directly here, instead of routing through
/// `dict.hyphenate` and post-processing its byte offsets, sidesteps that
/// footgun and keeps `hyphenate_word` above untouched.
///
/// Returns `(word.to_string(), Vec::new())` unchanged when `word` has no
/// soft hyphen, so a caller can always use the returned string in place of
/// `word` from here on.
pub(crate) fn strip_soft_hyphens(word: &str) -> (String, Vec<usize>) {
    if !word.contains('\u{ad}') {
        return (word.to_string(), Vec::new());
    }
    let mut clean = String::with_capacity(word.len());
    let mut breaks = Vec::new();
    let mut char_idx = 0usize;
    for c in word.chars() {
        if c == '\u{ad}' {
            breaks.push(char_idx);
        } else {
            clean.push(c);
            char_idx += 1;
        }
    }
    (clean, breaks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyphenation_breaks_into_expected_fragments() {
        // "hyphenation" -> "hy-phen-ation" per the crate's own README
        // example set (byte breaks map 1:1 to char offsets here — ASCII).
        let breaks = hyphenate_word(HyphenLang::EnglishUS, "hyphenation", 3, 2);
        assert!(!breaks.is_empty(), "expected at least one break");
        let chars: Vec<char> = "hyphenation".chars().collect();
        let mut prev = 0;
        let mut fragments = Vec::new();
        for &b in &breaks {
            fragments.push(chars[prev..b].iter().collect::<String>());
            prev = b;
        }
        fragments.push(chars[prev..].iter().collect::<String>());
        assert_eq!(fragments.join(""), "hyphenation");
        for f in &fragments {
            assert!(!f.is_empty());
        }
    }

    #[test]
    fn anfractuous_matches_the_crate_doc_example() {
        // Crate docs: `en_us.hyphenate("anfractuous").breaks == [2, 6, 8]`
        // (byte offsets) -> char offsets identical here (pure ASCII) ->
        // fragments `["an", "frac", "tu", "ous"]`.
        let breaks = hyphenate_word(HyphenLang::EnglishUS, "anfractuous", 0, 0);
        assert_eq!(breaks, vec![2, 6, 8]);
    }

    #[test]
    fn short_word_yields_no_breaks_under_min_fragment_filter() {
        // A 4-letter word cannot satisfy left_min=3 && right_min=2
        // simultaneously (would need >= 5 chars), so every candidate is
        // filtered out.
        let breaks = hyphenate_word(HyphenLang::EnglishUS, "word", 3, 2);
        assert!(breaks.is_empty());
    }

    #[test]
    fn dict_is_loaded_once_and_shared() {
        let a = dict(HyphenLang::EnglishUS).expect("embedded en-US dictionary");
        let b = dict(HyphenLang::EnglishUS).expect("embedded en-US dictionary");
        assert!(Arc::ptr_eq(&a, &b), "expected the same cached Arc instance");
    }

    #[test]
    fn en_gb_dict_is_loaded_once_and_shared() {
        let a = dict(HyphenLang::EnglishGB).expect("vendored en-GB dictionary");
        let b = dict(HyphenLang::EnglishGB).expect("vendored en-GB dictionary");
        assert!(Arc::ptr_eq(&a, &b), "expected the same cached Arc instance");
    }

    /// The whole point of shipping a *separate* en-GB dictionary: prove it
    /// is actually consulted, not silently aliased to en-US. "photography"
    /// is a real Knuth–Liang divergence between the two pattern sets (found
    /// by an exhaustive comparison over a candidate word list): en-US breaks
    /// after "pho", "tog", "ra" ("pho-tog-ra-phy"); en-GB breaks after "pho"
    /// only, plus one earlier point ("pho-togra-phy"). Both break sets fully
    /// survive the default `left_min=3`/`right_min=2` filter here (11
    /// chars), so this also doubles as a real fragment-boundary check, not
    /// just a "some breaks exist" smoke test.
    #[test]
    fn en_gb_hyphenates_a_word_differently_from_en_us_proving_the_gb_dictionary_is_used() {
        let us_breaks = hyphenate_word(HyphenLang::EnglishUS, "photography", 3, 2);
        let gb_breaks = hyphenate_word(HyphenLang::EnglishGB, "photography", 3, 2);
        assert_eq!(us_breaks, vec![3, 6, 8], "en-US: pho-tog-ra-phy");
        assert_eq!(gb_breaks, vec![3, 5], "en-GB: pho-togra-phy");
        assert_ne!(
            us_breaks, gb_breaks,
            "en-US and en-GB must disagree on this word, or this test isn't proving anything"
        );
    }

    #[test]
    fn strip_soft_hyphens_extracts_a_single_explicit_break() {
        // "hy\u{00AD}phenation" is authored as "hy-phenation": the soft
        // hyphen sits between the 2nd and 3rd chars of the cleaned word.
        let (clean, breaks) = strip_soft_hyphens("hy\u{ad}phenation");
        assert_eq!(clean, "hyphenation");
        assert_eq!(breaks, vec![2]);
    }

    #[test]
    fn strip_soft_hyphens_extracts_multiple_explicit_breaks_in_order() {
        let (clean, breaks) = strip_soft_hyphens("a\u{ad}b\u{ad}c\u{ad}d");
        assert_eq!(clean, "abcd");
        assert_eq!(breaks, vec![1, 2, 3]);
    }

    #[test]
    fn strip_soft_hyphens_is_a_no_op_when_no_soft_hyphen_is_present() {
        let (clean, breaks) = strip_soft_hyphens("hyphenation");
        assert_eq!(clean, "hyphenation");
        assert!(breaks.is_empty());
    }
}
