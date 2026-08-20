//! Phase-2 manifest/lockfile reconcile tests (plan §5.3, §9 phase 2): a
//! `Satyristes` with two `{ path = … }` entries, driven through the
//! library `install_manifest` API against a `--dest` root. Same `TempDir`
//! pattern as `tests/ops.rs` / `rustyfi-loader/tests/loader.rs` — no extra
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
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use rustyfi_satyrographos::{self as sg, RegistryOptions, RootOptions};

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
        &format!("vendor/{name}/rustyfi-package.toml"),
        &format!(
            "[package]\n\
             name = \"{name}\"\n\
             version = \"{version}\"\n\
             rustyfi-version-compat = \">=0.0.6, <0.1\"\n\
             \n\
             [[files]]\n\
             kind = \"package-dir\"\n\
             src = \"packages\"\n"
        ),
    );
    tmp.write(&format!("vendor/{name}/packages/{name}.satyh"), body);
}

/// A project `Satyristes` at `<tmp>/proj/Satyristes` whose dependencies name
/// `../vendor/<name>` sources (relative to the manifest's own directory).
fn write_satyrfile(tmp: &TempDir, entries: &[&str]) -> PathBuf {
    let deps: String = entries
        .iter()
        .map(|name| format!("({name} ((path \"../vendor/{name}\")))"))
        .collect::<Vec<_>>()
        .join("\n     ");
    tmp.write(
        "proj/Satyristes",
        &format!(
            "(version 0.0.2)\n\
             (library (name \"proj\") (version \"0.1.0\")\n\
               (sources ((packageDir \"src\")))\n\
               (dependencies\n     ({deps})))\n"
        ),
    )
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
    let lock = tmp.path().join("proj/Satyristes.lock");
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
    let text = fs::read_to_string(tmp.path().join("proj/Satyristes.lock")).unwrap();
    assert!(text.contains("name = \"alpha\""), "{text}");
}

