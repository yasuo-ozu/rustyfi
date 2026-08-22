//! `rustyfi-package.toml` parsing and the no-manifest fallback discovery,
//! producing a flat `PackagePlan`: the concrete list of (absolute source
//! file → root-relative destination) pairs that the installer stages and
//! materialises.
//!
//! The `kind` → destination mapping:
//!
//! | kind | destination |
//! |---|---|
//! | `package-dir` | recursively into `dist/packages/<name>/` |
//! | `package` | `dist/packages/<name>/<dst>` |
//! | `font-dir` | recursively into `dist/fonts/<name>/` |
//! | `font` | `dist/fonts/<name>/<dst>` |
//! | `hash` | `dist/hash/<dst>` — **flat, no per-library namespace** |
//! | `md` | `dist/md/<name>/<dst>` |
//! | `doc` | `dist/doc/<name>/<dst>` |
//! | `file` | `dist/<dst>` — arbitrary, root-relative |

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::util;

pub(crate) const MANIFEST_NAME: &str = "rustyfi-package.toml";

#[derive(Debug, Deserialize)]
pub(crate) struct Manifest {
    pub package: PackageMeta,
    #[serde(default)]
    pub files: Vec<FileDecl>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PackageMeta {
    pub name: String,
    pub version: String,
    /// Required, but phase 1 only *warns* against it — no hard gate, so
    /// this crate merely records it. Deliberately never read back out
    /// (see the doc above); `dead_code` would otherwise flag it now that
    /// `PackageMeta` is `pub(crate)` rather than fully public.
    #[serde(rename = "rustyfi-version-compat")]
    #[allow(dead_code)]
    pub rustyfi_version_compat: String,
    #[serde(default)]
    pub description: Option<String>,
    /// The SATySFi generation this package is written for. Absent means 0.0.
    #[serde(default)]
    pub lang: Lang,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FileDecl {
    pub kind: FileKind,
    /// Source path, relative to the source root.
    pub src: String,
    /// Destination, relative to the `dist/<kind-plural>/<name>/` prefix.
    /// Required for every kind *except* the `*-dir` kinds (which mirror a
    /// whole subtree).
    #[serde(default)]
    pub dst: Option<String>,
}

/// Which SATySFi language generation a library or doc target is written for.
///
/// The port bundles both corpora side by side and the loader keeps them apart
/// by DIRECTORY — `dist/packages/` is 0.0, `dist-v01/packages/` is 0.1 — so a
/// package's generation decides where it installs, not just what it says about
/// itself. Declared as `(lang 0.0)` / `(lang 0.1)`; absent means 0.0, which is
/// what every manifest written before this existed meant.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum Lang {
    #[default]
    #[serde(rename = "0.0")]
    V0_0,
    #[serde(rename = "0.1")]
    V0_1,
}

impl Lang {
    fn dist_dir(self) -> &'static str {
        match self {
            Lang::V0_0 => "dist",
            Lang::V0_1 => "dist-v01",
        }
    }

    /// As written in a manifest.
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::V0_0 => "0.0",
            Lang::V0_1 => "0.1",
        }
    }

    /// Parse `0.0`/`0.1` as a manifest writes them.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "0.0" => Some(Lang::V0_0),
            "0.1" => Some(Lang::V0_1),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum FileKind {
    PackageDir,
    Package,
    FontDir,
    Font,
    Hash,
    Md,
    /// A document product — a `(libraryDoc ...)`'s built PDF/etc, or a
    /// library's own pre-built doc file. Lands at `dist/doc/<name>/<dst>`,
    /// the same per-library-namespace convention `Md` uses.
    Doc,
    File,
}

/// One planned copy: absolute source file → root-relative destination
/// (`/`-separated).
#[derive(Debug, Clone)]
pub(crate) struct PlannedFile {
    pub src: PathBuf,
    pub dst: String,
    /// Merge into whatever is already at `dst` instead of replacing it — true
    /// for `*.satysfi-hash`, which every font package contributes entries to
    /// (see [`crate::hashfile`]). For everything else the destination belongs
    /// to one package and a copy is right.
    pub merge: bool,
}

