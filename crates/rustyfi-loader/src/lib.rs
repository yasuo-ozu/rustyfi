//! Multi-file loading layer for SATySFi documents: resolves `@require:` /
//! `@import:` headers to files on disk, recursively parses the whole
//! dependency graph, and returns it in dependency-first (topological) load
//! order.
//!
//! Transcribed from v0.0.6's `src/frontend/main.ml` (lines ~95-140):
//!
//! - `@import: name` resolves relative to the directory of the file
//!   *containing* the header (not the entry document's directory).
//! - `@require: name` resolves against the package/library root
//!   (`LoadOptions::lib_root`).
//! - Candidate extensions, tried in order: `.satyh`, then `.satyg` (the
//!   mode-specific `.satyh-<mode>` extensions from `main.ml` are out of
//!   scope here).
//! - The same file reached through two different headers is one graph node
//!   (deduplicated by canonical path).
//! - Every dependency must be a library file (`body: None`); the entry must
//!   be a document (`body: Some(..)`).
//! - A cycle in the dependency graph is an error naming the files involved.

mod error;
mod graph;
mod v006;
mod v01x;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub use error::LoadError;
pub use rustyfi_syntax::RustyfiVersion;

/// How multi-file dependencies are declared and resolved — Axis B of
/// `docs/plans/rustyfi-0-1-0-support.md` §1.2. Orthogonal to
/// [`RustyfiVersion`] (Axis A, the grammar generation), except that the one
/// combination with no upstream analogue — `V0_0_6` + `Envelopes` — is
/// rejected by [`load`] up front (plan §1.3's table).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LoadMode {
    /// `@require:`/`@import:` header search against [`LoadOptions::lib_root`] —
    /// today's only mode, and `dev-0-1-0`'s *only* mode too (its headers are
    /// byte-identical to 0.0.6's, minus `@stage:`). The [`Default`], so every
    /// existing `LoadOptions { .., ..Default::default() }` call site is
    /// unchanged — mirroring exactly how `version` was added.
    #[default]
    Legacy,
    /// `use package` / `use … of` headers resolved the `saphe-split` way
    /// (upstream ≈ "0.1.0-alpha.1"): local files by relative path, packages
    /// from a pre-solved envelope graph. Requires `version == V0_1`.
    Envelopes {
        /// Path to a pre-resolved `rustyfi-deps.yaml` (upstream's mandatory
        /// `--deps` flag on `rustyfi build`, `saphe-split:bin/rustyfi.ml`,
        /// `flag_deps`). `None` = no package dependencies available: any `use
        /// package` header is a [`LoadError::PackageDependencyUnresolved`].
        /// `Some(path)` (Ld3b-2) is decoded, its envelopes are read + topo-
        /// sorted, and each `use package M` header is validated against the
        /// config's `used_as` aliases; the envelope source files are prepended
        /// to the loaded program (dependency-first, before any local file).
        deps: Option<PathBuf>,
    },
}

/// Options controlling header resolution.
///
/// Implements [`Default`] (rather than requiring every field to be named at
/// each construction site) precisely so that adding `version` here did not
/// force edits to every existing `LoadOptions { lib_root: ... }` call site —
/// they now read `LoadOptions { lib_root: ..., ..Default::default() }`.
#[derive(Default)]
pub struct LoadOptions {
    /// Root used to resolve `@require: name` (searched as
    /// `<lib_root>/dist/packages/name.{satyh,satyg}`, then
    /// `<lib_root>/name.{satyh,satyg}`, then the nested
    /// `<lib_root>/dist/packages/name/name.{satyh,satyg}` layout the
    /// Satyrographos installer produces — see `resolve::resolve_require`).
    /// `None` means there is no package root configured, so any `@require:`
    /// header fails to resolve.
    pub lib_root: Option<PathBuf>,
    /// The SATySFi language version the input is expected to conform to.
    /// Defaults to [`RustyfiVersion::DEFAULT`] (0.0.6, the only version this
    /// loader implements). [`load`] rejects any version for which
    /// [`RustyfiVersion::is_implemented`] is false before doing any work.
    pub version: RustyfiVersion,
    /// How dependencies are resolved. Defaults to [`LoadMode::Legacy`], so
    /// every pre-Ld3a call site (which either uses `..Default::default()` or
    /// names only `lib_root`/`version`) behaves identically. Ignored by the
    /// Envelopes backend (which resolves `use … of` relative paths and, in
    /// Ld3b, a `rustyfi-deps.yaml` envelope graph — never `lib_root`).
    pub mode: LoadMode,
}

