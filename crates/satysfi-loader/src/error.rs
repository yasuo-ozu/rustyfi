use std::path::PathBuf;

use satysfi_syntax::ParseFileError;

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
