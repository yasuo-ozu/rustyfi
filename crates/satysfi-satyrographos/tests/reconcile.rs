//! Phase-2 manifest/lockfile reconcile tests (plan §5.3, §9 phase 2): a
//! `Satyrfile.toml` with two `{ path = … }` entries, driven through the
//! library `install_manifest` API against a `--dest` root. Same `TempDir`
//! pattern as `tests/ops.rs` / `satysfi-loader/tests/loader.rs` — no extra
//! dev-dependencies.
//!
//! Coverage:
//! - a no-change second reconcile skips every entry and leaves file **mtimes**
//!   untouched;
//! - editing one source re-materialises *only* that entry (content updated),
//!   the other entry's mtime preserved;
//! - dropping an entry from the manifest leaves its files installed (phase 2
//!   does not prune) and drops it from the rewritten lockfile;
//! - a hand-deleted receipt (lockfile drift) self-heals on the next reconcile.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use satysfi_satyrographos::{self as sg, RootOptions};

// ---------------------------------------------------------------------------
// Temp-dir fixture helper (same as tests/ops.rs).
// ---------------------------------------------------------------------------

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satyrographos-reconcile-{tag}-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&p, content).expect("write fixture file");
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Write a manifest-form package source at `<tmp>/vendor/<name>/`.
fn write_pkg(tmp: &TempDir, name: &str, version: &str, body: &str) {
    tmp.write(
        &format!("vendor/{name}/satysfi-package.toml"),
        &format!(
            "[package]\n\
             name = \"{name}\"\n\
             version = \"{version}\"\n\
             satysfi-version-compat = \">=0.0.6, <0.1\"\n\
             \n\
             [[files]]\n\
             kind = \"package-dir\"\n\
             src = \"packages\"\n"
        ),
    );
    tmp.write(&format!("vendor/{name}/packages/{name}.satyh"), body);
}

/// A two-entry `Satyrfile.toml` at `<tmp>/proj/Satyrfile.toml`, sources under
/// `../vendor/<name>` (relative to the manifest's own directory).
fn write_satyrfile(tmp: &TempDir, entries: &[&str]) -> PathBuf {
    let mut text = String::new();
    for name in entries {
        text.push_str(&format!(
            "[[library]]\nname = \"{name}\"\nsource = {{ path = \"../vendor/{name}\" }}\n\n"
        ));
    }
    tmp.write("proj/Satyrfile.toml", &text)
}

fn root_opts(root: &Path) -> RootOptions {
    RootOptions {
        dest: Some(root.to_path_buf()),
        ..Default::default()
    }
}

