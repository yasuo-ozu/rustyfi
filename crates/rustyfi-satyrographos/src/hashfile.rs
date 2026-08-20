//! `*.satysfi-hash` files, and the one thing that makes them different from
//! every other file this crate installs: they are **shared**.
//!
//! `dist/hash/fonts.satysfi-hash` is a single JSON object mapping a font
//! abbrev to the file that provides it, and *every* font package contributes
//! entries to it — the standard library names `ipaexm` and `lmroman`, a font
//! package adds `TheanoDidot`. Copying the file, as every other kind is
//! copied, would mean the last package installed erases the others' fonts (or,
//! with this crate's unmanaged-file guard, that no font package can be
//! installed into a root that has a standard library at all). Upstream
//! Satyrographos merges them, so this does too.
//!
//! Merging is textual on the value side, deliberately:
//!
//! ```text
//! { "TheanoDidot": <Single: { "src-dist": "fonts-theano/TheanoDidot.otf" }> }
//! ```
//!
//! That value is Yojson, not JSON — upstream's variant syntax, which
//! [`rustyfi_pdf`]'s reader accepts. Parsing it into a JSON model and printing
//! it back would silently rewrite a package's own declaration into a shape its
//! author never wrote, so each entry's text is preserved exactly as it came and
//! only the *keys* are interpreted. What is merged is the set of keys; what is
//! written back is the text.
//!
//! Ownership is per key. A receipt records which keys its package put in
//! ([`crate::receipts::FileEntry::keys`]), which is what lets `uninstall`
//! remove one package's fonts from a file three packages share.

use std::collections::BTreeSet;

/// One `*.satysfi-hash` file: keys in insertion order, each with its value's
/// original text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HashFile {
    entries: Vec<(String, String)>,
}

/// Why a hash file could not be read as one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl HashFile {
    /// Read a hash file. The outer value must be an object; each member's value
    /// is kept as text.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let chars: Vec<char> = text.chars().collect();
        let mut i = skip_ws(&chars, 0);
        if chars.get(i) != Some(&'{') {
            return Err(ParseError {
                message: "expected a JSON object `{ … }`".to_string(),
            });
        }
        i = skip_ws(&chars, i + 1);

        let mut entries: Vec<(String, String)> = Vec::new();
        while i < chars.len() && chars[i] != '}' {
            let (key, next) = string_at(&chars, i).ok_or_else(|| ParseError {
                message: format!("expected a quoted key at byte {i}"),
            })?;
            i = skip_ws(&chars, next);
            if chars.get(i) != Some(&':') {
                return Err(ParseError {
                    message: format!("expected `:` after key `{key}`"),
                });
            }
            let value_start = skip_ws(&chars, i + 1);
            let value_end = value_end(&chars, value_start).ok_or_else(|| ParseError {
                message: format!("unterminated value for key `{key}`"),
            })?;
            let value: String = chars[value_start..value_end].iter().collect();
            entries.push((key, value.trim_end().to_string()));

            i = skip_ws(&chars, value_end);
            if chars.get(i) == Some(&',') {
                i = skip_ws(&chars, i + 1);
            }
        }
        Ok(HashFile { entries })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The keys, in the order they appear.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    /// Drop `keys` — how a package's own entries leave a shared file.
    pub fn remove_keys(&mut self, keys: &[String]) {
        let drop: BTreeSet<&str> = keys.iter().map(String::as_str).collect();
        self.entries.retain(|(k, _)| !drop.contains(k.as_str()));
    }

    /// Add every entry of `other`, whose keys must not already be present —
    /// the caller removes the ones it owns first, so anything left is another
    /// package's (or the standard library's) and silently overwriting it would
    /// change which file a font name resolves to.
    pub fn merge_in(&mut self, other: &HashFile) -> Result<(), Vec<String>> {
        let mine: BTreeSet<&str> = self.keys().collect();
        let clashes: Vec<String> = other
            .keys()
            .filter(|k| mine.contains(k))
            .map(str::to_string)
            .collect();
        if !clashes.is_empty() {
            return Err(clashes);
        }
        self.entries.extend(other.entries.iter().cloned());
        Ok(())
    }

    /// Serialise: one entry per line, values as they came in.
    pub fn to_text(&self) -> String {
        if self.entries.is_empty() {
            return "{}\n".to_string();
        }
        let body = self
            .entries
            .iter()
            .map(|(k, v)| format!("  {}: {}", json_string(k), v))
            .collect::<Vec<_>>()
            .join(",\n");
        format!("{{\n{body}\n}}\n")
    }
}