#[derive(Debug)]
pub(crate) struct PackagePlan {
    pub name: String,
    /// Which corpus this plan installs into.
    pub lang: Lang,
    pub version: String,
    /// Recorded only; not currently consulted (kept for parity with the
    /// manifest, and for a future receipt/listing use).
    #[allow(dead_code)]
    pub description: Option<String>,
    /// Recorded only; never resolved through phase 3.
    #[allow(dead_code)]
    pub dependencies: BTreeMap<String, String>,
    pub files: Vec<PlannedFile>,
    /// True when this plan came from the no-manifest `packages/`-fallback
    /// rather than a real `rustyfi-package.toml`. Recorded only; not
    /// currently branched on.
    #[allow(dead_code)]
    pub from_fallback: bool,
}

/// Discover the install plan(s) under `source_root`, in precedence order:
/// `rustyfi-package.toml` manifest, else an upstream `Satyristes` build file
/// (phase 4), else the `packages/`-directory flat-copy fallback.
///
/// Returns a `Vec` because a single `Satyristes` may declare several
/// `(library ...)` blocks; the `toml` and fallback paths always yield exactly
/// one plan. A source carrying *both* a `rustyfi-package.toml` and a
/// `Satyristes` is rejected as ambiguous (the plan states no precedence).
pub(crate) fn discover(source_root: &Path) -> Result<Vec<PackagePlan>, Error> {
    let manifest_path = source_root.join(MANIFEST_NAME);
    let has_manifest = manifest_path.is_file();
    let has_satyristes = source_root.join(crate::satyristes::SATYRISTES_NAME).is_file();

    if has_manifest && has_satyristes {
        return Err(Error::AmbiguousSource {
            path: source_root.to_path_buf(),
        });
    }
    if has_manifest {
        let text = util::read_to_string(&manifest_path)?;
        let manifest: Manifest = toml::from_str(&text).map_err(|source| Error::Manifest {
            path: manifest_path.clone(),
            source,
        })?;
        Ok(vec![plan_from_manifest(source_root, manifest)?])
    } else if has_satyristes {
        crate::satyristes::read(source_root)
    } else if source_root.join("packages").is_dir() {
        Ok(vec![plan_from_fallback(source_root)?])
    } else {
        Err(Error::EmptySource {
            path: source_root.to_path_buf(),
        })
    }
}

pub(crate) fn plan_from_manifest(source_root: &Path, manifest: Manifest) -> Result<PackagePlan, Error> {
    let name = manifest.package.name;
    let lang = manifest.package.lang;
    let dist = lang.dist_dir();
    let mut files = Vec::new();
    for decl in &manifest.files {
        let src_abs = safe_source_join(source_root, &decl.src)?;
        match decl.kind {
            FileKind::PackageDir => {
                collect_dir(&src_abs, &format!("{dist}/packages/{name}"), &mut files)?
            }
            FileKind::FontDir => {
                collect_dir(&src_abs, &format!("{dist}/fonts/{name}"), &mut files)?
            }
            FileKind::Package => {
                let dst = require_dst(decl, "package")?;
                push_file(&src_abs, &format!("{dist}/packages/{name}/{dst}"), &mut files)?
            }
            FileKind::Font => {
                let dst = require_dst(decl, "font")?;
                push_file(&src_abs, &format!("{dist}/fonts/{name}/{dst}"), &mut files)?
            }
            FileKind::Md => {
                let dst = require_dst(decl, "md")?;
                push_file(&src_abs, &format!("{dist}/md/{name}/{dst}"), &mut files)?
            }
            FileKind::Doc => {
                let dst = require_dst(decl, "doc")?;
                push_file(&src_abs, &format!("{dist}/doc/{name}/{dst}"), &mut files)?
            }
            FileKind::Hash => {
                let dst = require_dst(decl, "hash")?;
                push_merge_file(&src_abs, &format!("{dist}/hash/{dst}"), &mut files)?
            }
            FileKind::File => {
                let dst = require_dst(decl, "file")?;
                push_file(&src_abs, &format!("{dist}/{dst}"), &mut files)?
            }
        }
    }
    Ok(PackagePlan {
        name,
        lang,
        version: manifest.package.version,
        description: manifest.package.description,
        dependencies: manifest.dependencies,
        files,
        from_fallback: false,
    })
}

