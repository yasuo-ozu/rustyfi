//! The `.opam` fields a SATySFi package needs before it can be installed:
//! `extra-source`, `build:`, and `depends:`.
//!
//! Font packages are the reason for the first two. `satysfi-fonts-theano`
//! ships no fonts — it ships an `extra-source` naming an upstream zip and a
//! checksum, and a `build:` line that unpacks it; only then do the paths its
//! `Satyristes` declares exist. Reading the whole opam language is not the
//! goal, and this deliberately does not: it scans for these fields and
//! ignores the rest, because everything else in the file is for OPAM, which
//! this port does not have.
//!
//! `depends:` is parsed too (name plus raw constraint text — see
//! [`Dependency`]) and RECORDED on [`Opam`], but deliberately not translated
//! into [`crate::version::Constraint`] or fed to the solver: see
//! [`Dependency`]'s own doc for why, and `registry::opam_index`'s module doc
//! for the sibling registry-index reader's story.
//!
//! ```text
//! extra-source "theano-2.0.otf.zip" {
//!   archive: "https://…/theano-2.0.otf.zip"
//!   checksum: [
//!     "sha256=e693…"
//!     "sha512=4463…"
//!   ]
//! }
//! depends: [
//!   "satysfi" {>= "0.0.3" & < "0.0.4"}
//! ]
//! build: [
//!   ["unzip" "-o" "theano-2.0.otf.zip" "*.otf" "-d" "theano"]
//! ]
//! ```

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::util;

/// One `extra-source "NAME" { … }` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraSource {
    /// The file it is fetched to, relative to the package directory.
    pub name: String,
    /// `archive:` (or `src:`) — where to fetch it from.
    pub url: String,
    /// The `sha256=` entry of `checksum:`, when the file declares one. Other
    /// algorithms are read past: sha256 is what this crate can verify, and a
    /// checksum it cannot check is worse than none, because it looks checked.
    pub sha256: Option<String>,
}

/// One `depends:` entry: an opam package name plus its raw constraint/filter
/// clause, exactly as written, if it has one.
///
/// The clause is kept as opam's own text (`>= "0.0.3" & < "0.0.4"`, `= "1.0.0"`,
/// a `{ os = "linux" }` build filter, …) rather than translated into
/// [`crate::version::Constraint`]. Opam's grammar has comparison ranges,
/// boolean `&`/`|` conjunction, and non-version filters; this crate's
/// constraint model is only an exact pin, a caret range, or "any"
/// ([`crate::version::Constraint`]) — there is no faithful mapping from one to
/// the other, and a *guessed* one would be worse than an admittedly-unparsed
/// one, because a wrong constraint that happens to parse looks trustworthy.
/// Likewise the name is opam's own package id (`"satysfi"`, `"ocaml"`,
/// `"satysfi-base"`, …), not resolved against this port's library-name
/// registry lookup — see `registry::opam_index`'s module doc for why that step
/// is also not taken here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub constraint: Option<String>,
}

/// What an `.opam` file says a package needs before installing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Opam {
    pub extra_sources: Vec<ExtraSource>,
    /// `build:` command lines, in order.
    pub build: Vec<Vec<String>>,
    /// `depends:` entries, in order (see [`Dependency`]). Parsed and
    /// recorded for inspection; not consulted by [`Opam::is_empty`] (nothing
    /// about *preparing* a package — fetching `extra-source`s, running
    /// `build:` — depends on them) and not fed to the solver.
    pub depends: Vec<Dependency>,
}

impl Opam {
    pub fn is_empty(&self) -> bool {
        self.extra_sources.is_empty() && self.build.is_empty()
    }
}

/// The `.opam` files in `dir`, sorted, so a package with several is read in a
/// stable order.
pub fn opam_files(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "opam"))
        .collect();
    found.sort();
    found
}

/// Read `path`, keeping only `extra-source`, `build:`, and `depends:`.
pub fn read(path: &Path) -> Result<Opam, Error> {
    Ok(parse(&util::read_to_string(path)?))
}

/// Everything the package directory declares, across its `.opam` files.
pub fn read_dir(dir: &Path) -> Result<Opam, Error> {
    let mut all = Opam::default();
    for file in opam_files(dir) {
        let one = read(&file)?;
        all.extra_sources.extend(one.extra_sources);
        all.build.extend(one.build);
        all.depends.extend(one.depends);
    }
    Ok(all)
}

/// Scan the three fields out of an opam file's text.
pub fn parse(text: &str) -> Opam {
    let mut opam = Opam::default();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if starts_at(&bytes, i, "extra-source") {
            if let Some((src, next)) = parse_extra_source(&bytes, i + "extra-source".len()) {
                opam.extra_sources.push(src);
                i = next;
                continue;
            }
        }
        // `build:` at the start of a line — not `x-build:` or a word ending
        // in "build" inside prose.
        if starts_at(&bytes, i, "build:") && at_field_start(&bytes, i) {
            if let Some((lines, next)) = parse_command_list(&bytes, i + "build:".len()) {
                opam.build.extend(lines);
                i = next;
                continue;
            }
        }
        // `depends:` — same start-of-line rule.
        if starts_at(&bytes, i, "depends:") && at_field_start(&bytes, i) {
            if let Some((deps, next)) = parse_depends(&bytes, i + "depends:".len()) {
                opam.depends.extend(deps);
                i = next;
                continue;
            }
        }
        i += 1;
    }
    opam
}