fn skip_ws(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// The quoted string starting at `i`, unescaped only for `\"` and `\\` — hash
/// keys are font abbrevs and script names, which use neither, and preserving
/// anything else verbatim is what round-tripping needs.
fn string_at(chars: &[char], i: usize) -> Option<(String, usize)> {
    if chars.get(i) != Some(&'"') {
        return None;
    }
    let mut out = String::new();
    let mut j = i + 1;
    while j < chars.len() {
        match chars[j] {
            '"' => return Some((out, j + 1)),
            '\\' if j + 1 < chars.len() => {
                out.push(chars[j + 1]);
                j += 2;
            }
            c => {
                out.push(c);
                j += 1;
            }
        }
    }
    None
}

/// Index one past the value starting at `from`: brackets are balanced, quoted
/// strings are skipped, and Yojson's `<…>` variant wrapper counts as a bracket
/// so `<Single: {…}>` is one value.
fn value_end(chars: &[char], from: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '"' => {
                let (_, next) = string_at(chars, i)?;
                i = next;
                continue;
            }
            '{' | '[' | '<' => depth += 1,
            '}' | ']' | '>' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            // A bare value (a number, `null`) ends at the separator.
            ',' | '}' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    (depth == 0).then_some(chars.len())
}

fn json_string(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_ports_own_plain_json_file() {
        let f = HashFile::parse(
            r#"{
  "ipaexm":     { "src": "dist/fonts/ipaexm.ttf" },
  "Junicode-b": { "src": "dist/fonts/Junicode-Bold.ttf" }
}"#,
        )
        .unwrap();
        assert_eq!(f.keys().collect::<Vec<_>>(), ["ipaexm", "Junicode-b"]);
    }

    #[test]
    fn a_yojson_variant_value_survives_a_round_trip() {
        // Upstream's own spelling. Reprinting this as plain JSON would rewrite
        // a package author's declaration.
        let text = r#"{ "TheanoDidot": <Single: { "src-dist": "fonts-theano/T.otf" }> }"#;
        let f = HashFile::parse(text).unwrap();
        let out = f.to_text();
        assert!(
            out.contains(r#"<Single: { "src-dist": "fonts-theano/T.otf" }>"#),
            "value text must be preserved verbatim, got {out}"
        );
        assert_eq!(HashFile::parse(&out).unwrap(), f, "and re-read identically");
    }

    #[test]
    fn merging_unions_the_keys() {
        let mut a = HashFile::parse(r#"{ "ipaexm": { "src": "a.ttf" } }"#).unwrap();
        let b = HashFile::parse(r#"{ "Theano": <Single: { "src-dist": "b.otf" }> }"#).unwrap();
        a.merge_in(&b).unwrap();
        assert_eq!(a.keys().collect::<Vec<_>>(), ["ipaexm", "Theano"]);
    }

    #[test]
    fn a_key_two_packages_both_claim_is_a_conflict_not_a_silent_win() {
        let mut a = HashFile::parse(r#"{ "ipaexm": { "src": "a.ttf" } }"#).unwrap();
        let b = HashFile::parse(r#"{ "ipaexm": { "src": "b.ttf" } }"#).unwrap();
        assert_eq!(a.merge_in(&b).unwrap_err(), vec!["ipaexm".to_string()]);
    }

    #[test]
    fn removing_a_packages_keys_leaves_the_others() {
        let mut f = HashFile::parse(
            r#"{ "a": { "src": "a" }, "b": { "src": "b" }, "c": { "src": "c" } }"#,
        )
        .unwrap();
        f.remove_keys(&["b".to_string()]);
        assert_eq!(f.keys().collect::<Vec<_>>(), ["a", "c"]);
        assert!(!f.is_empty());
    }

    #[test]
    fn an_emptied_file_is_still_valid_json() {
        let mut f = HashFile::parse(r#"{ "a": { "src": "a" } }"#).unwrap();
        f.remove_keys(&["a".to_string()]);
        assert!(f.is_empty());
        assert_eq!(HashFile::parse(&f.to_text()).unwrap(), HashFile::default());
    }

    #[test]
    fn a_brace_inside_a_string_does_not_end_the_value() {
        let f = HashFile::parse(r#"{ "k": { "src": "we{ird}.ttf" } }"#).unwrap();
        assert_eq!(f.keys().collect::<Vec<_>>(), ["k"]);
        assert!(f.to_text().contains("we{ird}.ttf"));
    }

    #[test]
    fn a_non_object_is_refused_rather_than_read_as_empty() {
        assert!(HashFile::parse("[]").is_err());
        assert!(HashFile::parse("").is_err());
    }
}
