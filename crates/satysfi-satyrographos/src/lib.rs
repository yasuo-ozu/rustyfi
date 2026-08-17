//! Satyrographos-style package management for the `satysfi-rust` port.
//!
//! This crate is the clap-free library half of the chimera CLI (see the
//! plan document `docs/chimera-satyrographos-plan.md`): given an options
//! struct and a source path, it materialises SATySFi packages under
//! `<lib_root>/dist/{packages,fonts,hash,md}/` (mirroring real
//! Satyrographos' layout) and records per-package receipts under
//! `<lib_root>/.satyrographos/receipts/`, so the thin `satysfi-cli` shell
//! only parses arguments and calls in here.
//!
//! ## Phase 1 scope (implemented here)
//!
//! - Root resolution (`roots`, plan §3/§4) and the managed-root marker.
//! - `satysfi-package.toml` manifest parsing plus the no-manifest
//!   `packages/`-flat-copy fallback (`manifest`, plan §5.1/§5.5).
//! - Directory *and* `.tar.gz`/`.tgz` sources, with a path-traversal
//!   ("zip-slip") guard (`archive`/`stage`, plan §6/§10).
//! - Per-package receipts (`receipts`, plan §5.2).
//! - The atomic install transaction — stage under `<root>/.satyrographos/
//!   tmp/`, then swap into place, orphaning a prior receipt's files on
//!   `--force` (`stage`, plan §6).
//! - `ops::{install, uninstall, list, status}` (plan §4.1-4.4).
//! - Phase 4's `Satyristes` S-expression reader (`satyristes`, plan §5.5):
//!   an alternative front-end that feeds the same install pipeline, so
//!   packages authored for real (OCaml) Satyrographos install directly.
//!
//! ## Phase 2 scope (also implemented here)
//!
//! - The project-level `Satyrfile.toml` manifest + `Satyrfile.lock`
//!   lockfile (`satyrfile`/`lockfile`, plan §5.3).
//! - `ops::reconcile::install_manifest` — the no-`PATH` `satyrographos
//!   install`: diff each manifest entry's source hash against the lockfile
//!   and the installed receipts, and re-materialise only changed/missing
//!   entries via the phase-1 `install` primitive (plan §8 phase 2).
//!
//! ## Phase 3 scope (also implemented here)
//!
//! - The remote registry (`registry`, plan §5.4): a git-hosted or
//!   plain-directory TOML index (`packages/<name>.toml`), acquired by shelling
//!   out to `git` (or read in place for a local index), with per-version
//!   `tarball_url` + `sha256`.
//! - `ops::registry_install` — the fetch → verify → materialise algorithm:
//!   download the tarball, verify its SHA-256 *before* touching `dist/`, then
//!   feed it through the phase-1 install pipeline with a `registry` receipt
//!   source (plan §5.4 steps 1-4).
//! - `{ registry = … }` sources in `Satyrfile.toml` now reconcile, locking the
//!   resolved `(version, url, sha256)` into `Satyrfile.lock` for reproducible
//!   re-installs without re-consulting the index.
//! - `ops::{search, update}` — index substring search and lockfile-vs-index
//!   upgrade reporting (plan §8).
//! - The HTTP transport is behind the `http` cargo feature (off by default), so
//!   path/archive/`file://` installs stay entirely offline (plan §8).
//!
//! Phase 5 (system fonts) is out of scope here.

mod archive;
pub mod error;
pub mod lockfile;
pub mod manifest;
pub mod ops;
pub mod receipts;
pub mod registry;
pub mod roots;
pub mod satyristes;
pub mod satyrfile;
pub mod solve;
mod stage;
mod util;
pub mod version;

pub use error::Error;

// Flat re-exports of the public API named in the plan's §7.2.
pub use ops::install::{install, InstallOptions, InstallReport};
pub use ops::list::{list, PackageSummary};
pub use ops::reconcile::{install_manifest, install_manifest_reg, ManifestReport};
pub use ops::status::{status, PackageStatus, StatusReport};
pub use ops::uninstall::{uninstall, RootOptions};

// Phase-2 manifest/lockfile schema types (plan §5.3).
pub use lockfile::{LockEntry, Lockfile};
pub use satyrfile::{find_upward, LibraryEntry, Satyrfile, SourceSpec};

// Phase-3 registry (plan §5.4): index client + the fetch/verify/materialize
// entry points and the new `search`/`update` operations.
pub use ops::registry_install::{install_registry, Resolved};
pub use ops::search::{search, SearchHit};
pub use ops::update::{update, Upgrade, UpdateReport};
pub use registry::{RegistryDepSource, RegistryOptions};

// Phase-7c solver (plan §7c / design-saphe-solver.md): the version/constraint
// value types and the backtracking dependency resolver.
pub use solve::{solve, DepSource, Solution};
pub use version::{Constraint, Version};
