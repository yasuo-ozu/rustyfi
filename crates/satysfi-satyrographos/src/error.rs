//! Error type for the package manager, styled after
//! [`satysfi_loader::LoadError`]: a `thiserror` enum with one specific
//! variant per failure mode, each carrying enough context to reconstruct
//! what went wrong (which path, which package). The CLI layer
//! (`satysfi-cli`) maps these variants to the exit codes in the plan's §4
//! surface spec (`0` ok, `2` filter, `3` root, `4` receipt, `5` fs/io) —
//! the exit-code policy stays out of this clap-free crate.

use std::path::PathBuf;

/// Everything that can go wrong while installing, uninstalling, or
/// inspecting packages.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A filesystem read/write/rename failed. `path` is the path being
    /// operated on when the underlying `io::Error` surfaced.
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Neither `--lib-root`/`--dest` was given nor `$SATYSFI_LIB_ROOT` set,
    /// so there is no root to install into or read from (exit `3`).
    #[error(
        "could not resolve a library root: pass `--lib-root DIR` or `--dest DIR`, \
         or set $SATYSFI_LIB_ROOT"
    )]
    RootResolution,

    /// A `satysfi-package.toml` manifest failed to parse.
    #[error("{path}: invalid satysfi-package.toml: {source}")]
    Manifest {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    /// A receipt file failed to parse.
    #[error("{path}: invalid receipt: {source}")]
    Receipt {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    /// A receipt for `name` already exists and `--force` was not given
    /// (exit `4`).
    #[error("package `{name}` is already installed (a receipt exists); pass `--force` to overwrite")]
    AlreadyInstalled { name: String },

    /// `uninstall`/`status NAME` was asked for a package with no receipt
    /// (exit `4`).
    #[error("package `{name}` is not installed (no receipt at {})", .receipt.display())]
    NotInstalled { name: String, receipt: PathBuf },

    /// A destination path already exists on disk but is not covered by any
    /// receipt — refusing to clobber files this tool did not place (exit
    /// `5`; mirrors upstream's managed-directory refusal, plan §2/§6).
    #[error(
        "destination `{}` already exists and is not managed by satyrographos; \
         remove it first (no receipt claims it)",
        .path.display()
    )]
    UnmanagedCollision { path: PathBuf },

    /// An archive entry's path escaped the extraction/staging root
    /// ("zip-slip"); refused before anything was written (exit `5`, plan
    /// §6/§10).
    #[error("unsafe path in archive escapes the extraction root: {}", .entry.display())]
    PathTraversal { entry: PathBuf },

    /// The install source is neither a directory nor a recognised
    /// `.tar.gz`/`.tgz` archive.
    #[error("install source `{}` is neither a directory nor a .tar.gz/.tgz archive", .path.display())]
    UnknownSource { path: PathBuf },

    /// The source has no `satysfi-package.toml` and no `packages/`
    /// subdirectory to fall back on — nothing installable.
    #[error(
        "no installable package found under `{}`: no satysfi-package.toml manifest \
         and no packages/ directory",
        .path.display()
    )]
    EmptySource { path: PathBuf },

    /// A generic archive (tar/gzip) decoding failure.
    #[error("archive error: {0}")]
    Archive(String),

    /// A `[[files]]` declaration of a non-`*-dir` kind omitted its required
    /// `dst` field (plan §5.1).
    #[error("[[files]] declaration of kind `{kind}` requires a `dst` field")]
    MissingDst { kind: &'static str },

    /// A `-l`/`--library NAME` filter was given but the source's declared
    /// package name is not in the requested set (exit `2`, plan §4.1).
    #[error("source declares package(s) `{declared}`, none of which is in the requested --library set")]
    LibraryFilter { declared: String },

    /// A `Satyristes` file (phase 4, plan §5.5) failed to tokenize/read or
    /// carried a form this port does not understand. `message` already
    /// includes a `line:col:` prefix for reader-level failures.
    #[error("{path}: invalid Satyristes: {message}")]
    Satyristes { path: PathBuf, message: String },

    /// An install source carries *both* a `satysfi-package.toml` and a
    /// `Satyristes` file. The plan (§5.5) names no precedence between them,
    /// so this port refuses the ambiguity rather than silently picking one.
    #[error(
        "source `{}` has both a satysfi-package.toml and a Satyristes file; \
         remove one (the plan states no precedence between them)",
        .path.display()
    )]
    AmbiguousSource { path: PathBuf },

    /// A `Satyristes` declares several `(library ...)` blocks and the
    /// `-l`/`--library` filter did not narrow the selection to exactly one
    /// (plan §4.1: one library materialised per install).
    #[error(
        "Satyristes declares multiple libraries (`{names}`); \
         select exactly one with -l/--library NAME"
    )]
    AmbiguousLibrary { names: String },

    /// A `Satyrfile.toml` project manifest failed to parse (phase 2, §5.3).
    #[error("{path}: invalid Satyrfile.toml: {source}")]
    Satyrfile {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    /// A `Satyrfile.lock` lockfile failed to parse (phase 2, §5.3).
    #[error("{path}: invalid Satyrfile.lock: {source}")]
    Lockfile {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    /// Manifest-mode `install` (no PATH) found no `Satyrfile.toml` at or above
    /// the search root (exit `3` — nothing to operate on).
    #[error("no Satyrfile.toml found in this directory or any parent")]
    SatyrfileNotFound,

    /// A `Satyrfile.toml` entry uses a `source` kind not materialisable in
    /// this phase (only `{{ path = … }}` and `{{ registry = … }}` are — `git`
    /// stays rejected, plan §5.4).
    #[error("unsupported source kind in Satyrfile.toml: {kind}")]
    UnsupportedSource { kind: &'static str },

    // --- Phase 3: registry (plan §5.4) --------------------------------------
    /// No registry URL could be resolved from `--registry`, `$SATYSFI_REGISTRY`,
    /// or a `Satyrfile.toml` `[registry]` url (exit `3` — nothing to consult).
    #[error(
        "no registry configured: pass `--registry URL`, set $SATYSFI_REGISTRY, \
         or add a [registry] url to Satyrfile.toml"
    )]
    NoRegistry,

    /// Acquiring/cloning the registry index via `git` failed (exit `5`).
    #[error("registry git operation failed: {message}")]
    GitFailed { message: String },

    /// The acquired index is malformed — a `packages/<name>.toml` failed to
    /// parse, or the clone has no `packages/` directory (exit `5`).
    #[error("invalid registry index: {message}")]
    RegistryIndex { message: String },

    /// No `packages/<name>.toml` in the index for the requested package
    /// (exit `4`).
    #[error("package `{name}` not found in the registry index")]
    PackageNotFound { name: String },

    /// The index has the package but not the requested version (exit `4`).
    #[error("package `{name}` has no version `{version}` in the registry index")]
    VersionNotFound { name: String, version: String },

    /// A downloaded tarball's SHA-256 did not match the index entry; nothing
    /// under `dist/` was touched (exit `5`, plan §5.4 step 3).
    #[error("checksum mismatch: expected sha256 {expected}, got {actual} (nothing installed)")]
    ChecksumMismatch { expected: String, actual: String },

    /// An `http(s)://` tarball was requested but the crate was built without
    /// the `http` feature (exit `5`).
    #[error(
        "cannot fetch `{url}`: this binary was built without the `http` feature \
         (path/archive/file:// installs are offline; rebuild with --features http)"
    )]
    HttpDisabled { url: String },

    /// An `http(s)://` fetch failed (exit `5`).
    #[error("failed to fetch `{url}`: {message}")]
    HttpFailed { url: String, message: String },
}

impl Error {
    /// Convenience for wrapping an [`std::io::Error`] with the path that
    /// caused it.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