/// A parsed file's CST, tagged by which grammar generation produced it.
/// `load()` picks the variant per `LoadOptions::version` (`V0_0_6` →
/// `V0_0_6`, `V0_1` → `V0_1`) — see `load`'s dispatch, below. Every file
/// in one `LoadedProgram` carries the same variant, since `opts.version` is
/// one value for the whole load; the enum exists so `LoadedFile` has a
/// single field type rather than forcing every consumer of `LoadedProgram`
/// to be generic over the CST type.
#[derive(Debug)]
pub enum LoadedCst {
    V0_0_6(rustyfi_syntax::cst::File),
    V0_1(rustyfi_syntax::cst_v1::FileV1),
}

impl LoadedCst {
    /// Whether this file is a document (has a body) rather than a library.
    /// Used by `load()`'s entry/dependency-shape validation
    /// (`DocumentAsDependency`/`LibraryAsEntry`) uniformly across both
    /// generations, so that validation logic itself needs no `match` at its
    /// call sites.
    pub fn is_document(&self) -> bool {
        match self {
            Self::V0_0_6(f) => f.body.is_some(),
            Self::V0_1(f) => matches!(f, rustyfi_syntax::cst_v1::FileV1::Document { .. }),
        }
    }

    /// This `V0_0_6` file's `@require:`/`@import:`/`@stage:` headers, or
    /// `None` for a `V0_1` file. Each generation's header list has a distinct
    /// element type since Ld3a (`V0_1` carries `HeaderV1`, the union grammar),
    /// so the shared facade offers one total accessor per generation rather
    /// than one `Header`-typed accessor for both.
    fn headers_v006(&self) -> Option<&[rustyfi_syntax::cst::Header]> {
        match self {
            Self::V0_0_6(f) => Some(&f.headers),
            Self::V0_1(_) => None,
        }
    }

    /// This `V0_1` file's headers (the `HeaderV1` union — Legacy `@`-headers
    /// plus the three `use` forms), or `None` for a `V0_0_6` file.
    fn headers_v1(&self) -> Option<&[rustyfi_syntax::cst_v1::HeaderV1]> {
        match self {
            Self::V0_0_6(_) => None,
            Self::V0_1(f) => Some(match f {
                rustyfi_syntax::cst_v1::FileV1::Document { headers, .. }
                | rustyfi_syntax::cst_v1::FileV1::Library { headers, .. } => headers,
            }),
        }
    }
}

/// Where a loaded file came from — metadata for diagnostics and for the
/// future `used_as` → module binding (Ld3c). Nothing in `rustyfi-lang` reads
/// it yet; it is additive metadata on [`LoadedFile`], so a 0.0.6 (or any
/// Legacy) load is byte-for-byte behavior-preserving.
///
/// Only two variants: a Legacy-mode file and an Envelopes-mode *local*
/// (`use … of`) file / the entry document are both just "a plain local file"
/// ([`FileOrigin::Local`], the [`Default`]); a distinct `Legacy` variant
/// would be a distinction without a consumer (revisit if Ld3c needs the
/// split). [`FileOrigin::Envelope`] tags a source file that came out of a
/// deps-config envelope (`rustyfi-envelope.yaml`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FileOrigin {
    /// A Legacy-mode file, an Envelopes-mode local (`use … of`) dependency,
    /// or the entry document. The [`Default`], so hand-built [`LoadedFile`]
    /// literals stay a one-line `origin: FileOrigin::default()`.
    #[default]
    Local,
    /// A source file of a deps-config envelope: `envelope` is the envelope's
    /// (deps-config) name, `module` the declared module name of this file.
    Envelope { envelope: String, module: String },
}

