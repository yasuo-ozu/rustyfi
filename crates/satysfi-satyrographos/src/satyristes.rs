//! Minimal S-expression reader for real (OCaml) Satyrographos `Satyristes`
//! build files (plan §5.5, phase 4). This is an alternative *front-end* for
//! the same [`crate::manifest::PackagePlan`] the `satysfi-package.toml`
//! parser produces — a `Satyristes` is read into one `manifest::Manifest`
//! per `(library ...)` block and run through the exact same
//! [`crate::manifest::plan_from_manifest`] destination logic, so the two
//! formats stay byte-for-byte identical in where files land.
//!
//! ## Grammar (hand-rolled, ~cf. sexplib)
//!
//! - lists `( … )`, bare atoms (`fontDir`, `0.0.2`, `fonts-theano`),
//!   double-quoted strings with `\\ \" \n \t \r` escapes;
//! - `;`-to-end-of-line comments (the README's `Satyristes` uses `;;`).
//!
//! Deliberately **not** built on `syan2`: that reader is scoped to the
//! SATySFi grammar, a different S-expression entirely (plan §8).
//!
//! ## Mapping (plan §5.5, verified verbatim against the upstream README)
//!
//! Top-level: `(library …)` is read; `(version …)` (the *Satyristes format*
//! version), `(opam …)`, `(libraryDoc …)`, `(compatibility …)` are
//! parsed-and-ignored; anything else is a hard error naming the form.
//!
//! Inside `(library …)`: `(name "…")`, `(version "…")`,
//! `(sources ((KIND …) …))`, `(dependencies ((name ()) …))`; `(opam …)`,
//! `(compatibility …)`, `(libraryDoc …)` ignored; anything else errors.
//!
//! | source declaration | `manifest::FileKind` |
//! |---|---|
//! | `(packageDir "src")` | `PackageDir` |
//! | `(package "dst" "src")` | `Package` |
//! | `(fontDir "src")` | `FontDir` |
//! | `(font "dst" "src")` | `Font` |
//! | `(hash "dst" "src")` | `Hash` (lands flat in `dist/hash/<dst>`) |
//! | `(md "dst" "src")` | `Md` |
//! | `(file "dst" "src")` | `File` |

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::Error;
use crate::manifest::{self, FileDecl, FileKind, Manifest, PackageMeta, PackagePlan};
use crate::util;

/// The upstream build-file name (plan §2/§5.5).
pub const SATYRISTES_NAME: &str = "Satyristes";

// ---------------------------------------------------------------------------
// Reader: tokenizer + recursive-descent S-expression parser.
// ---------------------------------------------------------------------------

/// A parsed S-expression node.
#[derive(Debug, Clone, PartialEq)]
enum Sexp {
    /// A bare, unquoted atom (`fontDir`, `0.0.2`, `fonts-theano`).
    Atom(String),
    /// A double-quoted string literal (escapes already resolved).
    Str(String),
    /// A parenthesised list.
    List(Vec<Sexp>),
}

/// A reader-level failure, carrying a `line:col:` position (plan §9: "with
/// positions if cheap").
#[derive(Debug)]
struct ParseError {
    line: usize,
    col: usize,
    msg: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.msg)
    }
}

