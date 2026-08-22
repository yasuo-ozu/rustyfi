//! saphe-split / "Envelopes" resolution backend (`LoadMode::Envelopes`):
//! `use package` / `use … of` headers resolved against local files and
//! a pre-solved `rustyfi-deps.yaml` envelope graph. Parallels `v006/`
//! (the Legacy backend). Transcribed from `saphe-split @
//! b836d512689248d18970674021ecaca409e0d897`.
//!
//! - `open_doc.rs` ≈ `openFileDependencyResolver.ml`: the
//!   open/document resolver — `use … of` local resolution, typed errors for
//!   `use package` / bare `use`.
//! - `deps.rs` / `envelope.rs` ≈ `depsConfig.ml` /
//!   `envelopeConfig.ml`: the two compiler-side handoff YAML decoders.
//! - `closed.rs` ≈ `closedFileDependencyResolver.ml` +
//!   `closedEnvelopeDependencyResolver.ml`: pure graph logic, no YAML of its
//!   own; plus `envelope.rs`'s reader half (`read`: directory listing +
//!   per-source parse).
//!
//! `open_doc::load`'s `Some(deps)` path wires all of it together: decode the
//! config, read + topo-sort its envelopes, prepend their (closed-sorted)
//! module sources to the program, and validate every `use package M` header
//! against the config's `used_as` aliases.

pub(crate) mod closed;
pub(crate) mod deps;
pub(crate) mod envelope;
pub(crate) mod open_doc;