/// One parsed file in a loaded program.
#[derive(Debug)]
pub struct LoadedFile {
    /// Canonicalized path to the file on disk.
    pub path: PathBuf,
    /// The file's parsed concrete syntax tree, tagged by grammar
    /// generation. Was `cst: rustyfi_syntax::cst::File` before `V0_1`
    /// support; every consumer must now match on the variant. See
    /// `LoadedCst`'s doc comment.
    pub cst: LoadedCst,
    /// Where this file came from ([`FileOrigin::Local`] for Legacy files and
    /// Envelopes-mode locals; [`FileOrigin::Envelope`] for deps-config
    /// envelope sources). Additive metadata — no consumer reads it yet.
    pub origin: FileOrigin,
    /// The `RustyfiVersion` grammar this SPECIFIC file was parsed under —
    /// always matches `cst`'s variant (`V0_0_6` <-> `LoadedCst::V0_0_6`,
    /// `V0_1` <-> `LoadedCst::V0_1`). Cross-version import (X1,
    /// `docs/plans/design-cross-version-import.md`): under `LoadMode::
    /// Envelopes` and under a `LoadOptions { version: V0_0_6, .. }` Legacy
    /// load, every file in one `LoadedProgram` shares one version (the
    /// load's `opts.version`), exactly as before this field existed. Only a
    /// `LoadOptions { version: V0_1, mode: Legacy, .. }` load can produce a
    /// MIXED-version `files` list: `load_legacy`'s worklist (see its doc
    /// comment) per-file-detects a `V0_0_6` dependency via the Q4 rule
    /// (`design-cross-version-import.md` §5) so a `V0_1` document can
    /// `@require:` a frozen `V0_0_6` package. Additive — every pre-X1
    /// `LoadedFile { .. }` call site now must also name this field, but no
    /// EXISTING (single-version) load's `cst`/`origin`/`path` values change.
    pub version: RustyfiVersion,
}

/// A fully loaded, dependency-resolved program.
#[derive(Debug)]
pub struct LoadedProgram {
    /// Dependency-first order: every file appears after all the files it
    /// depends on. Under Legacy mode that is the `@require:`/`@import:`
    /// order; under Envelopes mode the ordering contract is: all deps-config
    /// envelope sources first (dependency-first among themselves, each
    /// envelope's modules closed-sorted), then the local `use … of` files
    /// (dependency-first), then the entry document last.
    pub files: Vec<LoadedFile>,
}

/// Load `entry` (a `.saty` document) and its full transitive dependency
/// graph, dispatching on [`LoadOptions::mode`] (Axis B). [`LoadMode::Legacy`]
/// resolves `@require:`/`@import:` headers ([`load_legacy`]);
/// [`LoadMode::Envelopes`] resolves `use package`/`use … of` headers
/// (`v01x::open_doc`).
pub fn load(entry: &Path, opts: &LoadOptions) -> Result<LoadedProgram, LoadError> {
    if !opts.version.is_implemented() {
        return Err(LoadError::UnsupportedVersion {
            requested: opts.version,
            supported: RustyfiVersion::supported().to_vec(),
        });
    }

    match &opts.mode {
        LoadMode::Legacy => load_legacy(entry, opts),
        LoadMode::Envelopes { deps } => {
            // The one combination with no upstream analogue (plan §1.2, §1.3
            // row 4): 0.0.6 has no `use` headers to resolve against an
            // envelope graph at all. Reject before touching the filesystem,
            // like the version guard above. `!matches!(.., V0_1)` rather than
            // `== V0_0_6`: `RustyfiVersion` is `#[non_exhaustive]`, so any
            // hypothetical future third variant defaults to *rejected* under
            // Envelopes until someone decides otherwise.
            if !matches!(opts.version, RustyfiVersion::V0_1) {
                return Err(LoadError::InvalidModeVersion {
                    version: opts.version,
                });
            }
            v01x::open_doc::load(entry, deps.as_deref(), opts)
        }
    }
}

