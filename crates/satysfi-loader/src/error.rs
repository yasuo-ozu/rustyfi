use std::path::PathBuf;

use satysfi_syntax::{ParseFileError, SatysfiVersion};

/// Everything that can go wrong while loading a multi-file SATySFi program.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// Could not read a source file from disk.
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A file failed to parse (lex or grammar error). `source` carries the
    /// original [`ParseFileError`] (span + message) unchanged.
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseFileError,
    },

    /// `@require: name` could not be resolved to any file on disk.
    #[error(
        "cannot resolve `@require: {name}`; searched: {}",
        format_searched(.searched)
    )]
    UnresolvedRequire {
        name: String,
        searched: Vec<PathBuf>,
    },

    /// `@import: name` could not be resolved to any file on disk.
    #[error(
        "cannot resolve `@import: {name}` from {}; searched: {}",
        .from.display(),
        format_searched(.searched)
    )]
    UnresolvedImport {
        name: String,
        from: PathBuf,
        searched: Vec<PathBuf>,
    },

    /// The dependency graph contains a cycle; `chain` names the files
    /// involved, in traversal order, with the first file repeated at the end
    /// to make the loop explicit (e.g. `[a, b, a]`).
    #[error("dependency cycle detected: {}", format_chain(.chain))]
    Cycle { chain: Vec<PathBuf> },

    /// A file reached via `@require:`/`@import:` is a document (has a body)
    /// rather than a library.
    #[error("{path}: required/imported file must be a library (no `in ...` body), found a document")]
    DocumentAsDependency { path: PathBuf },

    /// The entry file is a library (no body) rather than a document.
    #[error("{path}: entry file must be a document (with an `in ...` body), found a library")]
    LibraryAsEntry { path: PathBuf },

    /// `opts.version` names a SATySFi version this port does not implement
    /// yet (checked before any file is even read).
    #[error(
        "SATySFi {requested} documents are not supported yet; supported: {}",
        format_versions(.supported)
    )]
    UnsupportedVersion {
        requested: SatysfiVersion,
        supported: Vec<SatysfiVersion>,
    },

    /// `LoadMode::Envelopes` was requested for a version with no `use`
    /// headers (plan §1.2: the rejected combination). Checked before any file
    /// is read.
    #[error(
        "SATySFi {version} has no `use` headers; `Envelopes` mode requires 0.1 \
         (drop --deps, or pass --target-version 0.1)"
    )]
    InvalidModeVersion { version: SatysfiVersion },

    /// Envelopes mode, Ld3a: a `satysfi-deps.yaml` was supplied but its
    /// consumption (envelope graph resolution) is not implemented yet.
    /// Follow-up: Ld3b.
    #[error(
        "{path}: satysfi-deps.yaml consumption is not implemented yet (Ld3b); \
         only `use … of` local dependencies are supported so far"
    )]
    DepsConfigUnsupported { path: PathBuf },

    /// `use package Mod` with no deps config to resolve `Mod` against
    /// (upstream's used_as map is empty). Follow-up: Ld3b.
    #[error(
        "{from}: cannot resolve `use package {module}` — no pre-resolved \
         dependency graph; pass --deps <satysfi-deps.yaml> (package resolution \
         lands in Ld3b)"
    )]
    PackageDependencyUnresolved { module: String, from: PathBuf },

    /// Bare `use Mod` at document/open level (upstream `CannotUseHeaderUse`):
    /// a document cannot reach into a package's internals by module name.
    /// Permanent (not a stub): Ld3b's closed resolver handles bare `use` only
    /// *inside* envelope source trees.
    #[error(
        "{from}: bare `use {module}` is only allowed between files inside one \
         package; a document must say `use package {module}` or `use {module} \
         of \"<path>\"`"
    )]
    BareUseOutsidePackage { module: String, from: PathBuf },

    /// `` use … of `relpath` `` matched no candidate file on disk.
    #[error(
        "cannot resolve `use … of `{relpath}`` from {}; searched: {}",
        .from.display(),
        format_searched(.searched)
    )]
    UnresolvedUseOf {
        relpath: String,
        from: PathBuf,
        searched: Vec<PathBuf>,
    },

    /// A `use`-family header under Legacy mode (the mode that resolves
    /// `@require:`/`@import:`). Names the fix.
    #[error(
        "{from}: `{header}` requires Envelopes mode (pass --deps, or let a \
         `use` header pin it); this load ran in Legacy (@require/@import) mode"
    )]
    EnvelopeHeaderUnderLegacy { header: String, from: PathBuf },

    /// A Legacy (`@require:`/`@import:`) header under Envelopes mode. Names
    /// the fix.
    #[error(
        "{from}: `{header}` is a Legacy (@require/@import) header; Envelopes \
         mode resolves `use package` / `use … of` headers only"
    )]
    LegacyHeaderUnderEnvelopes { header: String, from: PathBuf },
}

fn format_versions(versions: &[SatysfiVersion]) -> String {
    versions
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_searched(searched: &[PathBuf]) -> String {
    if searched.is_empty() {
        "(no candidates; is `lib_root` configured?)".to_string()
    } else {
        searched
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn format_chain(chain: &[PathBuf]) -> String {
    chain
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" -> ")
}