/// `[ "name" {constraint} "name2" ... ]` — one dependency name per quoted
/// string, each optionally followed by a brace-delimited constraint/filter
/// clause kept as raw text (see [`Dependency`]).
fn parse_depends(chars: &[char], from: usize) -> Option<(Vec<Dependency>, usize)> {
    let open = chars[from..]
        .iter()
        .position(|c| !c.is_whitespace())
        .map(|off| from + off)?;
    if chars[open] != '[' {
        return None;
    }
    let close = matching(chars, open, '[', ']')?;
    let inner: Vec<char> = chars[open + 1..close].to_vec();

    let mut deps = Vec::new();
    let mut i = 0;
    while i < inner.len() {
        if inner[i] == '"' {
            let (name, after_name) = next_string(&inner, i)?;
            let mut j = after_name;
            while j < inner.len() && inner[j].is_whitespace() {
                j += 1;
            }
            if j < inner.len() && inner[j] == '{' {
                let end = matching(&inner, j, '{', '}')?;
                let constraint: String = inner[j + 1..end].iter().collect::<String>().trim().to_string();
                deps.push(Dependency {
                    name,
                    constraint: Some(constraint),
                });
                i = end + 1;
            } else {
                deps.push(Dependency {
                    name,
                    constraint: None,
                });
                i = after_name;
            }
        } else {
            i += 1;
        }
    }
    Some((deps, close + 1))
}

fn starts_at(chars: &[char], i: usize, word: &str) -> bool {
    chars[i..].starts_with(&word.chars().collect::<Vec<_>>()[..])
}

/// A field name only counts at the start of a line (opam is line-oriented at
/// the top level), so `x-build:` or a `build:` inside a description does not
/// trigger.
fn at_field_start(chars: &[char], i: usize) -> bool {
    chars[..i]
        .iter()
        .rev()
        .take_while(|c| **c != '\n')
        .all(|c| c.is_whitespace())
}

/// `"name" { archive: "url" checksum: [ "sha256=…" ] }`
fn parse_extra_source(chars: &[char], from: usize) -> Option<(ExtraSource, usize)> {
    let (name, after_name) = next_string(chars, from)?;
    let open = chars[after_name..].iter().position(|c| *c == '{')? + after_name;
    let close = matching(chars, open, '{', '}')?;
    let body: String = chars[open + 1..close].iter().collect();

    let url = field_string(&body, "archive:")
        .or_else(|| field_string(&body, "src:"))
        .or_else(|| field_string(&body, "url:"))?;
    let sha256 = body
        .split('"')
        .find(|s| s.starts_with("sha256="))
        .map(|s| s.trim_start_matches("sha256=").to_string());
    Some((ExtraSource { name, url, sha256 }, close + 1))
}