fn plan_from_fallback(source_root: &Path) -> Result<PackagePlan, Error> {
    let name = source_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("package")
        .to_string();
    let packages = source_root.join("packages");
    let mut files = Vec::new();
    for p in util::read_dir_paths(&packages)? {
        let is_lib = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "satyh" || e == "satyg")
            .unwrap_or(false);
        if p.is_file() && is_lib {
            let file_name = p.file_name().unwrap().to_string_lossy();
            push_file(&p, &format!("dist/packages/{file_name}"), &mut files)?;
        }
    }
    files.sort_by(|a, b| a.dst.cmp(&b.dst));
    Ok(PackagePlan {
        name,
        // The no-manifest fallback has nothing to declare a generation with.
        lang: Lang::default(),
        version: "0.0.0".to_string(),
        description: None,
        dependencies: BTreeMap::new(),
        files,
        from_fallback: true,
    })
}

fn require_dst<'a>(decl: &'a FileDecl, kind: &'static str) -> Result<&'a str, Error> {
    decl.dst
        .as_deref()
        .ok_or(Error::MissingDst { kind })
}

fn push_file(src: &Path, dst: &str, out: &mut Vec<PlannedFile>) -> Result<(), Error> {
    push_one(src, dst, false, out)
}

/// As [`push_file`], for a destination several packages share.
fn push_merge_file(src: &Path, dst: &str, out: &mut Vec<PlannedFile>) -> Result<(), Error> {
    push_one(src, dst, true, out)
}

fn push_one(src: &Path, dst: &str, merge: bool, out: &mut Vec<PlannedFile>) -> Result<(), Error> {
    if !src.is_file() {
        return Err(Error::io(
            src,
            std::io::Error::new(std::io::ErrorKind::NotFound, "declared source file not found"),
        ));
    }
    out.push(PlannedFile {
        src: src.to_path_buf(),
        dst: dst.to_string(),
        merge,
    });
    Ok(())
}

/// Recursively collect every file under `dir` into `dst_prefix/<relpath>`.
fn collect_dir(dir: &Path, dst_prefix: &str, out: &mut Vec<PlannedFile>) -> Result<(), Error> {
    if !dir.is_dir() {
        return Err(Error::io(
            dir,
            std::io::Error::new(std::io::ErrorKind::NotFound, "declared source directory not found"),
        ));
    }
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((cur, rel)) = stack.pop() {
        for p in util::read_dir_paths(&cur)? {
            let name = util::file_name(&p);
            let child_rel = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if p.is_dir() {
                stack.push((p, child_rel));
            } else if p.is_file() {
                out.push(PlannedFile {
                    src: p,
                    dst: format!("{dst_prefix}/{child_rel}"),
                    merge: false,
                });
            }
        }
    }
    out.sort_by(|a, b| a.dst.cmp(&b.dst));
    Ok(())
}

/// Join `rel` onto `base`, rejecting absolute paths and `..` components (a
/// manifest must not reach outside its own source tree).
fn safe_source_join(base: &Path, rel: &str) -> Result<PathBuf, Error> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(Error::PathTraversal {
            entry: rel_path.to_path_buf(),
        });
    }
    for comp in rel_path.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err(Error::PathTraversal {
                entry: rel_path.to_path_buf(),
            });
        }
    }
    Ok(base.join(rel_path))
}