fn mtime(p: &Path) -> SystemTime {
    fs::metadata(p).expect("stat").modified().expect("mtime")
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn first_reconcile_installs_all_and_writes_lockfile() {
    let tmp = TempDir::new("first");
    write_pkg(&tmp, "alpha", "1.0.0", "let alpha = 1\n");
    write_pkg(&tmp, "beta", "2.0.0", "let beta = 2\n");
    let manifest = write_satyrfile(&tmp, &["alpha", "beta"]);
    let root = tmp.path().join("root");

    let report = sg::install_manifest(&manifest, &root_opts(&root)).expect("reconcile ok");
    assert_eq!(report.installed.len(), 2, "both entries fresh-installed");
    assert!(report.skipped.is_empty());

    assert!(root.join("dist/packages/alpha/alpha.satyh").is_file());
    assert!(root.join("dist/packages/beta/beta.satyh").is_file());

    // The lockfile was written next to the manifest, listing both entries with
    // a resolved sha256.
    let lock = tmp.path().join("proj/Satyrfile.lock");
    assert!(lock.is_file(), "lockfile written");
    let text = fs::read_to_string(&lock).unwrap();
    assert!(text.contains("name = \"alpha\""), "{text}");
    assert!(text.contains("name = \"beta\""), "{text}");
    assert!(text.contains("sha256"), "{text}");
}

#[test]
fn second_reconcile_no_change_preserves_mtimes() {
    let tmp = TempDir::new("mtime");
    write_pkg(&tmp, "alpha", "1.0.0", "let alpha = 1\n");
    write_pkg(&tmp, "beta", "2.0.0", "let beta = 2\n");
    let manifest = write_satyrfile(&tmp, &["alpha", "beta"]);
    let root = tmp.path().join("root");

    sg::install_manifest(&manifest, &root_opts(&root)).expect("first reconcile");

    let f_alpha = root.join("dist/packages/alpha/alpha.satyh");
    let f_beta = root.join("dist/packages/beta/beta.satyh");
    let before_alpha = mtime(&f_alpha);
    let before_beta = mtime(&f_beta);

    // Ensure a wall-clock gap so any rewrite would move the mtime.
    std::thread::sleep(Duration::from_millis(50));

    let report = sg::install_manifest(&manifest, &root_opts(&root)).expect("second reconcile");
    assert!(report.installed.is_empty(), "nothing re-materialised");
    assert_eq!(report.skipped.len(), 2, "both entries skipped");

    assert_eq!(mtime(&f_alpha), before_alpha, "alpha untouched");
    assert_eq!(mtime(&f_beta), before_beta, "beta untouched");
}

#[test]
fn changed_source_rematerializes_only_that_entry() {
    let tmp = TempDir::new("changed");
    write_pkg(&tmp, "alpha", "1.0.0", "let alpha = 1\n");
    write_pkg(&tmp, "beta", "2.0.0", "let beta = 2\n");
    let manifest = write_satyrfile(&tmp, &["alpha", "beta"]);
    let root = tmp.path().join("root");

    sg::install_manifest(&manifest, &root_opts(&root)).expect("first reconcile");

    let f_alpha = root.join("dist/packages/alpha/alpha.satyh");
    let f_beta = root.join("dist/packages/beta/beta.satyh");
    let before_beta = mtime(&f_beta);

    std::thread::sleep(Duration::from_millis(50));

    // Edit only alpha's source.
    fs::write(
        tmp.path().join("vendor/alpha/packages/alpha.satyh"),
        "let alpha = 999\n",
    )
    .unwrap();

    let report = sg::install_manifest(&manifest, &root_opts(&root)).expect("second reconcile");
    assert_eq!(report.installed.len(), 1, "only alpha re-materialised");
    assert_eq!(report.installed[0].name, "alpha");
    assert_eq!(report.skipped, ["beta"], "beta skipped");

    // alpha's content is updated; beta is untouched (mtime preserved).
    assert_eq!(
        fs::read_to_string(&f_alpha).unwrap(),
        "let alpha = 999\n",
        "alpha content updated"
    );
    assert_eq!(mtime(&f_beta), before_beta, "beta untouched");

    // The lockfile hash for alpha changed to match the new source.
    let text = fs::read_to_string(tmp.path().join("proj/Satyrfile.lock")).unwrap();
    assert!(text.contains("name = \"alpha\""), "{text}");
}

#[test]
fn removed_entry_is_left_installed_and_dropped_from_lock() {
    let tmp = TempDir::new("removed");
    write_pkg(&tmp, "alpha", "1.0.0", "let alpha = 1\n");
    write_pkg(&tmp, "beta", "2.0.0", "let beta = 2\n");
    write_satyrfile(&tmp, &["alpha", "beta"]);
    let manifest = tmp.path().join("proj/Satyrfile.toml");
    let root = tmp.path().join("root");

    sg::install_manifest(&manifest, &root_opts(&root)).expect("first reconcile");

    // Drop beta from the manifest, keep alpha.
    write_satyrfile(&tmp, &["alpha"]);
    let report = sg::install_manifest(&manifest, &root_opts(&root)).expect("second reconcile");
    assert_eq!(report.removed, ["beta"], "beta reported as dropped");

    // Phase 2 does not prune: beta's files and receipt survive.
    assert!(
        root.join("dist/packages/beta/beta.satyh").is_file(),
        "dropped entry's files are left installed"
    );
    assert!(root.join(".satyrographos/receipts/beta.toml").is_file());

    // But the lockfile no longer lists beta (mirrors the manifest 1:1).
    let text = fs::read_to_string(tmp.path().join("proj/Satyrfile.lock")).unwrap();
    assert!(text.contains("name = \"alpha\""), "{text}");
    assert!(!text.contains("name = \"beta\""), "beta dropped from lock: {text}");
}

#[test]
fn deleted_receipt_self_heals() {
    let tmp = TempDir::new("selfheal");
    write_pkg(&tmp, "alpha", "1.0.0", "let alpha = 1\n");
    let manifest = write_satyrfile(&tmp, &["alpha"]);
    let root = tmp.path().join("root");

    sg::install_manifest(&manifest, &root_opts(&root)).expect("first reconcile");

    // Hand-delete the receipt (lockfile drift): the lock still records alpha's
    // hash, but the receipt is gone.
    fs::remove_file(root.join(".satyrographos/receipts/alpha.toml")).unwrap();

    let report = sg::install_manifest(&manifest, &root_opts(&root)).expect("reconcile heals");
    assert_eq!(report.installed.len(), 1, "drifted entry re-installed");
    assert_eq!(report.installed[0].name, "alpha");
    assert!(
        root.join(".satyrographos/receipts/alpha.toml").is_file(),
        "receipt restored"
    );
}

#[test]
fn deleted_file_self_heals() {
    let tmp = TempDir::new("selfheal-file");
    write_pkg(&tmp, "alpha", "1.0.0", "let alpha = 1\n");
    let manifest = write_satyrfile(&tmp, &["alpha"]);
    let root = tmp.path().join("root");

    sg::install_manifest(&manifest, &root_opts(&root)).expect("first reconcile");

    // A recorded file vanishes though the receipt still claims it.
    let f = root.join("dist/packages/alpha/alpha.satyh");
    fs::remove_file(&f).unwrap();

    let report = sg::install_manifest(&manifest, &root_opts(&root)).expect("reconcile heals");
    assert_eq!(report.installed.len(), 1, "entry with a missing file re-installed");
    assert!(f.is_file(), "missing file restored");
}

#[test]
fn git_source_is_rejected_in_phase_2() {
    let tmp = TempDir::new("git");
    let manifest = tmp.write(
        "proj/Satyrfile.toml",
        "[[library]]\nname = \"remote\"\nsource = { git = \"https://x\", rev = \"abc\" }\n",
    );
    let root = tmp.path().join("root");
    let err = sg::install_manifest(&manifest, &root_opts(&root)).unwrap_err();
    assert!(matches!(err, sg::Error::UnsupportedSource { .. }), "{err}");
}
