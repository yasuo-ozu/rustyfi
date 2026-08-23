//! Language Server Protocol support for SATySFi, and the protocol-free
//! single-file analysis underneath it.
//!
//! The crate is deliberately two halves with a hard line between them:
//!
//! - [`analyze`] and friends ([`analyze_auto`], [`analyze_detected`],
//!   [`Diag`], [`Severity`], [`LineIndex`]), the cursor-driven half
//!   ([`build_model`], [`hover`], [`definition`], [`completions`]) and the
//!   outline ([`document_symbols`]) — **no LSP types, no filesystem, no
//!   I/O**, and nothing outside `rustyfi-syntax`.
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
//! - [`project`] (feature `typecheck`, on by default) — the *whole-program*
//!   tier: resolve the buffer's `@require:`/`@import:` graph off the disk and
//!   typecheck the resulting program. Needs a filesystem, so it is deliberately
//!   on the far side of the line from [`analyze`] and absent from the wasm
//!   build.
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
//! # Two tiers, and why the line is where it is
//!
//! [`analyze`] reports **lex and parse** diagnostics and no others. That is a
//! deliberate ceiling for a *detached buffer*, not an unfinished edge:
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
//! [`project::check`] is the answer to that, and it answers it by supplying
//! the missing program rather than by lowering the standard: it resolves the
//! buffer's real dependency graph off the disk (with the *buffer's* text
//! standing in for its own file, unsaved edits included) and typechecks that.
//! Where the program cannot be resolved — no library root, an uninstalled
//! package, a `use`-header document whose packaging mode has no seam for an
//! in-memory buffer — it degrades to exactly the parse tier above and records
//! why. A file's own text is analysable anywhere; its program is not, and the
//! two tiers say so separately.
//!
//! The cost is real and is the reason the tiers are separate at all: a parse
//! is sub-millisecond, whereas resolving and typechecking a document with a
//! full document class behind it is tens to hundreds of milliseconds
//! (measured in `tests/project.rs`). Both run per `didOpen`/`didChange`
//! today, which is honest for documents of the size this corpus holds; a
//! debounce and a dependency-graph cache are the next things to add, not a
//! reason to withhold the tier.
//!
//! # What the cursor-driven half does
//!
//! [`hover`], [`definition`] and [`completions`] answer from a [`Model`] — a
//! **cursor → syntax** mapping over the one buffer, built once by
//! [`build_model`] and shared by all three. They stay on the single-file side
//! of the line: [`project`]'s resolved program is not consulted, so
//! [`analyze`]'s ceiling applies to them too, and it shows up as *silence
//! rather than invention*: a name this file does not bind gets a hover saying
//! what kind of name it is and that it comes from elsewhere, no jump at all,
//! and no place in any completion list. Where a type appears in a hover it is
//! the type the **author wrote**, quoted from the buffer — an ascription, a
//! `sig`'s `val`, a synonym's right-hand side — never one this crate inferred.
//!
//! Half-typed buffers are the normal case for all three, not an edge: see
//! [`build_model`] for how a file that does not parse — or does not even lex —
//! still yields everything written before the break.
//!
//! [`document_symbols`] is a fourth reader of the same buffer, with the same
//! ceiling, but its own CST walk rather than the [`Model`]'s: it wants the
//! *outline* of every top-level binding, where the cursor half wants whatever
//! sits under one offset.

mod analysis;
mod budget;
mod features;
mod line_index;
mod model;
mod symbols;
mod walk006;
mod walk01;

#[cfg(feature = "server")]
pub mod jsonrpc;
#[cfg(feature = "typecheck")]
pub mod project;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
mod workspace;

pub use analysis::{analyze, analyze_auto, analyze_detected, detect_version, Diag, Severity};
pub use features::{completions, definition, hover, Completion, Definition, Hover};
pub use line_index::{LineIndex, Position};
pub use model::{build_model, ByteRange, Def, HeaderKind, HeaderRef, Hit, Model, Ns, Opaque, Ref};
pub use symbols::{
    document_symbols, document_symbols_auto, document_symbols_detected, Range, Symbol, SymbolKind,
};

/// Re-exported so a consumer of [`analyze`] does not have to depend on
/// `rustyfi-syntax` directly just to name its second argument.
pub use rustyfi_syntax::RustyfiVersion;
