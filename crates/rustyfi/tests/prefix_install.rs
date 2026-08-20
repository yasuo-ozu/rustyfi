//! An unpacked archive is self-contained: the binary finds its packages and
//! its shipped defaults relative to ITSELF, so `<prefix>/bin/rustyfi` works
//! from any directory with nothing exported.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmpdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "rustyfi-prefix-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&p).expect("temp dir");
    p
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), to).unwrap();
        }
    }
}

/// `<prefix>/{bin,lib,share}` laid out the way the release archive unpacks.
fn stage_prefix(tag: &str) -> PathBuf {
    let prefix = tmpdir(tag);
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::create_dir_all(prefix.join("share/rustyfi")).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_rustyfi"), prefix.join("bin/rustyfi")).unwrap();

    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    copy_dir(
        &repo.join("lib-rustyfi/dist/packages"),
        &prefix.join("lib/rustyfi/dist/packages"),
    );
    prefix
}

/// Run the staged binary with every relevant variable cleared, from a
/// directory that is not inside the prefix.
fn run(prefix: &Path, cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new(prefix.join("bin/rustyfi"))
        .args(args)
        .current_dir(cwd)
        .env_remove("RUSTYFI_LIB_ROOT")
        .env_remove("RUSTYFI_REGISTRY")
        .env_remove("RUSTYFI_CONFIG_DIR")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", prefix.join("no-such-home"))
        .output()
        .expect("run the staged binary")
}

#[test]
fn lib_root_resolves_relative_to_the_executable() {
    let prefix = stage_prefix("lib");
    let work = tmpdir("lib-work");
    fs::write(
        work.join("doc.saty"),
        "@require: stdja-mini\ndocument (| title = {t}; author = {a}; |) '< +p { hi } >\n",
    )
    .unwrap();

    let out = run(&prefix, &work, &["doc.saty", "--no-cache"]);
    assert!(
        out.status.success(),
        "a package `@require:` should resolve from <exe>/../lib/rustyfi:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(work.join("doc.pdf").is_file());

    let _ = fs::remove_dir_all(&prefix);
    let _ = fs::remove_dir_all(&work);
}

#[test]
fn the_shipped_config_is_found_relative_to_the_executable() {
    let prefix = stage_prefix("share");
    let work = tmpdir("share-work");

    // A repository the shipped config names, so its effect is observable.
    let index = prefix.join("index/packages");
    fs::create_dir_all(&index).unwrap();
    fs::write(
        index.join("great-package.toml"),
        "description = \"shipped-config demo\"\n\n\
         [versions.\"1.0.0\"]\n\
         tarball_url = \"file:///nonexistent.tar.gz\"\n\
         sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n",
    )
    .unwrap();
    fs::write(
        prefix.join("share/rustyfi/config.toml"),
        format!("[registry]\nurl = \"{}\"\n", prefix.join("index").display()),
    )
    .unwrap();

    let out = run(&prefix, &work, &["search", "demo"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("great-package"),
        "search should use <exe>/../share/rustyfi/config.toml:\n{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // And a config the user wrote shadows the shipped one.
    let user = tmpdir("share-user");
    fs::write(user.join("config.toml"), "[registry]\nurl = \"file:///nowhere\"\n").unwrap();
    let out = Command::new(prefix.join("bin/rustyfi"))
        .args(["search", "demo"])
        .current_dir(&work)
        .env_remove("RUSTYFI_REGISTRY")
        .env("RUSTYFI_CONFIG_DIR", &user)
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("great-package"),
        "the user's config must win over the shipped one"
    );

    for d in [prefix, work, user] {
        let _ = fs::remove_dir_all(d);
    }
}

#[test]
fn the_config_flag_names_the_file_and_reports_its_failures() {
    let prefix = stage_prefix("flag");
    let work = tmpdir("flag-work");

    let index = prefix.join("index/packages");
    fs::create_dir_all(&index).unwrap();
    fs::write(
        index.join("great-package.toml"),
        "description = \"flagged demo\"\n\n\
         [versions.\"1.0.0\"]\n\
         tarball_url = \"file:///nonexistent.tar.gz\"\n\
         sha256 = \"0000000000000000000000000000000000000000000000000000000000000000\"\n",
    )
    .unwrap();
    let named = work.join("named.toml");
    fs::write(
        &named,
        format!("[registry]\nurl = \"{}\"\n", prefix.join("index").display()),
    )
    .unwrap();

    // The file the flag names is read, even though nothing else points there.
    let out = run(&prefix, &work, &["search", "demo", "--config", named.to_str().unwrap()]);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("great-package"),
        "--config should be consulted:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // A file named explicitly and missing is an error, unlike an absent
    // discovered one.
    let out = run(&prefix, &work, &["search", "demo", "--config", "/no/such/config.toml"]);
    assert!(!out.status.success(), "a named-but-missing config should fail");

    // And a malformed one says so, naming the file.
    let bad = work.join("bad.toml");
    fs::write(&bad, "[registry]\nurl = 42\n").unwrap();
    let out = run(&prefix, &work, &["search", "demo", "--config", bad.to_str().unwrap()]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && stderr.contains("invalid config"), "{stderr}");

    for d in [prefix, work] {
        let _ = fs::remove_dir_all(d);
    }
}

#[test]
fn the_config_flag_is_accepted_by_every_command() {
    // It used to exist only on the commands that read it, so `list --config F`
    // was a usage error — surprising for a flag describing the run.
    let prefix = stage_prefix("global-flag");
    let work = tmpdir("global-flag-work");
    let cfg = work.join("config.toml");
    fs::write(&cfg, "[registry]\nurl = \"file:///nowhere\"\n").unwrap();

    for cmd in ["list", "status", "search", "update", "install", "build", "uninstall"] {
        let out = run(
            &prefix,
            &work,
            &[cmd, "--config", cfg.to_str().unwrap(), "--help"],
        );
        assert!(
            out.status.success(),
            "`{cmd} --config F --help` should parse:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    for d in [prefix, work] {
        let _ = fs::remove_dir_all(d);
    }
}
