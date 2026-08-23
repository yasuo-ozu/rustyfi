//! Language Server Protocol support for SATySFi, and the protocol-free
//! single-file analysis underneath it.
//!
//! The crate is deliberately two halves with a hard line between them:
//!
//! - [`analyze`] and friends ([`analyze_auto`], [`analyze_detected`],
//!   [`Diag`], [`Severity`], [`LineIndex`]), and the cursor-driven half
//!   ([`build_model`], [`hover`], [`definition`], [`completions`]) —
//!   **no LSP types, no filesystem, no I/O**, and nothing outside
//!   `rustyfi-syntax`.
//!   This half builds for `wasm32-unknown-unknown`, so the browser
//!   playground's editor gets exactly the diagnostics the editor on the
//!   desktop does, out of the same code:
//!
//!   ```console
//!   $ cargo build -p rustyfi-lsp --target wasm32-unknown-unknown --no-default-features
//!   ```
//!
//!   (The default features build for wasm too — `serde_json` is portable —
//!   but `--no-default-features` leaves `rustyfi-syntax` as the only
//!   dependency in the graph.)
//! - [`server`] (feature `server`, on by default) — the stdio JSON-RPC loop
//!   `rustyfi lsp` runs. It owns every LSP-shaped structure in the crate.
//!
//! # Positions
//!
//! Every position this crate produces is an LSP position: **zero-based**
//! lines, and characters counted in **UTF-16 code units**. Neither of the
//! obvious shortcuts works — `rustyfi_syntax::Loc` is 1-based with a `char`
//! column, and a byte offset is wrong for the whole Japanese corpus. See
//! [`LineIndex`].
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
//!
//! # What the cursor-driven half does instead
//!
//! [`hover`], [`definition`] and [`completions`] answer from a [`Model`] — a
//! **cursor → syntax** mapping over the one buffer, built once by
//! [`build_model`] and shared by all three. The same ceiling applies for the
//! same reason, and it shows up as *silence rather than invention*: a name
//! this file does not bind gets a hover saying what kind of name it is and
//! that it comes from elsewhere, no jump at all, and no place in any
//! completion list. Where a type appears in a hover it is the type the
//! **author wrote**, quoted from the buffer — an ascription, a `sig`'s
//! `val`, a synonym's right-hand side — never one this crate inferred.
//!
//! Half-typed buffers are the normal case for all three, not an edge: see
//! [`build_model`] for how a file that does not parse — or does not even lex —
//! still yields everything written before the break.

mod analysis;
mod features;
mod high_water;
mod line_index;
mod model;
mod walk006;
mod walk01;

#[cfg(feature = "server")]
pub mod jsonrpc;
#[cfg(feature = "server")]
pub mod server;

pub use analysis::{analyze, analyze_auto, analyze_detected, Diag, Severity};
pub use features::{completions, definition, hover, Completion, Definition, Hover};
pub use line_index::{LineIndex, Position};
pub use model::{
    build_model, ByteRange, Def, HeaderKind, HeaderRef, Hit, Model, Ns, Opaque, Ref,
};

/// Re-exported so a consumer of [`analyze`] does not have to depend on
/// `rustyfi-syntax` directly just to name its second argument.
pub use rustyfi_syntax::RustyfiVersion;