/// Char-buffer reader with 1-based line/column tracking.
struct Reader {
    src: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Reader {
    fn new(input: &str) -> Self {
        Reader {
            src: input.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.src.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError {
            line: self.line,
            col: self.col,
            msg: msg.into(),
        }
    }

    /// Skip whitespace and `;`-to-end-of-line comments.
    fn skip_trivia(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some(';') => {
                    while let Some(c) = self.peek() {
                        self.bump();
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    /// Parse every top-level form until end of input.
    fn parse_all(&mut self) -> Result<Vec<Sexp>, ParseError> {
        let mut forms = Vec::new();
        loop {
            self.skip_trivia();
            if self.peek().is_none() {
                return Ok(forms);
            }
            forms.push(self.parse_sexp()?);
        }
    }

    fn parse_sexp(&mut self) -> Result<Sexp, ParseError> {
        self.skip_trivia();
        match self.peek() {
            None => Err(self.err("unexpected end of input")),
            Some('(') => self.parse_list(),
            Some(')') => Err(self.err("unexpected `)`")),
            Some('"') => self.parse_string(),
            Some(_) => self.parse_atom(),
        }
    }

    fn parse_list(&mut self) -> Result<Sexp, ParseError> {
        self.bump(); // consume '('
        let mut items = Vec::new();
        loop {
            self.skip_trivia();
            match self.peek() {
                None => return Err(self.err("unterminated list: missing `)`")),
                Some(')') => {
                    self.bump();
                    return Ok(Sexp::List(items));
                }
                Some(_) => items.push(self.parse_sexp()?),
            }
        }
    }

    fn parse_string(&mut self) -> Result<Sexp, ParseError> {
        self.bump(); // consume opening '"'
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err("unterminated string literal")),
                Some('"') => return Ok(Sexp::Str(out)),
                Some('\\') => match self.bump() {
                    None => return Err(self.err("unterminated escape in string literal")),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('\\') => out.push('\\'),
                    Some('"') => out.push('"'),
                    // Unknown escape: keep the character verbatim (lenient,
                    // matching how sexplib tolerates stray backslashes for
                    // the simple path strings a Satyristes carries).
                    Some(other) => out.push(other),
                },
                Some(c) => out.push(c),
            }
        }
    }

    fn parse_atom(&mut self) -> Result<Sexp, ParseError> {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || matches!(c, '(' | ')' | ';' | '"') {
                break;
            }
            out.push(c);
            self.bump();
        }
        // `parse_sexp` only dispatches here on a non-delimiter char, so `out`
        // is always non-empty.
        Ok(Sexp::Atom(out))
    }
}

fn parse(input: &str) -> Result<Vec<Sexp>, ParseError> {
    Reader::new(input).parse_all()
}

// ---------------------------------------------------------------------------
// Interpretation: S-expressions -> library declarations -> PackagePlan.
// ---------------------------------------------------------------------------

/// One `(library ...)` block, before file-existence resolution.
#[derive(Debug)]
struct Library {
    name: String,
    version: String,
    sources: Vec<FileDecl>,
    dependencies: BTreeMap<String, String>,
}

/// The scalar payload of an `Atom` or `Str` (name/version/path values may be
/// written either way; the README uses strings).
fn scalar(s: &Sexp) -> Option<&str> {
    match s {
        Sexp::Atom(a) => Some(a),
        Sexp::Str(s) => Some(s),
        Sexp::List(_) => None,
    }
}

/// The leading symbol of a list form, if it starts with an atom.
fn head(list: &[Sexp]) -> Option<&str> {
    match list.first() {
        Some(Sexp::Atom(a)) => Some(a.as_str()),
        _ => None,
    }
}

/// Emit a debug-level note for a parsed-and-ignored form (plan §5.5). No log
/// framework is wired into this crate, so this stays a stderr line gated on
/// `$SATYSFI_DEBUG` rather than a hard dependency.
fn ignore_note(kind: &str) {
    if std::env::var_os("SATYSFI_DEBUG").is_some() {
        eprintln!("satyristes: ignoring `{kind}` form (out of scope through phase 3)");
    }
}

/// Read every `(library ...)` block from a parsed Satyristes.
fn libraries_from(forms: &[Sexp]) -> Result<Vec<Library>, String> {
    let mut libs = Vec::new();
    for form in forms {
        let list = match form {
            Sexp::List(l) if !l.is_empty() => l,
            _ => return Err("top-level form must be a non-empty list".to_string()),
        };
        let kind = head(list).ok_or("top-level form must start with a symbol")?;
        match kind {
            "library" => libs.push(parse_library(&list[1..])?),
            // Parsed and ignored (plan §5.5). `version` here is the
            // *Satyristes format* version, distinct from a library's own.
            "version" | "opam" | "libraryDoc" | "compatibility" => ignore_note(kind),
            other => return Err(format!("unknown top-level form `{other}`")),
        }
    }
    Ok(libs)
}

fn parse_library(items: &[Sexp]) -> Result<Library, String> {
    let mut name = None;
    let mut version = None;
    let mut sources = Vec::new();
    let mut dependencies = BTreeMap::new();
    for item in items {
        let list = match item {
            Sexp::List(l) if !l.is_empty() => l,
            _ => return Err("`library` field must be a non-empty list".to_string()),
        };
        let field = head(list).ok_or("`library` field must start with a symbol")?;
        match field {
            "name" => {
                name = Some(
                    list.get(1)
                        .and_then(scalar)
                        .ok_or("`(name ...)` needs a string value")?
                        .to_string(),
                )
            }
            "version" => {
                version = Some(
                    list.get(1)
                        .and_then(scalar)
                        .ok_or("`(version ...)` needs a value")?
                        .to_string(),
                )
            }
            "sources" => sources = parse_sources(&list[1..])?,
            "dependencies" => dependencies = parse_dependencies(&list[1..]),
            "opam" | "compatibility" | "libraryDoc" => ignore_note(field),
            other => return Err(format!("unknown `library` field `{other}`")),
        }
    }
    Ok(Library {
        name: name.ok_or("`library` is missing a `(name ...)`")?,
        version: version.ok_or("`library` is missing a `(version ...)`")?,
        sources,
        dependencies,
    })
}

/// Parse the `(sources ((KIND ...) ...))` body into `FileDecl`s. Accepts both
/// the README's wrapped `((decl) (decl))` list-of-lists and a bare sequence
/// of `(decl)` forms.
fn parse_sources(args: &[Sexp]) -> Result<Vec<FileDecl>, String> {
    let mut decls = Vec::new();
    for arg in args {
        match arg {
            Sexp::List(children) if matches!(children.first(), Some(Sexp::List(_))) => {
                for child in children {
                    decls.push(parse_source_decl(child)?);
                }
            }
            Sexp::List(_) => decls.push(parse_source_decl(arg)?),
            _ => return Err("`sources` entry must be a list".to_string()),
        }
    }
    Ok(decls)
}

fn parse_source_decl(sexp: &Sexp) -> Result<FileDecl, String> {
    let list = match sexp {
        Sexp::List(l) if !l.is_empty() => l,
        _ => return Err("source declaration must be a non-empty list".to_string()),
    };
    let kind = head(list).ok_or("source declaration must start with a symbol")?;
    // Collect the string arguments after the kind symbol.
    let args: Vec<&str> = list[1..]
        .iter()
        .map(|s| scalar(s).ok_or_else(|| format!("`{kind}` arguments must be strings")))
        .collect::<Result<_, _>>()?;

    // `*-dir` kinds take a single `src`; every other kind takes `dst` then
    // `src` (plan §5.5).
    let (file_kind, dst, src) = match kind {
        "packageDir" => (FileKind::PackageDir, None, one_arg(kind, &args)?),
        "fontDir" => (FileKind::FontDir, None, one_arg(kind, &args)?),
        "package" => two_args(FileKind::Package, kind, &args)?,
        "font" => two_args(FileKind::Font, kind, &args)?,
        "hash" => two_args(FileKind::Hash, kind, &args)?,
        "md" => two_args(FileKind::Md, kind, &args)?,
        "file" => two_args(FileKind::File, kind, &args)?,
        other => return Err(format!("unknown source kind `{other}`")),
    };
    Ok(FileDecl {
        kind: file_kind,
        src,
        dst,
    })
}

fn one_arg(kind: &str, args: &[&str]) -> Result<String, String> {
    match args {
        [src] => Ok(src.to_string()),
        _ => Err(format!("`{kind}` takes exactly one argument (src), got {}", args.len())),
    }
}

fn two_args(
    fk: FileKind,
    kind: &str,
    args: &[&str],
) -> Result<(FileKind, Option<String>, String), String> {
    match args {
        [dst, src] => Ok((fk, Some(dst.to_string()), src.to_string())),
        _ => Err(format!(
            "`{kind}` takes exactly two arguments (dst src), got {}",
            args.len()
        )),
    }
}

/// Parse `(dependencies ((name ()) ...))`. Upstream's `()` carries no version
/// (real constraints live in the sibling `.opam`, no analog here), so every
/// dependency is recorded with the wildcard `"*"` constraint (plan §5.5).
fn parse_dependencies(args: &[Sexp]) -> BTreeMap<String, String> {
    let mut deps = BTreeMap::new();
    for arg in args {
        let entries = match arg {
            Sexp::List(children) if matches!(children.first(), Some(Sexp::List(_))) => {
                children.as_slice()
            }
            other => std::slice::from_ref(other),
        };
        for entry in entries {
            if let Sexp::List(pair) = entry {
                if let Some(name) = pair.first().and_then(scalar) {
                    deps.insert(name.to_string(), "*".to_string());
                }
            }
        }
    }
    deps
}

// ---------------------------------------------------------------------------
// Public entry point.
// ---------------------------------------------------------------------------

/// Read the `Satyristes` at `source_root/Satyristes`, returning one
/// [`PackagePlan`] per `(library ...)` block (destination logic shared with
/// the `satysfi-package.toml` front-end via
/// [`crate::manifest::plan_from_manifest`]).
pub fn read(source_root: &Path) -> Result<Vec<PackagePlan>, Error> {
    let path = source_root.join(SATYRISTES_NAME);
    let text = util::read_to_string(&path)?;
    let forms = parse(&text).map_err(|pe| Error::Satyristes {
        path: path.clone(),
        message: pe.to_string(),
    })?;
    let libs = libraries_from(&forms).map_err(|message| Error::Satyristes {
        path: path.clone(),
        message,
    })?;
    if libs.is_empty() {
        return Err(Error::Satyristes {
            path,
            message: "no `(library ...)` block found".to_string(),
        });
    }
    let mut plans = Vec::with_capacity(libs.len());
    for lib in libs {
        let manifest = Manifest {
            package: PackageMeta {
                name: lib.name,
                version: lib.version,
                // No Satyristes analog: recorded empty; phase 1 never gates
                // on it (plan §5.1/§10).
                satysfi_version_compat: String::new(),
                description: None,
            },
            files: lib.sources,
            dependencies: lib.dependencies,
        };
        plans.push(manifest::plan_from_manifest(source_root, manifest)?);
    }
    Ok(plans)
}

// ---------------------------------------------------------------------------
// Reader unit tests (parse-level; no filesystem — plan §9).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(s: &str) -> Sexp {
        Sexp::Atom(s.to_string())
    }
    fn string(s: &str) -> Sexp {
        Sexp::Str(s.to_string())
    }
    fn list(v: Vec<Sexp>) -> Sexp {
        Sexp::List(v)
    }

    #[test]
    fn parses_atoms_strings_and_nesting() {
        let forms = parse("(a \"b c\" (d (e)))").unwrap();
        assert_eq!(
            forms,
            vec![list(vec![
                atom("a"),
                string("b c"),
                list(vec![atom("d"), list(vec![atom("e")])]),
            ])]
        );
    }

    #[test]
    fn skips_line_comments() {
        // `;;`-style comments as the README's Satyristes uses.
        let forms = parse(";; header\n(x 1) ; trailing\n(y 2)").unwrap();
        assert_eq!(
            forms,
            vec![
                list(vec![atom("x"), atom("1")]),
                list(vec![atom("y"), atom("2")]),
            ]
        );
    }

    #[test]
    fn resolves_string_escapes() {
        let forms = parse(r#"("a\tb\nc\"d\\e")"#).unwrap();
        assert_eq!(forms, vec![list(vec![string("a\tb\nc\"d\\e")])]);
    }

    #[test]
    fn dotted_atoms_and_hyphens() {
        // `0.0.2` and `fonts-theano` are single atoms.
        let forms = parse("(version 0.0.2) (dep fonts-theano)").unwrap();
        assert_eq!(
            forms,
            vec![
                list(vec![atom("version"), atom("0.0.2")]),
                list(vec![atom("dep"), atom("fonts-theano")]),
            ]
        );
    }

    #[test]
    fn malformed_unterminated_list_reports_position() {
        let err = parse("(a (b c)").unwrap_err();
        assert!(err.msg.contains("unterminated list"), "{}", err.msg);
        // Position is reported (non-zero line/col).
        assert!(err.line >= 1 && err.col >= 1);
    }

    #[test]
    fn malformed_unterminated_string() {
        let err = parse("(a \"oops)").unwrap_err();
        assert!(err.msg.contains("unterminated string"), "{}", err.msg);
    }

    #[test]
    fn stray_close_paren_errors() {
        let err = parse("a )").unwrap_err();
        assert!(err.msg.contains("unexpected `)`"), "{}", err.msg);
    }

    #[test]
    fn readme_great_package_libraries() {
        // The upstream README's own great-package Satyristes (verbatim).
        let text = r#"
;; For Satyrographos 0.0.2 series
(version 0.0.2)

;; Library declaration
(library
  ;; Library name
  (name "great-package")
  ;; Library version
  (version "1.0")
  ;; Files
  (sources
    ((fontDir "fonts")
     (hash "fonts.satysfi-hash" "hash/fonts.satysfi-hash")
     (packageDir "packages")))
  ;; OPAM package file
  (opam "satysfi-great-package.opam")
  ;; Dependency
  (dependencies ((fonts-theano ()))))
"#;
        let forms = parse(text).unwrap();
        let libs = libraries_from(&forms).unwrap();
        assert_eq!(libs.len(), 1);
        let lib = &libs[0];
        assert_eq!(lib.name, "great-package");
        assert_eq!(lib.version, "1.0");
        // Three source declarations, mapped per §5.5's table.
        assert_eq!(lib.sources.len(), 3);
        let font_dir = &lib.sources[0];
        assert_eq!(font_dir.kind, FileKind::FontDir);
        assert_eq!(font_dir.src, "fonts");
        assert_eq!(font_dir.dst, None);
        let hash = &lib.sources[1];
        assert_eq!(hash.kind, FileKind::Hash);
        assert_eq!(hash.dst.as_deref(), Some("fonts.satysfi-hash"));
        assert_eq!(hash.src, "hash/fonts.satysfi-hash");
        let package_dir = &lib.sources[2];
        assert_eq!(package_dir.kind, FileKind::PackageDir);
        assert_eq!(package_dir.src, "packages");
        // opam ignored; one dependency, wildcard constraint.
        assert_eq!(lib.dependencies.get("fonts-theano").map(String::as_str), Some("*"));
    }

    #[test]
    fn library_doc_and_compatibility_ignored() {
        let text = r#"
(library (name "p") (version "1") (sources ((packageDir "packages"))))
(libraryDoc (name "p-doc") (version "1"))
(compatibility ())
"#;
        let libs = libraries_from(&parse(text).unwrap()).unwrap();
        assert_eq!(libs.len(), 1);
        assert_eq!(libs[0].name, "p");
    }

    #[test]
    fn unknown_top_level_form_errors_naming_it() {
        let err = libraries_from(&parse("(frobnicate 1)").unwrap()).unwrap_err();
        assert!(err.contains("frobnicate"), "{err}");
        assert!(err.contains("unknown top-level form"), "{err}");
    }

    #[test]
    fn unknown_source_kind_errors_naming_it() {
        let text = r#"(library (name "p") (version "1") (sources ((widget "a" "b"))))"#;
        let err = libraries_from(&parse(text).unwrap()).unwrap_err();
        assert!(err.contains("widget"), "{err}");
    }

    #[test]
    fn multiple_libraries_enumerated() {
        let text = r#"
(library (name "a") (version "1") (sources ((packageDir "packages"))))
(library (name "b") (version "2") (sources ((packageDir "packages"))))
"#;
        let libs = libraries_from(&parse(text).unwrap()).unwrap();
        assert_eq!(libs.len(), 2);
        assert_eq!(libs[0].name, "a");
        assert_eq!(libs[1].name, "b");
    }
}