/// `[ ["cmd" "arg"] ["cmd" "arg"] ]` — the command lines, with any
/// `{ filter }` conditions ignored.
fn parse_command_list(chars: &[char], from: usize) -> Option<(Vec<Vec<String>>, usize)> {
    let open = chars[from..]
        .iter()
        .position(|c| !c.is_whitespace())
        .map(|off| from + off)?;
    if chars[open] != '[' {
        return None;
    }
    let close = matching(chars, open, '[', ']')?;
    let inner: Vec<char> = chars[open + 1..close].to_vec();

    // Either a list of lists, or a single bare command line.
    let mut lines = Vec::new();
    let mut i = 0;
    while i < inner.len() {
        match inner[i] {
            '[' => {
                let end = matching(&inner, i, '[', ']')?;
                lines.push(words(&inner[i + 1..end].iter().collect::<String>()));
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    if lines.is_empty() {
        let bare = words(&inner.iter().collect::<String>());
        if !bare.is_empty() {
            lines.push(bare);
        }
    }
    lines.retain(|l| !l.is_empty());
    Some((lines, close + 1))
}

/// The quoted strings in `text`, in order — an opam command line is a list of
/// them, and anything unquoted (a `{ os = "linux" }` filter, a variable) is
/// not something this port can evaluate, so it is left out.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else { break };
        out.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    out
}

/// The string value of `field` in `body`, e.g. `archive: "https://…"`.
fn field_string(body: &str, field: &str) -> Option<String> {
    let at = body.find(field)? + field.len();
    let rest = &body[at..];
    let start = rest.find('"')? + 1;
    let end = rest[start..].find('"')? + start;
    Some(rest[start..end].to_string())
}

/// The first quoted string at or after `from`, and the index past it.
fn next_string(chars: &[char], from: usize) -> Option<(String, usize)> {
    let start = chars[from..].iter().position(|c| *c == '"')? + from;
    let end = chars[start + 1..].iter().position(|c| *c == '"')? + start + 1;
    Some((chars[start + 1..end].iter().collect(), end + 1))
}

/// Index of the bracket closing the one at `open`, honouring nesting and
/// skipping brackets inside quoted strings.
fn matching(chars: &[char], open: usize, l: char, r: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    for (i, c) in chars.iter().enumerate().skip(open) {
        match c {
            '"' => in_string = !in_string,
            c if *c == l && !in_string => depth += 1,
            c if *c == r && !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real `satysfi-fonts-theano.opam`, trimmed to the two fields that
    /// matter here.
    const THEANO: &str = r#"
opam-version: "2.0"
name: "satysfi-fonts-theano"
description: """
This package installs fonts, and mentions build: in prose.
"""
extra-source "theano-2.0.otf.zip" {
  archive: "https://github.com/akryukov/theano/releases/download/v2.0/theano-2.0.otf.zip"
  checksum: [
    "sha256=e69375109af4af1328b3fcee338546ab08db5ee52a0d33c6749babb8169e3ef6"
    "sha512=4463a5ca837b2d96ca8c3f3ea539070b66fd4340257ce9458400fa312d02837c"
  ]
}
depends: [
  "satysfi" {>= "0.0.3" & < "0.0.4"}
]
build: [
  ["unzip" "-o" "theano-2.0.otf.zip" "*.otf" "-d" "theano"]
]
install: [
  ["satyrographos" "opam" "install" "-name" "fonts-theano"]
]
"#;

    #[test]
    fn reads_the_extra_source_and_its_sha256() {
        let opam = parse(THEANO);
        assert_eq!(opam.extra_sources.len(), 1);
        let src = &opam.extra_sources[0];
        assert_eq!(src.name, "theano-2.0.otf.zip");
        assert!(src.url.ends_with("theano-2.0.otf.zip"));
        assert_eq!(
            src.sha256.as_deref(),
            Some("e69375109af4af1328b3fcee338546ab08db5ee52a0d33c6749babb8169e3ef6"),
            "the sha512 beside it must not be mistaken for the sha256"
        );
    }

    #[test]
    fn reads_the_build_commands_only() {
        let opam = parse(THEANO);
        assert_eq!(
            opam.build,
            vec![vec!["unzip", "-o", "theano-2.0.otf.zip", "*.otf", "-d", "theano"]]
        );
        // `install:` is OPAM invoking Satyrographos, which this port does not
        // need: it installs from the Satyristes itself.
        assert!(
            !opam.build.iter().any(|l| l[0] == "satyrographos"),
            "install: must not be read as build:"
        );
    }

    #[test]
    fn a_build_word_in_prose_is_not_a_field() {
        // The description above says "build:" mid-sentence.
        assert_eq!(parse(THEANO).build.len(), 1);
    }

    #[test]
    fn a_file_with_neither_field_is_empty() {
        assert!(parse("opam-version: \"2.0\"\nname: \"x\"\n").is_empty());
    }

    #[test]
    fn reads_the_depends_name_and_raw_constraint() {
        let opam = parse(THEANO);
        assert_eq!(
            opam.depends,
            vec![Dependency {
                name: "satysfi".to_string(),
                constraint: Some(">= \"0.0.3\" & < \"0.0.4\"".to_string()),
            }]
        );
    }

    #[test]
    fn depends_is_not_counted_by_is_empty() {
        // Nothing about preparing a package (fetching extra-sources, running
        // build:) depends on `depends:`, so a file with only that field is
        // still "nothing to prepare".
        let opam = parse("depends: [\n  \"satysfi\" {>= \"0.0.3\"}\n]\n");
        assert_eq!(opam.depends.len(), 1);
        assert!(opam.is_empty());
    }

    #[test]
    fn a_dependency_with_no_constraint_clause_is_recorded_with_none() {
        let opam = parse("depends: [\n  \"ocaml\"\n  \"dune\" {build}\n]\n");
        assert_eq!(
            opam.depends,
            vec![
                Dependency { name: "ocaml".to_string(), constraint: None },
                Dependency { name: "dune".to_string(), constraint: Some("build".to_string()) },
            ]
        );
    }

    #[test]
    fn several_dependencies_read_in_order() {
        let opam = parse(
            "depends: [\n  \"satysfi-base\" {>= \"1.0.0\"}\n  \"satysfi-fonts-lmodern\"\n]\n",
        );
        assert_eq!(opam.depends.len(), 2);
        assert_eq!(opam.depends[0].name, "satysfi-base");
        assert_eq!(opam.depends[1].name, "satysfi-fonts-lmodern");
        assert_eq!(opam.depends[1].constraint, None);
    }

    #[test]
    fn a_depends_word_in_prose_is_not_a_field() {
        let opam = parse("description: \"this depends: on nothing\"\n");
        assert!(opam.depends.is_empty());
    }
}
