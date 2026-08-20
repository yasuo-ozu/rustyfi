//! saphe-split / "Envelopes" resolution backend (`LoadMode::Envelopes`):
//! `use package` / `use … of` headers resolved against local files and
//! (Ld3b) a pre-solved `rustyfi-deps.yaml` envelope graph. Parallels `v006/`
//! (the Legacy backend) exactly as crate map lays out. Transcribed from
//! `saphe-split @ b836d512`, `src/frontend/openFileDependencyResolver.ml`
//! (`open_doc`) — see the Ld3a spec §0 for the full citation table.
//!
//! Ld3a shipped only `open_doc` (the open/document resolver: `use … of`
//! local resolution + typed errors for `use package` / bare `use`).
//!
//! **Ld3b-1** (landed here) adds the two compiler-side handoff YAML
//! decoders — `deps.rs` (≈ `depsConfig.ml`, `rustyfi-deps.yaml`) and
//! `envelope.rs` (≈ `envelopeConfig.ml`, `rustyfi-envelope.yaml`'s decode
//! half) — pinned to `saphe-split @
//! b836d512689248d18970674021ecaca409e0d897`. This is decode-only: nothing
//! in `open_doc.rs` calls either module yet, so `Envelopes { deps: Some(_)
//! }` behaves exactly as it did after Ld3a. See
//! `/home/yasuo/.claude/jobs/a7244c0b/tmp/axis-b-ld3b.md` for the full
//! design (§1's Ld3b-1/Ld3b-2 slicing).
//!
//! **Ld3b-2** (landed here) adds `closed.rs` (≈
//! `closedFileDependencyResolver.ml` + `closedEnvelopeDependencyResolver.ml`
//! — pure graph logic, no YAML of its own), `envelope.rs`'s reader half
//! (`read`: directory listing + per-source parse), and wires all of it into
//! `open_doc::load` — the `Some(deps)` path now decodes the config, reads +
//! topo-sorts its envelopes, prepends their (closed-sorted) module sources to
//! the program, and validates every `use package M` header against the
//! config's `used_as` aliases.

pub(crate) mod closed;
pub(crate) mod deps;
pub(crate) mod envelope;
pub(crate) mod open_doc;
