//! Minimal S-expression reader for real (OCaml) Satyrographos `Satyristes`
//! build files (plan §5.5, phase 4). This is an alternative *front-end* for
//! the same [`crate::manifest::PackagePlan`] the `rustyfi-package.toml`
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
//! Inside `(library …)`: `(name "…")`, `(version "…")`, `(lang 0.0|0.1)`,
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
//! | `(doc "dst" "src")` | `Doc` (also how a `(libraryDoc ...)` block's own
//!   `(sources ...)` are read — see [`doc_target_plan`]) |
//! | `(file "dst" "src")` | `File` |

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::manifest::{self, FileDecl, FileKind, Lang, Manifest, PackageMeta, PackagePlan};
use crate::source::{LibraryEntry, RegistryConfig, RegistryKind, SourceSpec};
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

/// A `(libraryDoc ...)` block: a document built FROM a library rather than a
/// library itself. Unlike `(library ...)` it carries `(build ...)` command
/// lines, which is the whole point — the products are made by running a
/// typesetter, not copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocTarget {
    /// The doc's own name, e.g. `rustyfi-manual-doc`.
    pub name: String,
    pub version: String,
    /// Each `(build (CMD ARG ...))` line, in declaration order. The first
    /// token is the program; `rustyfi` and `satysfi` mean "the typesetter",
    /// resolved by the caller.
    pub build: Vec<Vec<String>>,
    /// `(sources ((doc "dst" "src") ...))` — what the build produces. `src` is
    /// relative to the MANIFEST, not to [`Self::working_directory`].
    pub sources: Vec<(String, String)>,
    /// Which SATySFi generation this doc is written for, `(lang 0.0|0.1)`.
    pub lang: Lang,
    /// `(workingDirectory "dir")` — where the build commands run, relative to
    /// the manifest. Upstream manifests keep a doc's sources in `doc/` and
    /// build there, so the command line names `great-package.saty` while the
    /// product is declared as `doc/great-package.pdf`.
    pub working_directory: Option<String>,
}

/// One `(library ...)` block, before file-existence resolution.
#[derive(Debug)]
struct Library {
    name: String,
    lang: Lang,
    opam: Option<String>,
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
/// `$RUSTYFI_DEBUG` rather than a hard dependency.
fn ignore_note(kind: &str) {
    if std::env::var_os("RUSTYFI_DEBUG").is_some() {
        eprintln!("satyristes: ignoring `{kind}` form (out of scope through phase 3)");
    }
}

/// Every `(libraryDoc ...)` block, in declaration order.
pub fn doc_targets(source_root: &Path) -> Result<Vec<DocTarget>, Error> {
    let path = source_root.join(SATYRISTES_NAME);
    let text = util::read_to_string(&path)?;
    let forms = parse(&text).map_err(|pe| Error::Satyristes {
        path: path.clone(),
        message: pe.to_string(),
    })?;
    split_forms(&forms)
        .map(|(_, docs)| docs)
        .map_err(|message| Error::Satyristes { path, message })
}

/// Turn a `(libraryDoc ...)`'s declared products into an installable
/// [`PackagePlan`], using the same `FileKind::Doc` → `dist/doc/<name>/<dst>`
/// destination convention a `(library ...)`'s own `doc` sources get.
///
/// This does not run any build command or check that a source file exists —
/// [`crate::ops::build::build`] does that first; `sources` is expected to be
/// already filtered to the pairs whose `src` is actually present on disk (a
/// declared-but-unwritten product is a manifest bug `build`'s own report
/// already surfaces, not something to fail an install over). `source_root` is
/// the directory holding the `Satyristes` — `src` is relative to it, per
/// [`DocTarget::sources`]'s own doc comment.
pub fn doc_target_plan(
    source_root: &Path,
    name: &str,
    version: &str,
    lang: Lang,
    sources: &[(String, String)],
) -> Result<PackagePlan, Error> {
    let files: Vec<FileDecl> = sources
        .iter()
        .map(|(dst, src)| FileDecl {
            kind: FileKind::Doc,
            src: src.clone(),
            dst: Some(dst.clone()),
        })
        .collect();
    let manifest = Manifest {
        package: PackageMeta {
            name: name.to_string(),
            version: version.to_string(),
            rustyfi_version_compat: String::new(),
            description: None,
            lang,
        },
        files,
        dependencies: BTreeMap::new(),
    };
    manifest::plan_from_manifest(source_root, manifest)
}

/// The `(library ...)` blocks alone. Only the unit tests want this view;
/// `read` needs the docs too, so it calls [`split_forms`] directly.
#[cfg(test)]
fn libraries_from(forms: &[Sexp]) -> Result<Vec<Library>, String> {
    Ok(split_forms(forms)?.0)
}

/// A `Satyristes` read as a PROJECT manifest: the dependencies that name a
/// source, plus the registry the project prefers.
///
/// The same file serves both roles. `(library ...)` describes what this
/// directory PUBLISHES; the dependency payloads describe what it CONSUMES, and
/// this view is the consuming half — the role `Satyristes` used to play.
#[derive(Debug, Default)]
pub struct Project {
    /// Every dependency that named a source, in declaration order.
    pub libraries: Vec<LibraryEntry>,
    /// The top-level `(registry ...)` form, if present.
    pub registry: Option<RegistryConfig>,
}

impl Project {
    pub fn registry_url(&self) -> Option<&str> {
        self.registry.as_ref().and_then(|r| r.url.as_deref())
    }

