//! Language Server Protocol support for SATySFi, and the protocol-free
//! single-file analysis underneath it.
//!
//! The crate is deliberately two halves with a hard line between them:
//!
//! - [`analyze`] and friends ([`Diag`], [`Severity`], [`LineIndex`],
//!   [`detect_version`]) — **no LSP types, no filesystem, no I/O**, and
//!   nothing outside `rustyfi-syntax`. This half builds for
//!   `wasm32-unknown-unknown` with `default-features = false`, so the
//!   browser playground's editor gets exactly the diagnostics the editor
//!   on the desktop does, out of the same code.
//! - [`server`] (feature `server`, on by default) — the stdio JSON-RPC loop
//!   `rustyfi lsp` runs. It owns every LSP-shaped structure in the crate.
//!
//! # Why the analysis stops at parsing
//!
//! [`analyze`] reports **lex and parse** diagnostics and no others. That is a
//! deliberate ceiling, not an unfinished edge:
//!
//! Elaboration and typechecking in this port run over a whole *program* — the
//! entry file's prelude spliced behind every `@require:`d package's, in
//! dependency order (`rustyfi_loader::load` → `rustyfi_lang::
//! compile_document_*`). A single file analysed on its own has none of that,
//! so every name a real document imports — `document`, `\emph`, `+p`,
//! `List.map` — is an unbound variable. Emitting those would bury the one
//! genuine error under a hundred false ones on precisely the documents that
//! compile fine, and "spurious diagnostics on a valid document are worse than
//! none". Parse errors, by contrast, are a property of the file's own text
//! and are honest without any context at all.
//!
//! Whole-program typechecking needs a resolved library root, a font store and
//! a build's worth of work per keystroke; it belongs behind a debounce and a
//! cache, and it is left for later rather than half-done here.

#![cfg_attr(docsrs, feature(doc_cfg))]

mod analysis;
mod high_water;
mod line_index;
mod version;

#[cfg(feature = "server")]
pub mod jsonrpc;
#[cfg(feature = "server")]
pub mod server;

pub use analysis::{analyze, analyze_auto, analyze_detected, Diag, Severity};
pub use line_index::{LineIndex, Position};
pub use version::detect_version;

/// Re-exported so a consumer of [`analyze`] does not have to depend on
/// `rustyfi-syntax` directly just to name its second argument.
pub use rustyfi_syntax::RustyfiVersion;
