//! Language Server Protocol support for SATySFi, and the protocol-free
//! single-file analysis underneath it.
//!
//! The crate is two halves with a hard line between them:
//!
//! - [`analyze`] and friends ([`analyze_auto`], [`analyze_detected`],
//!   [`Diag`], [`Severity`], [`LineIndex`]) — no LSP types, no filesystem, no
//!   I/O, and nothing outside `rustyfi-syntax`. This half builds for
//!   `wasm32-unknown-unknown`, so the browser playground's editor gets the
//!   same diagnostics the desktop one does out of the same code:
//!
//!   ```console
//!   $ cargo build -p rustyfi-lsp --target wasm32-unknown-unknown --no-default-features
//!   ```
//! - [`project`] (feature `typecheck`, on by default) — the *whole-program*
//!   tier: resolve the buffer's `@require:`/`@import:` graph off the disk and
//!   typecheck the resulting program. Needs a filesystem, so it is on the far
//!   side of the line and absent from the wasm build.
//! - [`server`] (feature `server`, on by default) — the stdio JSON-RPC loop
//!   `rustyfi lsp` runs. It owns every LSP-shaped structure in the crate.
//!
//! # Positions
//!
//! Every position this crate produces is an LSP position: **zero-based**
//! lines, characters in **UTF-16 code units**. Neither obvious shortcut
//! works — `rustyfi_syntax::Loc` is 1-based with a `char` column, and a byte
//! offset is wrong for the whole Japanese corpus. See [`LineIndex`].
//!
//! # Two tiers
//!
//! [`analyze`] reports **lex and parse** diagnostics and no others. This
//! port's elaboration and typechecking run over a whole *program* — the entry
//! file's prelude spliced behind every `@require:`d package's — so a single
//! file checked alone reports every imported name (`document`, `\emph`, `+p`,
//! `List.map`) as unbound. That would bury the one genuine error under a
//! hundred false ones on exactly the documents that compile. A parse error, by
//! contrast, is a property of the file's own text.
//!
//! [`project::check`] supplies the missing program instead of lowering the
//! standard: it resolves the buffer's real dependency graph off the disk (the
//! *buffer's* text standing in for its own file, unsaved edits included) and
//! typechecks that. Where the program cannot be resolved it degrades to the
//! parse tier and records why.

mod analysis;
mod high_water;
mod line_index;
mod symbols;

#[cfg(feature = "server")]
pub mod jsonrpc;
#[cfg(feature = "typecheck")]
pub mod project;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
mod workspace;

pub use analysis::{analyze, analyze_auto, analyze_detected, detect_version, Diag, Severity};
pub use line_index::{LineIndex, Position};
pub use symbols::{
    document_symbols, document_symbols_auto, document_symbols_detected, Range, Symbol, SymbolKind,
};

/// Re-exported so a consumer of [`analyze`] does not have to depend on
/// `rustyfi-syntax` directly just to name its second argument.
pub use rustyfi_syntax::RustyfiVersion;
