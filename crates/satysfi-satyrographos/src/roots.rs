//! Library-root resolution (plan §3/§4) and the managed-root marker
//! (plan §6).
//!
//! A "root" here is a SATySFi library root: the directory that holds
//! `dist/packages/…` (where the loader resolves `@require:` from, see
//! `satysfi-loader`) and, for this port, a `.satyrographos/` bookkeeping
//! subtree (receipts + staging). `install` materialises into `<root>/dist/…`
//! and records `<root>/.satyrographos/receipts/<name>.toml`.

use std::path::{Path, PathBuf};

use crate::error::Error;

/// The marker/bookkeeping directory under a managed root.
pub const MANAGED_DIR: &str = ".satyrographos";

/// Resolve the root to operate on, following the plan §4 precedence for the
/// *library-management* commands (the compile-mode version→path fallback of
/// §3 step 4 lives in the CLI layer, not here — this crate never sees a
/// `SatysfiVersion`):
///
/// 1. `dest` (the raw `--dest DIR` override) — used verbatim, bypassing
///    discovery entirely.
/// 2. `lib_root` (the `--lib-root DIR` flag).
/// 3. `$SATYSFI_LIB_ROOT`.
///
/// If none is available, [`Error::RootResolution`] (CLI exit `3`). `dest`
/// and `lib_root` are mutually exclusive at the CLI (an `ArgGroup`); if both
/// somehow arrive here, `dest` wins.
pub fn resolve_root(lib_root: Option<&Path>, dest: Option<&Path>) -> Result<PathBuf, Error> {
    if let Some(dest) = dest {
        return Ok(dest.to_path_buf());
    }
    if let Some(lib_root) = lib_root {
        return Ok(lib_root.to_path_buf());
    }
    if let Some(env) = std::env::var_os("SATYSFI_LIB_ROOT") {
        return Ok(PathBuf::from(env));
    }
    Err(Error::RootResolution)
}

/// The `<root>/.satyrographos/` bookkeeping directory.
pub fn managed_dir(root: &Path) -> PathBuf {
    root.join(MANAGED_DIR)
}

/// Whether `root` is already managed by this tool (its `.satyrographos/`
/// marker exists, even if empty) — plan §6's managed-root check.
pub fn is_managed(root: &Path) -> bool {
    managed_dir(root).is_dir()
}

/// Ensure `root` is a managed root, creating the `.satyrographos/`,
/// `.satyrographos/receipts/`, and `.satyrographos/tmp/` skeleton on first
/// use (plan §6: "`install`/`uninstall` create it on first use"). Idempotent.
pub fn ensure_managed(root: &Path) -> Result<(), Error> {
    for sub in [managed_dir(root), receipts_dir(root), tmp_dir(root)] {
        std::fs::create_dir_all(&sub).map_err(|e| Error::io(&sub, e))?;
    }
    Ok(())
}

/// `<root>/.satyrographos/receipts/`.
pub fn receipts_dir(root: &Path) -> PathBuf {
    managed_dir(root).join("receipts")
}

/// `<root>/.satyrographos/tmp/` — staging and archive extraction, kept under
/// `<root>` so the final rename into `dist/` is same-filesystem/atomic
/// (plan §6).
pub fn tmp_dir(root: &Path) -> PathBuf {
    managed_dir(root).join("tmp")
}