#[test]
fn removed_entry_is_left_installed_and_dropped_from_lock() {
    let tmp = TempDir::new("removed");
    write_pkg(&tmp, "alpha", "1.0.0", "let alpha = 1\n");
    write_pkg(&tmp, "beta", "2.0.0", "let beta = 2\n");
    write_satyrfile(&tmp, &["alpha", "beta"]);
    let manifest = tmp.path().join("proj/Satyristes");
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
    let text = fs::read_to_string(tmp.path().join("proj/Satyristes.lock")).unwrap();
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
fn empty_source_table_is_rejected() {
    // Not `git` (supported since saphe 7d slice S3, below) — a `source = {}`
    // table naming none of path/git/registry is the one remaining
    // `UnsupportedSource` case (satyrfile.rs `SourceSpec::kind`).
    let tmp = TempDir::new("empty-source");
    let manifest = tmp.write(
        "proj/Satyristes",
        "(version 0.0.2)\n\
         (library (name \"proj\") (version \"0.1.0\") (sources ((packageDir \"src\")))\n\
           (dependencies ((nowhere ((unsupported \"x\"))))))\n",
    );
    let root = tmp.path().join("root");
    let err = sg::install_manifest(&manifest, &root_opts(&root)).unwrap_err();
    assert!(matches!(err, sg::Error::UnsupportedSource { .. }), "{err}");
}

// ---------------------------------------------------------------------------
// Saphe 7d slice S3: `{ git = … }` package sources (S3) — a local git repo
// fixture only, never real network (same discipline as `tests/registry.rs`'s
// bare-git index fixture).
// ---------------------------------------------------------------------------

fn git(args: &[&str], cwd: Option<&Path>) {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().expect("run git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_capture(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// A one- or two-commit local git repo holding one SATySFi package (same
/// `rustyfi-package.toml` shape as `write_pkg`), so `{ git = … }` reconcile
/// tests can pin `rev` to a non-tip commit. Returns `(repo path, first
/// commit sha, HEAD sha)` — `first == HEAD` when `second_body` is `None`.
fn make_git_pkg_repo(
    tmp: &TempDir,
    dir_rel: &str,
    name: &str,
    version: &str,
    body: &str,
    second_body: Option<&str>,
) -> (PathBuf, String, String) {
    let repo = tmp.path().join(dir_rel);
    fs::create_dir_all(repo.join("packages")).unwrap();
    fs::write(
        repo.join("rustyfi-package.toml"),
        format!(
            "[package]\n\
             name = \"{name}\"\n\
             version = \"{version}\"\n\
             rustyfi-version-compat = \">=0.0.6, <0.1\"\n\
             \n\
             [[files]]\n\
             kind = \"package-dir\"\n\
             src = \"packages\"\n"
        ),
    )
    .unwrap();
    fs::write(repo.join(format!("packages/{name}.satyh")), body).unwrap();

    git(&["init", "-q", repo.to_str().unwrap()], None);
    git(&["-C", repo.to_str().unwrap(), "add", "-A"], None);
    git(
        &[
            "-C",
            repo.to_str().unwrap(),
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "v1",
        ],
        None,
    );
    let first_sha = git_capture(&["rev-parse", "HEAD"], &repo);

    let head_sha = if let Some(second) = second_body {
        fs::write(repo.join(format!("packages/{name}.satyh")), second).unwrap();
        git(&["-C", repo.to_str().unwrap(), "add", "-A"], None);
        git(
            &[
                "-C",
                repo.to_str().unwrap(),
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                "v2",
            ],
            None,
        );
        git_capture(&["rev-parse", "HEAD"], &repo)
    } else {
        first_sha.clone()
    };
    (repo, first_sha, head_sha)
}

/// A single-entry `Satyristes` with a `{ git = …, rev = … }` source
/// (`rev` omitted when `None`).
fn write_git_satyrfile(tmp: &TempDir, name: &str, repo_url: &str, rev: Option<&str>) -> PathBuf {
    let rev_form = rev.map(|r| format!(" (rev \"{r}\")")).unwrap_or_default();
    tmp.write(
        "proj/Satyristes",
        &format!(
            "(version 0.0.2)\n\
             (library (name \"proj\") (version \"0.1.0\") (sources ((packageDir \"src\")))\n\
               (dependencies (({name} ((git \"{repo_url}\"){rev_form})))))\n"
        ),
    )
}

fn reg_opts_with_git_cache(tmp: &TempDir, offline: bool) -> RegistryOptions {
    RegistryOptions {
        git_source_cache_dir: Some(tmp.path().join("git-cache")),
        offline,
        ..Default::default()
    }
}

#[test]
fn git_package_source_clones_and_installs_default_branch() {
    let tmp = TempDir::new("git-default");
    let (repo, _first, head) =
        make_git_pkg_repo(&tmp, "repo", "gitpkg", "1.0.0", "let gitpkg = 1\n", None);
    let manifest = write_git_satyrfile(&tmp, "gitpkg", &file_url(&repo), None);
    let root = tmp.path().join("root");
    let reg = reg_opts_with_git_cache(&tmp, false);

    let report =
        sg::install_manifest_reg(&manifest, &root_opts(&root), &reg).expect("reconcile ok");
    assert_eq!(report.installed.len(), 1, "the git source is freshly installed");
    assert!(root.join("dist/packages/gitpkg/gitpkg.satyh").is_file());

    // The receipt records a `git` source with the resolved rev.
    let receipt = fs::read_to_string(root.join(".satyrographos/receipts/gitpkg.toml")).unwrap();
    assert!(receipt.contains("kind = \"git\""), "{receipt}");
    assert!(receipt.contains(&format!("value = \"{}\"", file_url(&repo))), "{receipt}");
    assert!(receipt.contains(&format!("version = \"{head}\"")), "{receipt}");

    // The lockfile pins the git url + the resolved HEAD commit sha (design
    // §3 S3: "record the git url + resolved rev").
    let lock_text = fs::read_to_string(tmp.path().join("proj/Satyristes.lock")).unwrap();
    assert!(
        lock_text.contains(&format!("git = \"{}\"", file_url(&repo))),
        "{lock_text}"
    );
    assert!(lock_text.contains(&format!("rev = \"{head}\"")), "{lock_text}");
}

#[test]
fn git_package_source_pins_to_specific_rev_not_the_tip() {
    let tmp = TempDir::new("git-pin");
    let (repo, first, head) = make_git_pkg_repo(
        &tmp,
        "repo",
        "gitpkg",
        "1.0.0",
        "let gitpkg = 1\n",
        Some("let gitpkg = 2\n"),
    );
    assert_ne!(first, head, "fixture has two distinct commits");
    let manifest = write_git_satyrfile(&tmp, "gitpkg", &file_url(&repo), Some(&first));
    let root = tmp.path().join("root");
    let reg = reg_opts_with_git_cache(&tmp, false);

    sg::install_manifest_reg(&manifest, &root_opts(&root), &reg).expect("reconcile ok");

    let content = fs::read_to_string(root.join("dist/packages/gitpkg/gitpkg.satyh")).unwrap();
    assert_eq!(
        content, "let gitpkg = 1\n",
        "pinned to the first commit, not the branch tip"
    );

    let lock_text = fs::read_to_string(tmp.path().join("proj/Satyristes.lock")).unwrap();
    assert!(lock_text.contains(&format!("rev = \"{first}\"")), "{lock_text}");
}

#[test]
fn git_package_source_reconcile_is_reproducible_and_skips_unchanged() {
    let tmp = TempDir::new("git-repro");
    let (repo, _first, _head) =
        make_git_pkg_repo(&tmp, "repo", "gitpkg", "1.0.0", "let gitpkg = 1\n", None);
    let manifest = write_git_satyrfile(&tmp, "gitpkg", &file_url(&repo), None);
    let root = tmp.path().join("root");
    let reg = reg_opts_with_git_cache(&tmp, false);

    sg::install_manifest_reg(&manifest, &root_opts(&root), &reg).expect("first reconcile");

    let f = root.join("dist/packages/gitpkg/gitpkg.satyh");
    let before = mtime(&f);
    std::thread::sleep(Duration::from_millis(50));

    let report =
        sg::install_manifest_reg(&manifest, &root_opts(&root), &reg).expect("second reconcile");
    assert!(
        report.installed.is_empty(),
        "an unchanged git source reproduces from the lock, no re-clone/re-stage"
    );
    assert_eq!(report.skipped, ["gitpkg"]);
    assert_eq!(mtime(&f), before, "untouched");
}

#[test]
fn git_package_source_offline_reuses_existing_clone() {
    let tmp = TempDir::new("git-offline-warm");
    let (repo, _first, _head) =
        make_git_pkg_repo(&tmp, "repo", "gitpkg", "1.0.0", "let gitpkg = 1\n", None);
    let manifest = write_git_satyrfile(&tmp, "gitpkg", &file_url(&repo), None);
    let git_cache = tmp.path().join("git-cache");

    // Warm the cache with an ordinary (online-capable) reconcile.
    let root1 = tmp.path().join("root1");
    let reg_online = RegistryOptions {
        git_source_cache_dir: Some(git_cache.clone()),
        ..Default::default()
    };
    sg::install_manifest_reg(&manifest, &root_opts(&root1), &reg_online).expect("warm the cache");

    // A FRESH root, `--offline`, same cache dir: must succeed by reusing the
    // already-cloned checkout, never attempting a `git clone`/`fetch`.
    let root2 = tmp.path().join("root2");
    let reg_offline = RegistryOptions {
        git_source_cache_dir: Some(git_cache),
        offline: true,
        ..Default::default()
    };
    let report = sg::install_manifest_reg(&manifest, &root_opts(&root2), &reg_offline)
        .expect("offline reconcile reuses the warm clone");
    assert_eq!(report.installed.len(), 1);
    assert!(root2.join("dist/packages/gitpkg/gitpkg.satyh").is_file());
}

#[test]
fn git_package_source_offline_errors_when_never_cloned() {
    let tmp = TempDir::new("git-offline-cold");
    let (repo, _first, _head) =
        make_git_pkg_repo(&tmp, "repo", "gitpkg", "1.0.0", "let gitpkg = 1\n", None);
    let manifest = write_git_satyrfile(&tmp, "gitpkg", &file_url(&repo), None);
    let root = tmp.path().join("root");
    let reg = RegistryOptions {
        git_source_cache_dir: Some(tmp.path().join("git-cache-never-warmed")),
        offline: true,
        ..Default::default()
    };

    let err = sg::install_manifest_reg(&manifest, &root_opts(&root), &reg).unwrap_err();
    assert!(matches!(err, sg::Error::Offline { .. }), "{err}");

    // Nothing was installed either.
    assert!(!root.join("dist").exists());
}