/// Resolve one Legacy (`@require:`/`@import:`/`@stage:`) header to a file
/// path, or `None` for `@stage:` (which drives no dependency edge). Shared by
/// the `V0_0_6` and `V0_1`-Legacy header loops in [`load_legacy`].
fn resolve_legacy_header(
    header: &rustyfi_syntax::cst::Header,
    dir: &Path,
    from: &Path,
    opts: &LoadOptions,
) -> Result<Option<PathBuf>, LoadError> {
    Ok(Some(match header {
        rustyfi_syntax::cst::Header::Import(tok) => {
            v006::resolve::resolve_import(dir, &tok.content).map_err(|searched| {
                LoadError::UnresolvedImport {
                    name: tok.content.clone(),
                    from: from.to_path_buf(),
                    searched,
                }
            })?
        }
        rustyfi_syntax::cst::Header::Require(tok) => {
            v006::resolve::resolve_require(opts.lib_root.as_deref(), &tok.content).map_err(
                |searched| LoadError::UnresolvedRequire {
                    name: tok.content.clone(),
                    searched,
                },
            )?
        }
        // `@stage: persistent` / `@stage: 0` / `@stage: 1` — this port is
        // single-stage only, so the header carries no loader-visible
        // information (see `cst::Header::Stage`'s doc comment); it drives no
        // dependency edge.
        rustyfi_syntax::cst::Header::Stage(_) => return Ok(None),
    }))
}

