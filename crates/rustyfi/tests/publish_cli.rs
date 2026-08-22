//! `publish` at the CLI — **no network, no TTY**: drive the built binary
//! against a plain-directory repository in a tempdir.
//!
//! Three things only this level can show: that the subcommand is wired into
//! BOTH personalities, that standing in the project directory is enough (no
//! `--project`), and that a published package is immediately visible to
//! `search` — the same index reader `install` goes through.
//!
//! The push is never run. `publish` prints it; that it stays printed is what
//! the last assertion here checks.

#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const SHA: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const URL: &str = "https://example.invalid/great-package-1.0.0.tar.gz";

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

fn tmpdir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-publish-cli-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

/// Run under an overridden `argv[0]` (the multicall dispatch key), from `cwd`,
/// with every ambient registry/config setting removed so the test's own
/// arguments are the only input.
fn run(arg0: &str, cwd: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .arg0(arg0)
        .args(args)
        .current_dir(cwd)
        .env_remove("RUSTYFI_REGISTRY")
        .env_remove("RUSTYFI_CONFIG_DIR")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", cwd.join("no-such-home"))
        .output()
        .expect("run the binary")
}

fn project(dir: &Path) -> PathBuf {
    let root = dir.join("great-package");
    fs::create_dir_all(root.join("packages")).unwrap();
    fs::write(
        root.join("Satyristes"),
        "(version 0.0.2)\n\
         (library (name \"great-package\") (version \"1.0.0\") (lang 0.0)\n  \
           (sources ((packageDir \"packages\")))\n  \
           (dependencies ((base ()))))\n",
    )
    .unwrap();
    root
}

/// A repository already holding one OPAM package, so the shape is detectable.
fn opam_repo(dir: &Path) -> PathBuf {
    let repo = dir.join("repo");
    let existing = repo.join("packages/satysfi-other/satysfi-other.0.1.0");
    fs::create_dir_all(&existing).unwrap();
    fs::write(
        existing.join("opam"),
        "opam-version: \"2.0\"\nsynopsis: \"a neighbour\"\nurl {\n  \
         src: \"https://example.invalid/other.tar.gz\"\n  checksum: [ \"sha512=aaaa\" ]\n}\n",
    )
    .unwrap();
    repo
}

#[test]
fn publishing_from_the_project_directory_makes_the_package_searchable() {
    let dir = tmpdir("roundtrip");
    let proj = project(&dir);
    let repo = opam_repo(&dir);

    // No `--project`: the Satyristes is found from the working directory.
    let out = run(
        "rustyfi",
        &proj,
        &[
            "publish",
            "--url",
            URL,
            "--sha256",
            SHA,
            "--registry",
            repo.to_str().unwrap(),
            "--description",
            "A great SATySFi package",
        ],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "publish should succeed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("published great-package 1.0.0 as satysfi-great-package (opam repository)"),
        "{stdout}"
    );
    assert!(stdout.contains("install as `great-package`"), "{stdout}");
    // The push is a printed next step, never a performed one.
    assert!(stdout.contains("next:"), "{stdout}");
    assert!(stdout.contains("push origin HEAD"), "{stdout}");

    // The same index reader `install` uses now sees it, under the name a
    // `@require:` or a dependency entry writes.
    let out = run(
        "satyrographos",
        &dir,
        &["search", "great", "--registry", repo.to_str().unwrap()],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "search should succeed:\n{stdout}");
    assert!(
        stdout.contains("great-package (satysfi-great-package) 1.0.0 — A great SATySFi package"),
        "{stdout}"
    );
}

#[test]
fn several_configured_repositories_fail_with_a_list_rather_than_a_guess() {
    let dir = tmpdir("ambiguous");
    let proj = project(&dir);
    let first = opam_repo(&tmpdir("ambiguous-a"));
    let second = opam_repo(&tmpdir("ambiguous-b"));
    let config = dir.join("config.toml");
    fs::write(
        &config,
        format!(
            "[[registry]]\nurl = \"{}\"\n\n[[registry]]\nurl = \"{}\"\n",
            first.display(),
            second.display()
        ),
    )
    .unwrap();

    let out = run(
        "satyrographos",
        &proj,
        &[
            "publish",
            "--url",
            URL,
            "--sha256",
            SHA,
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "nothing resolved to publish into"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains(&first.display().to_string()), "{stderr}");
    assert!(stderr.contains(&second.display().to_string()), "{stderr}");
    assert!(stderr.contains("--registry"), "{stderr}");
    // Neither repository was written to.
    for repo in [&first, &second] {
        assert!(
            !repo.join("packages/satysfi-great-package").exists(),
            "{} must be untouched",
            repo.display()
        );
    }
}