    pub fn registry_mirrors(&self) -> &[String] {
        self.registry.as_ref().map(|r| r.mirrors.as_slice()).unwrap_or(&[])
    }

    pub fn registry_kind(&self) -> Option<RegistryKind> {
        self.registry.as_ref().and_then(|r| r.kind)
    }
}

/// Read `path` as a project manifest.
pub fn read_project(path: &Path) -> Result<Project, Error> {
    let text = util::read_to_string(path)?;
    let forms = parse(&text).map_err(|pe| Error::Satyristes {
        path: path.to_path_buf(),
        message: pe.to_string(),
    })?;
    let mut project = Project::default();
    for form in &forms {
        let Sexp::List(list) = form else { continue };
        match head(list) {
            Some("registry") => project.registry = Some(parse_registry(&list[1..])),
            Some("library") => {
                for item in &list[1..] {
                    let Sexp::List(field) = item else { continue };
                    if head(field) == Some("dependencies") {
                        for (name, source) in parse_dependency_entries(&field[1..]) {
                            match source {
                                Ok(Some(source)) => {
                                    project.libraries.push(LibraryEntry { name, source })
                                }
                                Ok(None) => {}
                                Err(kind) => return Err(Error::UnsupportedSource { kind }),
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(project)
}

/// `(registry (url "…") (kind git|sparse|auto) (mirrors ("…" "…")))` — the
/// project-level registry, previously `Satyristes`'s `[registry]` table.
fn parse_registry(items: &[Sexp]) -> RegistryConfig {
    let mut cfg = RegistryConfig::default();
    for item in items {
        let Sexp::List(kv) = item else { continue };
        match head(kv) {
            Some("url") => cfg.url = kv.get(1).and_then(scalar).map(str::to_string),
            Some("kind") => {
                cfg.kind = match kv.get(1).and_then(scalar) {
                    Some("git") => Some(RegistryKind::Git),
                    Some("sparse") => Some(RegistryKind::Sparse),
                    Some("auto") => Some(RegistryKind::Auto),
                    _ => None,
                }
            }
            Some("mirrors") => {
                if let Some(Sexp::List(urls)) = kv.get(1) {
                    cfg.mirrors = urls.iter().filter_map(scalar).map(str::to_string).collect();
                }
            }
            Some(other) => ignore_note(other),
            None => {}
        }
    }
    cfg
}

/// The `.opam` files the `(library ...)` blocks claim as their own.
///
/// A package directory may hold several — `satysfi-fonts-theano` ships one for
/// the fonts and one for its documentation — and only the libraries' own
/// matter here. The doc one's `build:` is OPAM calling Satyrographos to
/// install the built PDF, which this port does from the manifest itself.
///
/// Reads the manifest WITHOUT planning, because planning walks the source
/// files, and for a font package those do not exist until the opam build has
/// run. This is what breaks that circle.
pub fn library_opam_files(source_root: &Path) -> Vec<PathBuf> {
    let path = source_root.join(SATYRISTES_NAME);
    let Ok(text) = util::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(forms) = parse(&text) else {
        return Vec::new();
    };
    let Ok((libs, _docs)) = split_forms(&forms) else {
        return Vec::new();
    };
    libs.into_iter()
        .filter_map(|l| l.opam)
        .map(|f| source_root.join(f))
        .filter(|p| p.is_file())
        .collect()
}

/// The nearest `Satyristes` at or above `start`, if any — how a project
/// manifest is located when none is named.
pub fn find_upward(start: &Path) -> Option<PathBuf> {
    let start = start
        .canonicalize()
        .or_else(|_| std::path::absolute(start))
        .unwrap_or_else(|_| start.to_path_buf());
    let mut dir = Some(start.as_path());
    while let Some(d) = dir {
        let candidate = d.join(SATYRISTES_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    None
}

/// One pass over the top-level forms, sorting them into libraries and docs.
fn split_forms(forms: &[Sexp]) -> Result<(Vec<Library>, Vec<DocTarget>), String> {
    let mut docs = Vec::new();
    let mut libs = Vec::new();
    for form in forms {
        let list = match form {
            Sexp::List(l) if !l.is_empty() => l,
            _ => return Err("top-level form must be a non-empty list".to_string()),
        };
        let kind = head(list).ok_or("top-level form must start with a symbol")?;
        match kind {
            "library" => libs.push(parse_library(&list[1..])?),
            "libraryDoc" => docs.push(parse_library_doc(&list[1..])?),
            // Parsed and ignored (plan §5.5). `version` here is the
            // *Satyristes format* version, distinct from a library's own.
            "version" | "opam" | "compatibility" => ignore_note(kind),
            other => return Err(format!("unknown top-level form `{other}`")),
        }
    }
    Ok((libs, docs))
}

/// `(libraryDoc (name ...) (version ...) (build ((cmd arg ...) ...))
///              (sources ((doc "dst" "src") ...)) (dependencies ...))`.
/// `dependencies` is accepted and ignored: nothing here installs, so there is
/// nothing to order.
fn parse_library_doc(items: &[Sexp]) -> Result<DocTarget, String> {
    let mut name = None;
    let mut version = None;
    let mut build = Vec::new();
    let mut sources = Vec::new();
    let mut working_directory = None;
    let mut lang = Lang::default();
    for item in items {
        let list = match item {
            Sexp::List(l) if !l.is_empty() => l,
            _ => return Err("`libraryDoc` field must be a non-empty list".to_string()),
        };
        match head(list).ok_or("`libraryDoc` field must start with a symbol")? {
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
                        .ok_or("`(version ...)` needs a string value")?
                        .to_string(),
                )
            }
            "build" => {
                let lines = match list.get(1) {
                    Some(Sexp::List(l)) => l,
                    _ => return Err("`(build ...)` needs a list of command lines".to_string()),
                };
                for line in lines {
                    let Sexp::List(words) = line else {
                        return Err("each `build` entry must be a command line list".to_string());
                    };
                    let cmd: Option<Vec<String>> =
                        words.iter().map(|w| scalar(w).map(str::to_string)).collect();
                    let cmd = cmd.ok_or("a `build` command line must be all scalars")?;
                    if cmd.is_empty() {
                        return Err("a `build` command line must name a program".to_string());
                    }
                    build.push(cmd);
                }
            }
            "sources" => {
                let decls = match list.get(1) {
                    Some(Sexp::List(l)) => l,
                    _ => return Err("`(sources ...)` needs a list".to_string()),
                };
                for decl in decls {
                    let Sexp::List(d) = decl else {
                        return Err("each `sources` entry must be a list".to_string());
                    };
                    match head(d) {
                        Some("doc") => {
                            let dst = d.get(1).and_then(scalar).ok_or("`(doc ...)` needs a dst")?;
                            let src = d.get(2).and_then(scalar).ok_or("`(doc ...)` needs a src")?;
                            sources.push((dst.to_string(), src.to_string()));
                        }
                        // A doc may also ship non-`doc` files; nothing here
                        // installs them, so they are read past rather than
                        // rejected.
                        _ => ignore_note("libraryDoc source"),
                    }
                }
            }
            "lang" => lang = parse_lang(list)?,
            "workingDirectory" => {
                working_directory = Some(
                    list.get(1)
                        .and_then(scalar)
                        .ok_or("`(workingDirectory ...)` needs a string value")?
                        .to_string(),
                )
            }
            // Anything else is read past rather than rejected. These manifests
            // are written for upstream Satyrographos, which has fields this
            // port does not model; refusing them would make an unrelated
            // `install` fail on a doc block it never even looks at.
            other => ignore_note(other),
        }
    }
    Ok(DocTarget {
        name: name.ok_or("`libraryDoc` needs a `(name ...)`")?,
        version: version.unwrap_or_default(),
        build,
        sources,
        working_directory,
        lang,
    })
}

/// `(lang 0.0)` / `(lang "0.1")` — which SATySFi generation the block is
/// written for. A block that says nothing is 0.0, which is what every manifest
/// written before this existed meant.
fn parse_lang(list: &[Sexp]) -> Result<Lang, String> {
    let value = list.get(1).and_then(scalar).ok_or("`(lang ...)` needs a value")?;
    Lang::parse(value).ok_or_else(|| format!("unknown `lang` value `{value}` (expected 0.0 or 0.1)"))
}

fn parse_library(items: &[Sexp]) -> Result<Library, String> {
    let mut name = None;
    let mut version = None;
    let mut lang = Lang::default();
    let mut opam = None;
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
            "lang" => lang = parse_lang(list)?,
            "opam" => opam = list.get(1).and_then(scalar).map(str::to_string),
            "compatibility" | "libraryDoc" => ignore_note(field),
            other => return Err(format!("unknown `library` field `{other}`")),
        }
    }
    Ok(Library {
        name: name.ok_or("`library` is missing a `(name ...)`")?,
        version: version.ok_or("`library` is missing a `(version ...)`")?,
        lang,
        opam,
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
        "doc" => two_args(FileKind::Doc, kind, &args)?,
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
    parse_dependency_entries(args)
        .into_iter()
        .map(|(name, _)| (name, "*".to_string()))
        .collect()
}

/// `(dependencies ((name (SOURCE ...)) ...))` — each entry's name plus, when
/// the payload names one, where to get it.
///
/// Upstream's payload is a version-constraint list, and an empty `()` still
/// means exactly what it always did: a declared dependency with no source,
/// which nothing can materialise. A payload naming a source is this port's
/// extension, and it is what makes `Satyristes` a PROJECT manifest — upstream
/// gets sources from OPAM, which this port does not have.
///
/// ```text
/// (dependencies
///   ((xpath   ((path "../vendor/xpath")))
///    (theano  ((registry "fonts-theano") (version "1.0.0")))
///    (grafite ((git "https://…") (rev "abc1234")))
///    (base    ())))
/// ```
fn parse_dependency_entries(args: &[Sexp]) -> Vec<(String, Result<Option<SourceSpec>, String>)> {
    let mut deps = Vec::new();
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
                    deps.push((name.to_string(), source_from(&pair[1..])));
                }
            }
        }
    }
    deps
}

/// A source out of a dependency's payload, if it names one.
fn source_from(payload: &[Sexp]) -> Result<Option<SourceSpec>, String> {
    let forms = match payload.first() {
        Some(Sexp::List(l)) if matches!(l.first(), Some(Sexp::List(_))) => l.as_slice(),
        Some(Sexp::List(_)) => payload,
        _ => return Ok(None),
    };
    let mut spec = SourceSpec::default();
    let mut unknown: Option<String> = None;
    for form in forms {
        let Sexp::List(kv) = form else { continue };
        let (Some(key), Some(value)) = (head(kv), kv.get(1).and_then(scalar)) else {
            continue;
        };
        match key {
            "path" => spec.path = Some(value.to_string()),
            "git" => spec.git = Some(value.to_string()),
            "rev" => spec.rev = Some(value.to_string()),
            "registry" => spec.registry = Some(value.to_string()),
            "version" => spec.version = Some(value.to_string()),
            other => unknown.get_or_insert_with(|| other.to_string()).clone_from(&other.to_string()),
        }
    }
    if spec != SourceSpec::default() {
        return Ok(Some(spec));
    }
    // `()` is upstream's "declared, no constraints" — a dependency with no
    // source, which nothing materialises. A payload that names something we do
    // not recognise is a different thing: a typo or an unsupported kind, and
    // silently treating it as "no source" would drop the dependency.
    match unknown {
        Some(kind) => Err(kind),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Public entry point.
// ---------------------------------------------------------------------------

/// Read the `Satyristes` at `source_root/Satyristes`, returning one
/// [`PackagePlan`] per `(library ...)` block (destination logic shared with
/// the `rustyfi-package.toml` front-end via
/// [`crate::manifest::plan_from_manifest`]).
pub fn read(source_root: &Path) -> Result<Vec<PackagePlan>, Error> {
    let path = source_root.join(SATYRISTES_NAME);
    let text = util::read_to_string(&path)?;
    let forms = parse(&text).map_err(|pe| Error::Satyristes {
        path: path.clone(),
        message: pe.to_string(),
    })?;
    let (libs, docs) = split_forms(&forms).map_err(|message| Error::Satyristes {
        path: path.clone(),
        message,
    })?;
    if libs.is_empty() {
        // A doc-only manifest is legitimate — it just has nothing to INSTALL,
        // so say where its targets are handled instead of only what is absent.
        let message = if docs.is_empty() {
            "no `(library ...)` block found".to_string()
        } else {
            format!(
                "no `(library ...)` block found; this manifest declares only doc \
                 target(s) (`{}`), which `satyrographos build` runs",
                docs.iter()
                    .map(|d| d.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        return Err(Error::Satyristes { path, message });
    }
    let mut plans = Vec::with_capacity(libs.len());
    for lib in libs {
        let manifest = Manifest {
            package: PackageMeta {
                name: lib.name,
                version: lib.version,
                // No Satyristes analog: recorded empty; phase 1 never gates
                // on it (plan §5.1/§10).
                rustyfi_version_compat: String::new(),
                description: None,
                lang: lib.lang,
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
  (opam "rustyfi-great-package.opam")
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
    fn a_library_may_declare_a_doc_source_directly() {
        // `(doc "dst" "src")` used to parse fine as a source declaration and
        // then be rejected as an "unknown source kind" — there was no
        // `FileKind` for it. It now maps to `FileKind::Doc`, the same
        // dst/src shape `md`/`font`/`file` already use.
        let text = r#"(library (name "p") (version "1")
            (sources ((packageDir "packages") (doc "manual.pdf" "docs/manual.pdf"))))"#;
        let libs = libraries_from(&parse(text).unwrap()).unwrap();
        assert_eq!(libs[0].sources.len(), 2);
        let doc = &libs[0].sources[1];
        assert_eq!(doc.kind, FileKind::Doc);
        assert_eq!(doc.dst.as_deref(), Some("manual.pdf"));
        assert_eq!(doc.src, "docs/manual.pdf");
    }

    #[test]
    fn doc_target_plan_installs_under_dist_doc() {
        // A `(libraryDoc ...)`'s own declared products, once built, land at
        // `dist/doc/<name>/<dst>` — the same per-library namespace `md` uses
        // — via the exact `manifest::plan_from_manifest` destination logic
        // every other source kind goes through.
        let dir = std::env::temp_dir().join(format!(
            "rustyfi-doctargetplan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("out.pdf"), b"pdf bytes").unwrap();

        let plan = doc_target_plan(
            &dir,
            "p-doc",
            "1.0",
            Lang::V0_0,
            &[("manual.pdf".to_string(), "out.pdf".to_string())],
        )
        .expect("plan");
        assert_eq!(plan.name, "p-doc");
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].dst, "dist/doc/p-doc/manual.pdf");

        let _ = std::fs::remove_dir_all(&dir);
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