/// The `LoadMode::Legacy` backend: `@require:`/`@import:` header resolution
/// against `lib_root`, recursive parse, and dependency-first ordering. The
/// body is the pre-Ld3a `load()` verbatim (Ld1's shared worklist/validation
/// shell around the `v006::` calls), with the header loop now dispatching per
/// grammar generation so a `V0_1`-under-Legacy file with a `use` header gets
/// a typed [`LoadError::EnvelopeHeaderUnderLegacy`] (a better diagnostic than
/// the old parse error) — the only Legacy-visible behavior change in Ld3a.
fn load_legacy(entry: &Path, opts: &LoadOptions) -> Result<LoadedProgram, LoadError> {
    let entry_canon = canonicalize(entry)?;

    let mut next_id: u32 = 0;
    let mut id_of: HashMap<PathBuf, u32> = HashMap::new();
    let mut path_of: HashMap<u32, PathBuf> = HashMap::new();
    let mut cst_of: HashMap<u32, LoadedCst> = HashMap::new();
    // X1 (design-cross-version-import.md §5, Q4): the per-file version each
    // graph node was actually parsed under — see `LoadedFile::version`'s doc
    // comment. Populated in lockstep with `cst_of` below; `V0_0_6` loads
    // insert `V0_0_6` for every node (unchanged behavior).
    let mut version_of: HashMap<u32, RustyfiVersion> = HashMap::new();
    // X1 Q4: node ids reached via at least one `@require:` header edge (as
    // opposed to only `@import:` edges) — the "resolves under `lib_root`'s
    // package tree" half of the per-file detection rule. Populated as
    // dependency edges are discovered, below; irrelevant (never consulted)
    // for a `V0_0_6` load.
    let mut require_targets: HashSet<u32> = HashSet::new();
    // X4a Q4-mirror (design-cross-version-import.md §X4.3 item 2): node ids
    // reached via at least one `@require:` edge that resolved PHYSICALLY
    // under `dist-v01/packages/` — the "resolves under the 0.1 corpus" half
    // of the mirrored per-file detection rule. Populated in lockstep with
    // `require_targets`, below; irrelevant (never consulted) for a `V0_1`
    // load (that load uses `require_targets`/`is_dist_packages_target`
    // instead).
    let mut require_v01_targets: HashSet<u32> = HashSet::new();
    let mut adjacency: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut processed: HashSet<u32> = HashSet::new();

    let entry_id = alloc_id(entry_canon, &mut next_id, &mut id_of, &mut path_of);

    let mut worklist = vec![entry_id];
    while let Some(id) = worklist.pop() {
        if processed.contains(&id) {
            continue;
        }
        processed.insert(id);

        let path = path_of[&id].clone();
        let src = std::fs::read_to_string(&path).map_err(|source| LoadError::Io {
            path: path.clone(),
            source,
        })?;
        // X1 Q4 (design-cross-version-import.md §5, "decision of record for
        // X1"): under a `V0_1` load, every NON-entry file gets its own
        // per-file version — `sniff_version` first (a `use`/`val`-shaped
        // file sniffs `Some(V0_1)` even inside the frozen corpus), else
        // `V0_0_6` if this id was reached via at least one `@require:` edge
        // (the corpus IS `dist/packages/`, §4), else `opts.version`
        // (`@import:`-relative siblings of the entry, and the entry itself,
        // stay `V0_1`). A `V0_0_6` load is untouched: `file_version` is
        // always `opts.version` there, exactly the old unconditional match.
        let file_version = match opts.version {
            RustyfiVersion::V0_1 if id != entry_id => rustyfi_syntax::sniff_version(&src)
                .unwrap_or(
                    // A non-sniffable `@require:` target defaults to V0_0_6
                    // ONLY when it is physically under the frozen 0.0.6 corpus
                    // `dist/packages/` (§4). This must EXCLUDE `dist-v01/
                    // packages/` — those are V0_1 packages and the substring
                    // `/dist/packages/` does not match `/dist-v01/packages/`.
                    // Everything else (dist-v01 requires, @import: siblings)
                    // stays `opts.version` = V0_1.
                    if require_targets.contains(&id)
                        && path.to_string_lossy().contains("/dist/packages/")
                    {
                        RustyfiVersion::V0_0_6
                    } else {
                        RustyfiVersion::V0_1
                    },
                ),
            // X4a Q4-mirror (design-cross-version-import.md §X4.3 item 2): a
            // `V0_0_6`-rooted load's NON-entry file defaults to `opts.version`
            // (`V0_0_6`) unless `sniff_version` returns `Some(V0_1)`, in which
            // case it MUST default to `V0_1` when this id was reached via at
            // least one `@require:` edge that resolved physically under the
            // 0.1 corpus `dist-v01/packages/` (the mirror of `require_targets`
            // + `is_dist_packages_target` above) — a `module … :> sig …`-headed
            // 0.1 package (e.g. `v01-sealed.satyh`) sniffs `None` just like a
            // 0.0.6 `module`-headed corpus file does (`version.rs`'s own doc
            // comment: a bare `module` head is deliberately no signal), so
            // this provenance fallback is what actually resolves it. This is a
            // PURE WIDENING: nothing that used to resolve `V0_0_6` can flip to
            // `V0_1` unless it is BOTH under `dist-v01/packages/` AND reached
            // via `@require:` — `require_v01_targets` is empty for every
            // existing 0.0.6 fixture (none of them ever resolves a
            // `dist-v01/packages/` target), so this arm's `unwrap_or` falls
            // through to `V0_0_6` exactly as before for every pre-X4a load.
            RustyfiVersion::V0_0_6 if id != entry_id => rustyfi_syntax::sniff_version(&src)
                .unwrap_or(if require_v01_targets.contains(&id) {
                    RustyfiVersion::V0_1
                } else {
                    RustyfiVersion::V0_0_6
                }),
            other => other,
        };
        let cst: LoadedCst = match file_version {
            RustyfiVersion::V0_0_6 => LoadedCst::V0_0_6(
                rustyfi_syntax::parse_file(&src).map_err(|source| LoadError::Parse {
                    path: path.clone(),
                    source,
                })?,
            ),
            RustyfiVersion::V0_1 => LoadedCst::V0_1(
                rustyfi_syntax::parse_file_v1(&src).map_err(|source| LoadError::Parse {
                    path: path.clone(),
                    source,
                })?,
            ),
            // `RustyfiVersion` is `#[non_exhaustive]` — a catch-all is
            // required even though `load`'s `is_implemented()` guard above
            // already rejects every version this crate doesn't handle
            // before the loop starts. Unreachable in practice; a clear
            // message rather than a silent wrong-parse if `is_implemented()`
            // and this match ever drift apart.
            other => unreachable!(
                "RustyfiVersion::is_implemented() admitted {other} but load()'s \
                 parse dispatch has no arm for it"
            ),
        };
        version_of.insert(id, file_version);

        if id == entry_id {
            if !cst.is_document() {
                return Err(LoadError::LibraryAsEntry { path });
            }
        } else if cst.is_document() {
            return Err(LoadError::DocumentAsDependency { path });
        }

        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        // Collect this file's resolved dependency paths (per grammar
        // generation), then allocate ids for them uniformly below — so the
        // id/worklist bookkeeping is written exactly once. The `bool` is
        // whether the header that resolved this path was `@require:` (X1 Q4:
        // feeds `require_targets`, below) as opposed to `@import:`.
        let mut resolved_deps: Vec<(PathBuf, bool)> = Vec::new();
        if let Some(headers) = cst.headers_v006() {
            for header in headers {
                let is_require = matches!(header, rustyfi_syntax::cst::Header::Require(_));
                if let Some(resolved) = resolve_legacy_header(header, &dir, &path, opts)? {
                    resolved_deps.push((resolved, is_require));
                }
            }
        } else if let Some(headers) = cst.headers_v1() {
            use rustyfi_syntax::cst_v1::HeaderV1;
            for header in headers {
                match header {
                    // dev-0-1-0 semantics under Legacy: an `@`-header on a 0.1
                    // file resolves exactly like a 0.0.6 one (unchanged from
                    // Ld2).
                    HeaderV1::Legacy(h) => {
                        let is_require = matches!(h, rustyfi_syntax::cst::Header::Require(_));
                        if let Some(resolved) = resolve_legacy_header(h, &dir, &path, opts)? {
                            resolved_deps.push((resolved, is_require));
                        }
                    }
                    // A `use`-family header under Legacy mode: previously a
                    // *parse* error (no `use` grammar existed); now a typed
                    // *mode* error naming the fix. This is the single
                    // Legacy-path behavior change in all of Ld3a — a
                    // previously-failing input fails better; no
                    // previously-succeeding input changes.
                    HeaderV1::UsePackage { .. }
                    | HeaderV1::UseOf { .. }
                    | HeaderV1::Use { .. } => {
                        return Err(LoadError::EnvelopeHeaderUnderLegacy {
                            header: header.display_name(),
                            from: path.clone(),
                        });
                    }
                }
            }
        }

        let mut deps = Vec::new();
        for (resolved, is_require) in resolved_deps {
            let dep_canon = canonicalize(&resolved)?;
            // X1 Q4 (design-cross-version-import.md §5): "a `@require:`-
            // resolved target … that RESOLVES UNDER `lib-rustyfi/dist/
            // packages/`" — the FROZEN 0.0.6 corpus path specifically, NOT
            // every `@require:` edge. This is the load-bearing narrowing: a
            // `V0_1` package `@require:`d out of `dist-v01/packages/` (the
            // 0.1 corpus — reached via `resolve_require`'s `lib_root/name`
            // fallback, so its canonical path is NOT under a `dist/packages`
            // segment) must stay `V0_1`, or it would be mis-parsed with the
            // 0.0.6 grammar. Only a target physically under a `dist/packages`
            // directory is the frozen 0.0.6 corpus and eligible for the Q4
            // provenance downgrade (a genuinely-0.1 package dropped there
            // still wins via its own `Some(V0_1)` sniff, per the rule).
            let is_corpus_target = is_require && is_dist_packages_target(&dep_canon);
            // X4a Q4-mirror: the same narrowing, for the 0.1 corpus —
            // `is_require && is_dist_v01_packages_target(&dep_canon)`.
            // Deliberately checked independently of `is_corpus_target`
            // (`dist` and `dist-v01` never both match the same path), so a
            // `@require:` edge lands in at most one of `require_targets`/
            // `require_v01_targets`.
            let is_v01_corpus_target = is_require && is_dist_v01_packages_target(&dep_canon);
            let dep_id = alloc_id(dep_canon, &mut next_id, &mut id_of, &mut path_of);
            if is_corpus_target {
                require_targets.insert(dep_id);
            }
            if is_v01_corpus_target {
                require_v01_targets.insert(dep_id);
            }
            deps.push(dep_id);
            worklist.push(dep_id);
        }

        adjacency.insert(id, deps);
        cst_of.insert(id, cst);
    }

    let order = graph::toposort(&adjacency).map_err(|chain_ids| LoadError::Cycle {
        chain: graph::chain_to_paths(&chain_ids, &path_of),
    })?;

    let files = order
        .into_iter()
        .map(|id| LoadedFile {
            path: path_of[&id].clone(),
            cst: cst_of
                .remove(&id)
                .expect("every graph node id was parsed before toposort"),
            // Legacy-mode files are all plain local files.
            origin: FileOrigin::Local,
            version: version_of
                .remove(&id)
                .expect("every graph node id was version-tagged before toposort"),
        })
        .collect();

    Ok(LoadedProgram { files })
}

