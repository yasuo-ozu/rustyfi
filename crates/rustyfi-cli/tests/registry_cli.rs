//! Phase-3 registry CLI tests (plan §5.4/§8/§9) — **no network**. Drive the
//! *built* `rustyfi-rust` binary under its `satyrographos` personality against
//! a plain-directory `file://` index (built in a tempdir), covering:
//!
//! - `install <name> --registry file://…` (registry form: the arg names no
//!   path on disk, so it is a registry package) → golden stdout + `dist/`;
//! - `search <term> --registry file://…` → golden stdout;
//! - `update` → golden upgrade report against a reconciled `Satyrfile.lock`;
//! - a corrupted index digest → exit `5`, nothing under `dist/`.
//!
//! All of this runs with the default (feature-off) binary, so the whole test
//! stays offline via `file://`.

#![cfg(unix)]

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi-rust"))
}

fn tmpdir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-cli-registry-{tag}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        n
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn sha256_of(path: &Path) -> String {
    let out = Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("sha256sum");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

/// Build a package tarball under `work/tarballs/` and return `(path, sha256)`.
fn make_tarball(work: &Path, name: &str, version: &str) -> (PathBuf, String) {
    let src = work.join(format!("src/{name}-{version}"));
    std::fs::create_dir_all(src.join("packages")).unwrap();
    std::fs::write(
        src.join("rustyfi-package.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\n\
             rustyfi-version-compat = \">=0.0.6, <0.1\"\n\n\
             [[files]]\nkind = \"package-dir\"\nsrc = \"packages\"\n"
        ),
    )
    .unwrap();
    std::fs::write(src.join(format!("packages/{name}.satyh")), "let x = 1\n").unwrap();

    let tarball = work.join(format!("tarballs/{name}-{version}.tar.gz"));
    std::fs::create_dir_all(tarball.parent().unwrap()).unwrap();
    let ok = Command::new("tar")
        .args([
            "-czf",
            tarball.to_str().unwrap(),
            "-C",
            src.to_str().unwrap(),
            ".",
        ])
        .status()
        .unwrap()
        .success();
    assert!(ok, "tar failed");
    let sha = sha256_of(&tarball);
    (tarball, sha)
}

/// Write `<index>/packages/<name>.toml` and return the index root.
fn write_index_entry(
    work: &Path,
    name: &str,
    description: &str,
    versions: &[(&str, &Path, &str)],
) -> PathBuf {
    let index = work.join("index");
    let file = index.join(format!("packages/{name}.toml"));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let mut toml = format!("description = \"{description}\"\n\n");
    for (v, tarball, sha) in versions {
        toml.push_str(&format!(
            "[versions.\"{v}\"]\ntarball_url = \"file://{}\"\nsha256 = \"{sha}\"\n\n",
            tarball.display()
        ));
    }
    std::fs::write(&file, toml).unwrap();
    index
}

fn run(args: &[&str], cwd: Option<&Path>) -> Output {
    let mut cmd = Command::new(bin());
    cmd.arg0("satyrographos").args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.output().expect("spawn satyrographos")
}

fn file_url(p: &Path) -> String {
    format!("file://{}", p.display())
}

#[test]
fn cli_registry_install_from_plain_index() {
    let work = tmpdir("cli-install");
    let (tarball, sha) = make_tarball(&work, "great-package", "1.0.0");
    let index = write_index_entry(
        &work,
        "great-package",
        "A great SATySFi package",
        &[("1.0.0", &tarball, &sha)],
    );
    let root = work.join("root");

    // `great-package` names no path on disk → registry form.
    let out = run(
        &[
            "install",
            "great-package",
            "--registry",
            &file_url(&index),
            "--dest",
            root.to_str().unwrap(),
        ],
        None,
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("installed great-package 1.0.0"), "{stdout}");
    assert!(root
        .join("dist/packages/great-package/great-package.satyh")
        .is_file());

    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn cli_search_golden_output() {
    let work = tmpdir("cli-search");
    let (gp, gp_sha) = make_tarball(&work, "great-package", "1.0.0");
    write_index_entry(
        &work,
        "great-package",
        "A great SATySFi package",
        &[("1.0.0", &gp, &gp_sha)],
    );
    let (ft, ft_sha) = make_tarball(&work, "fonts-theano", "0.2.0");
    let index = write_index_entry(
        &work,
        "fonts-theano",
        "Theano didot fonts",
        &[("0.2.0", &ft, &ft_sha)],
    );

    let out = run(
        &["search", "package", "--registry", &file_url(&index)],
        None,
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // "package" matches great-package's name/description only.
    assert_eq!(
        stdout, "great-package 1.0.0 — A great SATySFi package\n",
        "{stdout:?}"
    );

    // A broad term ("a" appears in both names) listing both, sorted by name.
    let out = run(&["search", "a", "--registry", &file_url(&index)], None);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout,
        "fonts-theano 0.2.0 — Theano didot fonts\ngreat-package 1.0.0 — A great SATySFi package\n",
        "{stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn cli_update_golden_output() {
    let work = tmpdir("cli-update");
    let (t10, sha10) = make_tarball(&work, "great-package", "1.0.0");
    let (t11, sha11) = make_tarball(&work, "great-package", "1.1.0");
    let index = write_index_entry(
        &work,
        "great-package",
        "A great SATySFi package",
        &[("1.0.0", &t10, &sha10), ("1.1.0", &t11, &sha11)],
    );

    // A project pinning 1.0.0; reconcile to write the lockfile at 1.0.0.
    let proj = work.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join("Satyrfile.toml"),
        format!(
            "[registry]\nurl = \"{}\"\n\n\
             [[library]]\nname = \"great-package\"\n\
             source = {{ registry = \"great-package\", version = \"1.0.0\" }}\n",
            file_url(&index)
        ),
    )
    .unwrap();
    let root = work.join("root");
    let out = run(&["install", "--dest", root.to_str().unwrap()], Some(&proj));
    assert!(
        out.status.success(),
        "reconcile stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // `update` reports 1.1.0 as an available upgrade (does not apply it).
    let out = run(&["update"], Some(&proj));
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("great-package: 1.0.0 -> 1.1.0 available"),
        "{stdout}"
    );
    // The lock is unchanged (still 1.0.0) — update only reports.
    let lock = std::fs::read_to_string(proj.join("Satyrfile.lock")).unwrap();
    assert!(lock.contains("version = \"1.0.0\""), "{lock}");
    assert!(
        !lock.contains("1.1.0"),
        "update must not apply the upgrade:\n{lock}"
    );

    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn cli_sha256_mismatch_exits_5_and_touches_nothing() {
    let work = tmpdir("cli-mismatch");
    let (tarball, _real) = make_tarball(&work, "great-package", "1.0.0");
    let bad = "0000000000000000000000000000000000000000000000000000000000000000";
    let index = write_index_entry(&work, "great-package", "x", &[("1.0.0", &tarball, bad)]);
    let root = work.join("root");

    let out = run(
        &[
            "install",
            "great-package",
            "--registry",
            &file_url(&index),
            "--dest",
            root.to_str().unwrap(),
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(5), "checksum mismatch is exit 5");
    assert!(
        !root.join("dist").exists(),
        "dist/ must be untouched on mismatch"
    );

    let _ = std::fs::remove_dir_all(&work);
}
