use std::path::PathBuf;

use rustyfi_syntax::{ParseFileError, RustyfiVersion};

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
        requested: RustyfiVersion,
        supported: Vec<RustyfiVersion>,
    },

    /// `LoadMode::Envelopes` was requested for a version with no `use`
    /// headers (plan §1.2: the rejected combination). Checked before any file
    /// is read.
    #[error(
        "SATySFi {version} has no `use` headers; `Envelopes` mode requires 0.1 \
         (drop --deps, or pass --target-version 0.1)"
    )]
    InvalidModeVersion { version: RustyfiVersion },

    /// `use package Mod` with no deps config to resolve `Mod` against
    /// (upstream's used_as map is empty).
    #[error(
        "{from}: cannot resolve `use package {module}` — no pre-resolved \
         dependency graph; pass --deps <rustyfi-deps.yaml>"
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

    /// Ld3b-1: the `--deps` file (`rustyfi-deps.yaml`) could not be read.
    /// Upstream `DepsConfigNotFound`, `depsConfig.ml:40-45`. Nothing calls
    /// [`crate::v01x::deps::load`] yet (that wiring is Ld3b-2); this variant
    /// exists so the decoder's own unit tests can exercise the real error
    /// path.
    #[error("{path}: cannot read rustyfi-deps.yaml: {source}")]
    DepsConfigNotFound {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Ld3b-1: `rustyfi-deps.yaml` failed to decode or validate — either a
    /// YAML/shape error from `serde_yaml` or one of the two non-structural
    /// checks (`path` must be absolute; `used_as` must be an uppercased
    /// identifier). Upstream `DepsConfigError` wrapping `YamlError`
    /// (`depsConfig.ml:46-47`, `yamlDecoder.ml`); `message` carries a
    /// dotted-path context string in the same spirit as upstream's
    /// `show_yaml_context` (wording is this port's own, per Ld3b spec §3.4).
    #[error("{path}: invalid rustyfi-deps.yaml: {message}")]
    DepsConfigDecode { path: PathBuf, message: String },

    /// Ld3b-1: a `rustyfi-envelope.yaml` could not be read. Upstream
    /// `EnvelopeConfigNotFound`, `envelopeConfig.ml:142-147`.
    #[error("{path}: cannot read rustyfi-envelope.yaml: {source}")]
    EnvelopeConfigNotFound {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Ld3b-1: a `rustyfi-envelope.yaml` failed to decode or validate.
    /// Upstream `EnvelopeConfigError`. Covers the `library`/`font` branch
    /// check, `opentype_single`/`opentype_collection` branch check, relative
    /// `path`, lowercased font `name`, and the 18 `markdown_conversion`
    /// command/identifier shapes.
    #[error("{path}: invalid rustyfi-envelope.yaml: {message}")]
    EnvelopeConfigDecode { path: PathBuf, message: String },

    /// Ld3b-2: a `source_directories` entry of an envelope could not be
    /// listed. Upstream `CannotReadDirectory`, `envelopeReader.ml:15-16`.
    #[error("{path}: cannot list envelope source directory: {source}")]
    CannotReadDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Ld3b-2: two deps-config envelopes share a name. Upstream
    /// `EnvelopeNameConflict`, `closedEnvelopeDependencyResolver.ml:50-51`.
    #[error("rustyfi-deps.yaml: two envelopes are named `{name}`")]
    EnvelopeNameConflict { name: String },

    /// Ld3b-2: an envelope depends on a name absent from the deps config's
    /// envelope set. Upstream `DependencyOnUnknownEnvelope`,
    /// `closedEnvelopeDependencyResolver.ml:71-76`.
    #[error("envelope `{depending}` depends on unknown envelope `{depended}`")]
    DependencyOnUnknownEnvelope { depending: String, depended: String },

    /// Ld3b-2: a cycle in the envelope dependency graph. Upstream
    /// `CyclicEnvelopeDependency`; `chain` is names (not paths), the first
    /// element repeated last, matching [`Cycle`]'s shape.
    #[error("envelope dependency cycle: {}", .chain.join(" -> "))]
    CyclicEnvelopeDependency { chain: Vec<String> },

    /// Ld3b-2: two source files in one envelope declare the same module
    /// name. Upstream `FileModuleNameConflict`,
    /// `closedFileDependencyResolver.ml:20-22`.
    #[error(
        "envelope module `{module}` is declared by both {} and {}",
        .prev.display(),
        .path.display()
    )]
    FileModuleNameConflict {
        module: String,
        prev: PathBuf,
        path: PathBuf,
    },

    /// Ld3b-2: a bare `use M` inside an envelope names no sibling module.
    /// Upstream `FileModuleNotFound`, `closedFileDependencyResolver.ml:37-41`.
    #[error("{from}: `use {module}` names no module in this envelope")]
    FileModuleNotFound { module: String, from: PathBuf },

    /// Ld3b-2: a `use … of` header inside an envelope source tree. Upstream
    /// `CannotUseHeaderUseOf`, `closedFileDependencyResolver.ml:51-52`.
    #[error(
        "{from}: `use {module} of …` is not allowed inside a package; package \
         files address siblings by bare `use <Module>`"
    )]
    UseOfInsidePackage { module: String, from: PathBuf },

    /// Ld3b-2: a `use package M` header whose head matches no `used_as` alias
    /// in the supplied deps config. Upstream `UnknownPackageDependency`
    /// (typecheck-side there, load-side here — this port's loader is the only
    /// header consumer; see Ld3b spec §6 step 5).
    #[error(
        "{from}: `use package {module}` does not match any dependency alias \
         (`used_as`) in the supplied rustyfi-deps.yaml"
    )]
    UnknownPackageDependency { module: String, from: PathBuf },
}

fn format_versions(versions: &[RustyfiVersion]) -> String {
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