pub(crate) fn canonicalize(path: &Path) -> Result<PathBuf, LoadError> {
    std::fs::canonicalize(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Whether `path` lives under a `dist/packages/` directory — the frozen
/// 0.0.6 corpus layout (`docs/plans/design-cross-version-import.md` §4/Q4),
/// the `@require:`-provenance signal the X1 per-file version detector uses to
/// downgrade a sniff-`None` corpus dependency to `V0_0_6`. Matches ANY two
/// consecutive components `dist` then `packages` anywhere in the path, so it
/// recognizes both this port's own `lib-rustyfi/dist/packages/` and a
/// Satyrographos-style `<root>/dist/packages/` install — but deliberately
/// NOT the 0.1 corpus `dist-v01/packages/` (`dist-v01` != `dist`), whose
/// `V0_1` packages must keep the load's `opts.version`.
fn is_dist_packages_target(path: &Path) -> bool {
    let comps: Vec<_> = path.components().collect();
    comps.windows(2).any(|w| {
        w[0].as_os_str() == "dist" && w[1].as_os_str() == "packages"
    })
}

/// Whether `path` lives under a `dist-v01/packages/` directory — the 0.1
/// corpus layout (Slice X4a, `docs/plans/design-cross-version-import.md`
/// §X4.3 item 2), the MIRROR of [`is_dist_packages_target`] used by the
/// symmetric per-file version detector to default a sniff-`None` 0.1-corpus
/// dependency (e.g. a `module … :> sig …`-headed package like
/// `v01-sealed.satyh`) to `V0_1` under a `V0_0_6`-rooted load. Matches ANY
/// two consecutive components `dist-v01` then `packages` — deliberately NOT
/// `dist` then `packages` (the inverse of `is_dist_packages_target`'s own
/// care to exclude `dist-v01`), so the two helpers are mutually exclusive on
/// every real path.
fn is_dist_v01_packages_target(path: &Path) -> bool {
    let comps: Vec<_> = path.components().collect();
    comps.windows(2).any(|w| {
        w[0].as_os_str() == "dist-v01" && w[1].as_os_str() == "packages"
    })
}

pub(crate) fn alloc_id(
    path: PathBuf,
    next_id: &mut u32,
    id_of: &mut HashMap<PathBuf, u32>,
    path_of: &mut HashMap<u32, PathBuf>,
) -> u32 {
    if let Some(&id) = id_of.get(&path) {
        return id;
    }
    let id = *next_id;
    *next_id += 1;
    id_of.insert(path.clone(), id);
    path_of.insert(id, path);
    id
}
