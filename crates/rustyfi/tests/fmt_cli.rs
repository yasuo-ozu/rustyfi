//! `rustyfi fmt`, driven as a real subprocess.
//!
//! What only a subprocess can prove is the part that is the whole contract of a
//! `--check` mode: **the exit code**. A unit test can assert that a function
//! returned `Declined`; only this can assert that a shell sees `6`. The codes
//! are also the easiest thing in the tool to get silently wrong — a `--check`
//! that always exits 0 is indistinguishable from a clean tree — so every code
//! the subcommand documents has a test here, and each one was mutation-tested
//! by making the binary return 0 in its place and watching the test fail.
//!
//! The formatter itself is `rustyfi-lsp`'s and is tested there. Under slice 0 it
//! is a provable no-op, so nothing here can produce a non-empty diff through
//! the CLI yet; the diff renderer's own tests live beside it in
//! `crates/rustyfi/src/fmt.rs`. The `--check` test asserts the property that
//! matters regardless of slice: after a check run, the file's **bytes and mtime
//! are both unchanged**, which is a real assertion even for a no-op formatter
//! because a `--check` that wrote its (identical) output back would still touch
//! the mtime and invalidate every build system watching it.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

/// The repository root — `crates/rustyfi/`'s grandparent.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/rustyfi lives two levels below the repo root")
        .to_path_buf()
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn fmt(args: &[&str]) -> Run {
    fmt_with_stdin(args, None)
}

fn fmt_with_stdin(args: &[&str], stdin: Option<&str>) -> Run {
    let mut child = Command::new(bin())
        .arg("fmt")
        .args(args)
        .stdin(match stdin {
            Some(_) => Stdio::piped(),
            None => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A relative path in `args` is resolved from here, and the default-path
        // case walks up from here looking for a `Satyristes` — so pinning it is
        // not tidiness, it is what makes those two tests mean anything.
        .current_dir(repo_root())
        .spawn()
        .expect("spawn `rustyfi fmt`");
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(text.as_bytes())
            .expect("write to `rustyfi fmt`");
    }
    let out = child.wait_with_output().expect("wait for `rustyfi fmt`");
    Run {
        code: out.status.code().expect("an exit code"),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A scratch directory that removes itself, so a failing assertion cannot leave
/// a half-formatted fixture behind for the next run to trip over.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "rustyfi-fmt-cli-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the scratch directory");
        Scratch(dir)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("write a fixture");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Bytes and mtime together: the pair that distinguishes "did not change the
/// file" from "did not touch the file".
fn stamp(path: &Path) -> (Vec<u8>, std::time::SystemTime) {
    let bytes = std::fs::read(path).expect("read the fixture");
    let modified = std::fs::metadata(path)
        .expect("stat the fixture")
        .modified()
        .expect("an mtime");
    (bytes, modified)
}

/// A valid 0.0.6 library, small enough to read and real enough to lex.
const CLEAN_V006: &str = "@require: stdja\n\nlet greeting = `hello`\nlet two = 1 + 1\n";

/// A 0.1 document, so the per-file sniff-and-fall-back path is exercised rather
/// than only 0.0.6's.
///
/// It was `"use package open Stdlib\n\nval two = 1 + 1\n"` — a bare top-level
/// `val`, which **0.1 has no production for**: a 0.1 binding lives inside a
/// `module`. So this fixture never parsed, took the old identity fallback,
/// came back byte-identical and passed. That is the silent-failure defect
/// caught in this very test file: "0 need reformatting" was
/// indistinguishable from "already formatted", and the assertion below was
/// vacuous for as long as it existed. Wrapping the binding in a `module` is
/// what makes it a 0.1 file rather than something that merely lexes as one.
const CLEAN_V01: &str =
    "use package open Stdlib\n\nmodule M = struct\n  val two = 1 + 1\nend\n";

/// Never lexes, under either generation: the backtick string literal is never
/// closed, so there is no token stream to re-emit.
const UNLEXABLE: &str = "@require: stdja\n\nlet broken = `no closing backtick\n";

/// Lexes under 0.0.6, does **not** parse: `1+h` lexes as the integer `1`
/// followed by the block-command token `+h`, which no expression accepts.
///
/// Deliberately untidy in three ways the whitespace-only formatter fixes and
/// the old identity fallback did not — trailing whitespace, a blank-line run
/// past the cap, and a tab in the indentation — so that a test can tell tier 2
/// from "returned unchanged". Without the untidiness, both look like `0 need
/// reformatting`, which is exactly how the defect hid.
const UNPARSEABLE: &str = "@require: stdja\n\nlet a = 1 + 1   \n\n\n\n\n\tlet bad = 1+h 2\n";

/// [`UNPARSEABLE`] after tier 2: trailing whitespace gone, the five-blank run
/// capped at two, the tab expanded to `CstOptions::tab_spaces` columns — and
/// every non-blank line otherwise exactly as written, because a buffer that
/// does not parse gets no layout.
const UNPARSEABLE_TIDIED: &str =
    "@require: stdja\n\nlet a = 1 + 1\n\n\n  let bad = 1+h 2\n";

#[test]
fn a_clean_file_exits_zero() {
    let s = Scratch::new("clean");
    let path = s.write("clean.satyh", CLEAN_V006);
    let r = fmt(&["--check", path.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 0, "stderr: {}\nstdout: {}", r.stderr, r.stdout);
    // A clean tree prints no diff at all, which is what makes the exit code
    // usable from a script without parsing anything.
    assert_eq!(r.stdout, "", "a clean file should produce no diff");
    assert!(
        r.stderr.contains("0 need reformatting, 0 declined"),
        "stderr: {}",
        r.stderr
    );
}

#[test]
fn a_clean_v01_file_exits_zero_without_a_lang_flag() {
    let s = Scratch::new("clean-v01");
    let path = s.write("clean.saty", CLEAN_V01);
    let r = fmt(&["--check", path.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "");
}

#[test]
fn a_file_that_does_not_lex_exits_six_and_is_not_written() {
    let s = Scratch::new("declined");
    let path = s.write("broken.satyh", UNLEXABLE);
    let before = stamp(&path);
    // NOT `--check`: this is the in-place mode, the one that writes. The point
    // of the test is that a decline reaches it and produces no write.
    let r = fmt(&[path.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 6, "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("declined"),
        "the reason should be named: {}",
        r.stderr
    );
    assert_eq!(
        stamp(&path),
        before,
        "a declined file must not be written, not even with identical bytes"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        UNLEXABLE
    );
}

#[test]
fn a_declined_file_exits_six_under_check_too() {
    let s = Scratch::new("declined-check");
    let path = s.write("broken.satyh", UNLEXABLE);
    let r = fmt(&["--check", path.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 6, "stderr: {}", r.stderr);
}

/// `6` outranks the clean verdict of everything around it: one broken file in a
/// directory of good ones still fails the run, and the good ones are still left
/// alone.
#[test]
fn one_declined_file_among_clean_ones_still_exits_six() {
    let s = Scratch::new("mixed");
    let good = s.write("good.satyh", CLEAN_V006);
    let bad = s.write("bad.satyh", UNLEXABLE);
    let before = stamp(&good);
    let r = fmt(&[s.0.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 6, "stderr: {}", r.stderr);
    assert!(r.stderr.contains("1 declined"), "stderr: {}", r.stderr);
    assert_eq!(stamp(&good), before);
    assert_eq!(std::fs::read_to_string(&bad).expect("read back"), UNLEXABLE);
}

/// **The defect.** A file that lexes but does not parse must not report
/// success.
///
/// Before `CstOutcome`, `format_cst` answered `Some(source)` from the identity
/// builder here, `--check` compared it to the file, found no difference and
/// exited `0` with "0 need reformatting, 0 declined". One line that does not
/// parse silenced the whole file and CI went green over a formatter that had
/// done nothing at all.
#[test]
fn a_file_that_does_not_parse_exits_seven_under_check() {
    let s = Scratch::new("unparsed-check");
    let path = s.write("bad.satyh", UNPARSEABLE);
    let r = fmt(&["--check", path.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 7, "stderr: {}\nstdout: {}", r.stderr, r.stdout);
    assert!(
        r.stderr.contains("1 unparsed"),
        "the summary must name the count: {}",
        r.stderr
    );
    assert!(
        r.stderr.contains("only whitespace was normalised"),
        "the warning must say what WAS done: {}",
        r.stderr
    );
    // And it is still a real `--check`: the improvement tier 2 would make is
    // shown as a diff, not swallowed.
    assert!(
        !r.stdout.is_empty(),
        "an unparsed file that tier 2 would change must still print its diff"
    );
}

/// Tier 2 is `crate::format`, not the identity — asserted through the bytes on
/// disk, which is the only place the difference is observable to a user.
///
/// This is the half a `--check` cannot prove: `--check` writes nothing, so a
/// tier that returned the input verbatim and a tier that tidied it would both
/// look like "the file is unchanged on disk" there.
#[test]
fn an_unparsed_file_is_still_tidied_in_place() {
    let s = Scratch::new("unparsed-write");
    let path = s.write("bad.satyh", UNPARSEABLE);
    let r = fmt(&[path.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 7, "stderr: {}", r.stderr);
    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        UNPARSEABLE_TIDIED,
        "tier 2 must be the whitespace-only formatter's output. Returning the \
         input verbatim — the old identity fallback — passes every other \
         assertion in this file."
    );
    // Idempotent, and still reported: a second run changes nothing and still
    // does not claim success.
    let again = fmt(&[path.to_str().expect("utf-8 path")]);
    assert_eq!(again.code, 7, "stderr: {}", again.stderr);
    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        UNPARSEABLE_TIDIED
    );
}

/// `6 > 7`: a file that does not lex is worse news than one that merely does
/// not parse, because the first produced no bytes at all.
#[test]
fn a_decline_outranks_an_unparsed_file() {
    let s = Scratch::new("unparsed-vs-declined");
    s.write("bad.satyh", UNPARSEABLE);
    s.write("worse.satyh", UNLEXABLE);
    let r = fmt(&["--check", s.0.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 6, "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("1 declined, 1 unparsed"),
        "both counts are still reported: {}",
        r.stderr
    );
}

/// Stdin takes the same road. Its exit code is computed in `run_stdin` rather
/// than by `Tally::exit_code`, so it is a second code path with the same
/// contract — and the one a `rustyfi fmt --check - < buffer` hook uses.
#[test]
fn stdin_that_does_not_parse_exits_seven() {
    let r = fmt_with_stdin(&["--check", "-"], Some(UNPARSEABLE));
    assert_eq!(r.code, 7, "stderr: {}", r.stderr);
    let out = fmt_with_stdin(&["-"], Some(UNPARSEABLE));
    assert_eq!(out.code, 7, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout, UNPARSEABLE_TIDIED,
        "stdin mode must emit tier 2's output, not the input"
    );
}

#[test]
fn check_writes_nothing() {
    let s = Scratch::new("check-writes-nothing");
    let path = s.write("clean.satyh", CLEAN_V006);
    let before = stamp(&path);
    let r = fmt(&["--check", path.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let after = stamp(&path);
    assert_eq!(
        after.0, before.0,
        "--check must not change the file's bytes"
    );
    assert_eq!(
        after.1, before.1,
        "--check must not even TOUCH the file — an identical write still moves \
         the mtime and invalidates every build system watching it"
    );
}

#[test]
fn emit_stdout_writes_nothing_and_prints_the_text() {
    let s = Scratch::new("emit-stdout");
    let path = s.write("clean.satyh", CLEAN_V006);
    let before = stamp(&path);
    let r = fmt(&["--emit", "stdout", path.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, CLEAN_V006);
    assert_eq!(stamp(&path), before);
}

/// stdin in, formatted text out.
///
/// The literal equality holds because [`CLEAN_V006`] is written the way the
/// formatter writes it; if a later slice changes that, the right fix is to
/// re-canonicalise the fixture, not to weaken the assertion. **Idempotence** is
/// asserted alongside it because that one is slice-independent — feeding the
/// output back in must be a fixed point at every slice, and it is one of the
/// three properties `crates/rustyfi-lsp/tests/format.rs:12-21` puts above
/// output stability.
#[test]
fn stdin_round_trips() {
    let r = fmt_with_stdin(&["-"], Some(CLEAN_V006));
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(
        r.stdout, CLEAN_V006,
        "the fixture should already be canonically formatted"
    );
    let again = fmt_with_stdin(&["-"], Some(&r.stdout));
    assert_eq!(again.code, 0, "stderr: {}", again.stderr);
    assert_eq!(again.stdout, r.stdout, "formatting must be idempotent");
}

/// A file whose last line has no terminator must come back whole.
///
/// `main` leaves through `std::process::exit`, which skips the runtime's
/// end-of-`main` stdout flush; Rust's stdout is a `LineWriter`, so a final
/// unterminated line is exactly the thing that would be dropped — silently, and
/// only for the files that have one.
#[test]
fn a_file_with_no_final_newline_is_not_truncated() {
    const NO_FINAL_NEWLINE: &str = "@require: stdja\n\nlet two = 1 + 1";
    let r = fmt_with_stdin(&["-"], Some(NO_FINAL_NEWLINE));
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    // The formatter now TERMINATES the file, so the expectation ends in a
    // newline where it once did not. That is the parity work landing, not a
    // regression — checked before this line was changed: the output is the
    // input plus one `\n`, with `let two = 1 + 1` fully present.
    //
    // Keeping the whole tail in the assertion rather than relaxing it to
    // `contains` is the point. What this test exists for is a REAL bug: writing
    // through `std::process::exit` skips the runtime's end-of-`main` stdout
    // flush, so an unterminated last line was silently dropped. An assertion
    // that only checked the line appeared somewhere would still pass against a
    // truncated tail, which is exactly the failure being guarded.
    //
    // BUT NOTE WHAT THIS NO LONGER COVERS. Two mutants were run when the
    // expectation was updated: truncating a byte on this path FAILS the test,
    // so the truncation guard is real; removing `flush_stdout()` DOES NOT.
    // Rust's stdout is line-buffered, so now that the formatter always emits a
    // trailing newline the buffer drains on its own and the explicit flush is
    // no longer load-bearing here. The parity work silently took this test's
    // coverage of the flush bug away with it.
    //
    // The flush still matters where output does NOT end in a newline — the
    // `--check` diff path — so it must not be deleted on the strength of this
    // test passing without it.
    assert!(
        r.stdout.ends_with("let two = 1 + 1\n"),
        "the unterminated last line was lost or left unterminated: {:?}",
        r.stdout
    );
}

#[test]
fn stdin_that_does_not_lex_exits_six_and_prints_no_text() {
    let r = fmt_with_stdin(&["-"], Some(UNLEXABLE));
    assert_eq!(r.code, 6, "stderr: {}", r.stderr);
    assert_eq!(
        r.stdout, "",
        "a declined buffer must not be echoed: a shell redirect would then have \
         silently 'formatted' it"
    );
}

#[test]
fn stdin_under_check_is_clean_and_writes_no_text() {
    let r = fmt_with_stdin(&["--check", "-"], Some(CLEAN_V006));
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, "");
}

#[test]
fn an_unknown_path_is_a_filesystem_error() {
    let r = fmt(&["--check", "no/such/file.saty"]);
    assert_eq!(r.code, 5, "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("no/such/file.saty"),
        "the failing path should be named: {}",
        r.stderr
    );
}

/// There are TWO routes to exit `5`, and this is the second one.
///
/// A path named on the command line that cannot be stat'ed is refused before any
/// formatting starts (the test above); a file the DIRECTORY WALK found and then
/// could not read is refused per-file, through the run's tally. A mutation that
/// made the tally's `5` unreachable left the first test passing, which is how
/// this gap was found — so both routes have a test now.
#[test]
#[cfg(unix)]
fn an_unreadable_file_inside_a_walked_directory_is_a_filesystem_error() {
    use std::os::unix::fs::PermissionsExt as _;
    // Root reads a mode-000 file happily, so the fixture cannot be built there.
    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    let s = Scratch::new("unreadable");
    s.write("good.satyh", CLEAN_V006);
    let bad = s.write("bad.satyh", CLEAN_V006);
    std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000))
        .expect("make the fixture unreadable");
    let r = fmt(&["--check", s.0.to_str().expect("utf-8 path")]);
    // Restore before asserting, so a failure still lets `Scratch` clean up.
    let _ = std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o644));
    assert_eq!(r.code, 5, "stderr: {}", r.stderr);
    assert!(
        r.stderr.contains("bad.satyh"),
        "the unreadable file should be named: {}",
        r.stderr
    );
}

/// `geteuid(2)`, declared directly rather than by taking a `libc` dependency for
/// one call in one test.
#[cfg(unix)]
unsafe fn libc_geteuid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

/// `5` outranks `6`: a path that could not be read makes the whole answer
/// unreliable, so it is reported ahead of a decline.
#[test]
fn a_filesystem_error_outranks_a_decline() {
    let s = Scratch::new("io-over-decline");
    let bad = s.write("bad.satyh", UNLEXABLE);
    let r = fmt(&[bad.to_str().expect("utf-8 path"), "no/such/file.saty"]);
    assert_eq!(r.code, 5, "stderr: {}", r.stderr);
}

#[test]
fn mixing_stdin_with_a_path_is_a_usage_error() {
    let s = Scratch::new("stdin-mixed");
    let path = s.write("clean.satyh", CLEAN_V006);
    let r = fmt(&["-", path.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 2, "stderr: {}", r.stderr);
}

#[test]
fn check_and_emit_together_are_a_usage_error() {
    let s = Scratch::new("check-emit");
    let path = s.write("clean.satyh", CLEAN_V006);
    let r = fmt(&[
        "--check",
        "--emit",
        "files",
        path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(r.code, 2, "stderr: {}", r.stderr);
}

#[test]
fn an_unknown_lang_is_a_usage_error() {
    let s = Scratch::new("bad-lang");
    let path = s.write("clean.satyh", CLEAN_V006);
    let r = fmt(&[
        "--check",
        "--lang",
        "0.2",
        path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(r.code, 2, "stderr: {}", r.stderr);
}

#[test]
fn an_unknown_emit_destination_is_a_usage_error() {
    let r = fmt(&["--emit", "nowhere", "-"]);
    assert_eq!(r.code, 2, "stderr: {}", r.stderr);
}

/// `--lang` PINS the generation rather than merely preferring it, so a 0.1 file
/// declared as 0.0.6 is declined instead of quietly formatted as what it
/// actually is. The point is that the user's assertion about the file is
/// reported wrong, not worked around.
///
/// The `auto` half of it also pins the FALLBACK specifically: this fixture
/// carries no headers at all, so the sniffer answers 0.0.6, under which it does
/// not lex. Only the second attempt formats it. That is `format.rs:245-286`'s
/// "let the attempt be the test", and without it this file would be reported
/// unformattable when it formats perfectly.
#[test]
fn lang_pins_the_generation_rather_than_preferring_it() {
    let s = Scratch::new("lang-pins");
    // `A.B.C` is 0.1's `LongUpper` — a dotted path whose LAST segment is
    // capitalised. Under 0.0.6 that is a hard LEX error, not merely a parse
    // one, which is what makes it visible to a formatter that only lexes.
    // (`Foo.Bar.baz`, lower-case tail, lexes fine under both.)
    // `use A.B.C` rather than `val x = A.B.C`: the latter does not PARSE under
    // 0.1 either (a bare top-level `val` is not a 0.1 production), so it used
    // to reach the identity fallback and exit 0 for the wrong reason — the
    // fallback under test never had to work. A `use` header takes the same
    // `LongUpper`, and the whole file parses.
    let path = s.write("v01.saty", "use A.B.C\nmodule M = struct\n  val x = 1\nend\n");
    let auto = fmt(&["--check", path.to_str().expect("utf-8 path")]);
    assert_eq!(
        auto.code, 0,
        "the sniff-and-fall-back path should format it: {}",
        auto.stderr
    );
    let pinned = fmt(&[
        "--check",
        "--lang",
        "0.0",
        path.to_str().expect("utf-8 path"),
    ]);
    assert_eq!(
        pinned.code, 6,
        "pinned to 0.0.6 it does not lex, so it must decline: {}",
        pinned.stderr
    );
}

/// With no PATHS and no `Satyristes` above the working directory there is no
/// project, and that is a usage error rather than a walk of whatever directory
/// the user happened to be standing in. A formatter that rewrites files in
/// place must never guess at which files.
#[test]
fn no_paths_and_no_manifest_is_a_usage_error() {
    let s = Scratch::new("no-manifest");
    let path = s.write("clean.satyh", CLEAN_V006);
    let out = Command::new(bin())
        .arg("fmt")
        .arg("--check")
        .current_dir(&s.0)
        .output()
        .expect("run `rustyfi fmt` with no paths");
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("Satyristes"),
        "the reason should name the missing manifest: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // And nothing was formatted on the way to deciding that.
    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        CLEAN_V006
    );
}

/// With a `Satyristes`, the same bare invocation finds the project and formats
/// it — the other half of the previous test, so that "usage error" cannot be
/// how the default path set behaves generally.
#[test]
fn no_paths_with_a_manifest_formats_the_project() {
    let s = Scratch::new("manifest");
    s.write("Satyristes", "(version \"0.0.3\")\n");
    let path = s.write("clean.satyh", CLEAN_V006);
    let nested = s.0.join("sub");
    std::fs::create_dir_all(&nested).expect("create a subdirectory");
    std::fs::write(nested.join("nested.satyg"), CLEAN_V006).expect("write a nested fixture");
    // Not source, and must not be looked at: the walk is extension-filtered.
    s.write("README.md", "not SATySFi source, and `unclosed\n");
    let out = Command::new(bin())
        .arg("fmt")
        .arg("--check")
        .current_dir(&s.0)
        .output()
        .expect("run `rustyfi fmt` with no paths");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(0), "stderr: {stderr}");
    assert!(
        stderr.contains("2 file(s) checked"),
        "the walk should find both sources, recursively, and nothing else: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        CLEAN_V006
    );
}

/// A NAMED file is formatted whatever its extension — naming it is the user
/// saying it is SATySFi source. Only a directory walk is extension-filtered.
#[test]
fn a_named_file_is_formatted_whatever_its_extension() {
    let s = Scratch::new("odd-extension");
    let path = s.write("source.txt", CLEAN_V006);
    let r = fmt(&["--emit", "stdout", path.to_str().expect("utf-8 path")]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert_eq!(r.stdout, CLEAN_V006);
}

/// The corpus sweep, as a test rather than as a number in a commit message.
///
/// This is `docs/plans/formatter-cst/config.md` §4.3's "the honest way to answer
/// 'does the formatter damage the corpus?'", and it is worth having here because
/// it shares no line of code with `rustyfi-lsp`'s own tests: the same claim,
/// reached through the subcommand, the file walk and the exit code.
///
/// **It asserts the properties that hold at every slice, not a count.** A count
/// would have to be edited by every slice that widens what the formatter does,
/// and a test edited on every change asserts nothing. What must never move:
///
/// - **nothing is declined.** Every bundled file lexes, so every one of them has
///   a token stream to re-emit. A decline here means the formatter lost the
///   ability to read a file it could read before, which is the one regression
///   this project cannot ship.
/// - **nothing is unparsed.** All 209 bundled files parse, measured; a file
///   that stopped parsing would be laid out by nobody and merely
///   whitespace-tidied, which — before `CstOutcome` existed — was reported as
///   success. This is the assertion that makes the corpus sweep able to see
///   that at all.
/// - **the exit code is `0` or `1`.** Never `5` (the walk failed to read the
///   corpus), never `6`, never `7`.
/// - **the diff and the count agree.** A non-empty diff exactly when files need
///   reformatting: that equivalence is the whole of what makes `--check` a
///   measurement rather than a boolean, and it is the thing that would silently
///   break if the counter and the printer ever drifted apart.
/// - **the whole corpus was walked**, floored rather than pinned so that adding
///   a corpus file does not fail the test — but "everything was skipped" cannot
///   pass either, which is how a walk bug would otherwise look identical to a
///   clean corpus.
///
/// The measured numbers, for the record, since the point of the subcommand is to
/// produce them: under **slice 0** the sweep was 209 files, **0 needing
/// reformatting, 0 declined** — an independent corroboration of slice 0's
/// byte-for-byte no-op claim, since nothing in this path is shared with the
/// tests that assert it. Under **slice 1** (re-indentation, 0.0.6 only) it is
/// 209 files, **137 needing reformatting, 0 declined**: 29 of 30 in
/// `dist/packages`, 108 of 132 in `layout-tests/corpus`, and 0 of 47 in
/// `dist-v01/packages`, which slice 1 does not cover yet. With both builders
/// and the tier list wired it is 209 files, **132 needing reformatting, 0
/// declined, 0 unparsed** — and the last of those four is the measurement that
/// says the silent-failure defect was never reachable from the bundled corpus.
#[test]
fn the_bundled_corpus_is_walked_and_nothing_is_declined() {
    let dirs = [
        "lib-rustyfi/dist/packages",
        "lib-rustyfi/dist-v01/packages",
        "layout-tests/corpus",
    ];
    let root = repo_root();
    for d in dirs {
        assert!(
            root.join(d).is_dir(),
            "{d} is missing — is the checkout complete?"
        );
    }
    // Every corpus file's bytes, so that `--check`'s "writes nothing" promise is
    // asserted over the real corpus and not only over a scratch fixture.
    let before = corpus_bytes(&root, &dirs);
    assert!(
        before.len() >= 200,
        "expected the whole bundled corpus, found {} files",
        before.len()
    );

    let r = fmt(&["--check", dirs[0], dirs[1], dirs[2]]);
    assert!(
        r.code == 0 || r.code == 1,
        "a --check sweep of the corpus must be clean or merely unformatted, \
         never a decline (6) or a filesystem failure (5).\nexit: {}\nstderr: {}",
        r.code,
        r.stderr
    );
    let (checked, reformatted, declined, unparsed) = summary(&r.stderr);
    assert_eq!(
        declined, 0,
        "every bundled file lexes, so none of them may be declined.\nstderr: {}",
        r.stderr
    );
    assert_eq!(
        unparsed, 0,
        "all 209 bundled files parse, so none of them may fall through to the \
         whitespace-only tier — a file that does is formatted by nobody.\nstderr: {}",
        r.stderr
    );
    assert!(
        checked >= 200,
        "expected the whole bundled corpus, found {checked} files"
    );
    assert_eq!(
        checked,
        before.len(),
        "the subcommand's walk and this test's walk should find the same files"
    );
    // The equivalence, in both directions.
    assert_eq!(
        reformatted > 0,
        !r.stdout.is_empty(),
        "a non-empty diff exactly when files need reformatting.\nstderr: {}",
        r.stderr
    );
    assert_eq!(
        r.code == 1,
        reformatted > 0,
        "exit 1 exactly when files need reformatting.\nstderr: {}",
        r.stderr
    );
    assert_eq!(
        corpus_bytes(&root, &dirs),
        before,
        "--check must not have written a single corpus byte"
    );
}

/// `"209 file(s) checked: 132 need reformatting, 0 declined, 0 unparsed"` ->
/// `(209, 132, 0, 0)`. Parsed rather than substring-matched so the numbers can
/// be compared, which is what lets the test above assert an equivalence instead
/// of a fixed count.
fn summary(stderr: &str) -> (usize, usize, usize, usize) {
    let line = stderr
        .lines()
        .find(|l| l.contains(" file(s) checked"))
        .unwrap_or_else(|| panic!("no summary line in stderr:\n{stderr}"));
    let nums: Vec<usize> = line
        .split_whitespace()
        .filter_map(|w| w.parse().ok())
        .collect();
    assert_eq!(nums.len(), 4, "expected four counts in {line:?}");
    (nums[0], nums[1], nums[2], nums[3])
}

/// Every corpus source file's path and bytes, sorted — the before/after
/// comparison that makes "`--check` writes nothing" an assertion over the real
/// corpus.
fn corpus_bytes(root: &Path, dirs: &[&str]) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read a corpus directory").flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => walk(&path, out),
                Ok(t) if t.is_file() => {
                    if matches!(
                        path.extension().and_then(|e| e.to_str()),
                        Some("saty" | "satyh" | "satyg")
                    ) {
                        out.push(path);
                    }
                }
                _ => {}
            }
        }
    }
    let mut paths = Vec::new();
    for d in dirs {
        walk(&root.join(d), &mut paths);
    }
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(&p).expect("read a corpus file");
            (p, bytes)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The five formatting options: flags, environment variables, and precedence.
//
// The flag half is unit-tested in `fmt_opts` against the real clap definition.
// What can only be tested out here is the ENVIRONMENT half, because a unit
// test would have to mutate the process environment and race every other test
// in the binary. A subprocess has its own.
// ---------------------------------------------------------------------------

/// Like [`fmt`], with environment variables set for the child only.
fn fmt_env(args: &[&str], env: &[(&str, &str)], stdin: Option<&str>) -> Run {
    let mut cmd = Command::new(bin());
    cmd.arg("fmt")
        .args(args)
        .stdin(match stdin {
            Some(_) => Stdio::piped(),
            None => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(repo_root());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn `rustyfi fmt`");
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(text.as_bytes())
            .expect("write to `rustyfi fmt`");
    }
    let out = child.wait_with_output().expect("wait for `rustyfi fmt`");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A record wide enough to be flat at 100 columns and broken at 40, so one
/// document distinguishes every width setting below.
const WIDE: &str = "let f x = (| alpha = 1; beta = 2; gamma = 3; delta = 4; epsilon = 5 |)\n\
                    in\n\
                    document (|title = {P}; author = {r};|) '< +p { hi } >\n";

fn is_flat(text: &str) -> bool {
    text.contains("(| alpha = 1;") && text.contains("epsilon = 5 |)")
}

#[test]
fn the_width_environment_variable_is_read() {
    let run = fmt_env(
        &["--emit", "stdout", "-"],
        &[("RUSTYFI_FMT_MAX_WIDTH", "40")],
        Some(WIDE),
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        !is_flat(&run.stdout),
        "40 columns should break the record:\n{}",
        run.stdout
    );
    // And the control, so this cannot pass because the record breaks anyway.
    let plain = fmt_with_stdin(&["--emit", "stdout", "-"], Some(WIDE));
    assert!(
        is_flat(&plain.stdout),
        "the default 100 should keep it flat:\n{}",
        plain.stdout
    );
}

#[test]
fn a_flag_beats_the_environment_for_the_same_option() {
    let run = fmt_env(
        &["--emit", "stdout", "--max-width", "100", "-"],
        &[("RUSTYFI_FMT_MAX_WIDTH", "40")],
        Some(WIDE),
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        is_flat(&run.stdout),
        "the flag should win over the variable:\n{}",
        run.stdout
    );
}

#[test]
fn a_flag_and_a_variable_naming_different_options_both_apply() {
    // Width from the flag, indent from the environment: if resolution were
    // all-or-nothing per surface, one of these two assertions would fail.
    let run = fmt_env(
        &["--emit", "stdout", "--max-width", "40", "-"],
        &[("RUSTYFI_FMT_TAB_SPACES", "4")],
        Some(WIDE),
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(!is_flat(&run.stdout), "width applied:\n{}", run.stdout);
    assert!(
        run.stdout.contains("\n    alpha = 1;"),
        "tab_spaces 4 applied:\n{}",
        run.stdout
    );
}

#[test]
fn an_empty_variable_is_not_set() {
    // `RUSTYFI_FMT_MAX_WIDTH=` is how a shell script spells "leave it alone".
    // Parsing it as a number would exit 2 on a job that is asking for the
    // default.
    let run = fmt_env(
        &["--emit", "stdout", "-"],
        &[("RUSTYFI_FMT_MAX_WIDTH", "")],
        Some(WIDE),
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(is_flat(&run.stdout), "{}", run.stdout);
}

#[test]
fn a_bad_variable_is_a_usage_error_that_names_the_variable() {
    let run = fmt_env(
        &["--emit", "stdout", "-"],
        &[("RUSTYFI_FMT_MAX_WIDTH", "5")],
        Some(WIDE),
    );
    assert_eq!(run.code, 2, "out of range is a usage error");
    assert!(
        run.stderr.contains("RUSTYFI_FMT_MAX_WIDTH"),
        "the message must name the VARIABLE, since there is no flag on the \
         command line to look at: {}",
        run.stderr
    );
    assert!(run.stderr.contains("20..=1000"), "{}", run.stderr);
    assert!(
        run.stdout.is_empty(),
        "nothing is written on a usage error: {}",
        run.stdout
    );
}

#[test]
fn a_bad_variable_refuses_before_writing_anything() {
    // The dangerous shape: in-place mode, a tree of real files, and a bad
    // setting. Nothing may be rewritten -- so this must fail at resolution,
    // before the first file is touched.
    let dir = Scratch::new("bad-env-writes-nothing");
    let f = dir.write("a.saty", WIDE);
    let before = std::fs::read(&f).expect("read back");

    let run = fmt_env(
        &[f.to_str().expect("utf-8 path")],
        &[("RUSTYFI_FMT_TAB_SPACES", "0")],
        None,
    );
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert_eq!(
        std::fs::read(&f).expect("read after"),
        before,
        "the file must be untouched"
    );
}

#[test]
fn the_boolean_options_reach_the_formatter_from_both_surfaces() {
    // A `%` comment long enough to be reflowed, and prose enough to pass the
    // classifier. With wrapping off it must come back on one line.
    let src = "% This is an ordinary English sentence written as documentation \
               prose, and it is deliberately far longer than the hundred column \
               budget so that the comment wrapper has something to do with it.\n\
               let x = 1\n\
               in\n\
               document (|title = {P}; author = {r};|) '< +p { hi } >\n";

    let on = fmt_with_stdin(&["--emit", "stdout", "-"], Some(src));
    assert_eq!(on.code, 0, "{}", on.stderr);

    let off_flag = fmt_with_stdin(
        &["--emit", "stdout", "--wrap-comments", "false", "-"],
        Some(src),
    );
    assert_eq!(off_flag.code, 0, "{}", off_flag.stderr);

    let off_env = fmt_env(
        &["--emit", "stdout", "-"],
        &[("RUSTYFI_FMT_WRAP_COMMENTS", "false")],
        Some(src),
    );
    assert_eq!(off_env.code, 0, "{}", off_env.stderr);

    // The two "off" surfaces must agree with each other...
    assert_eq!(
        off_flag.stdout, off_env.stdout,
        "the flag and the variable must mean the same thing"
    );
    // ...and must differ from "on", or this test proves nothing about either.
    assert_ne!(
        on.stdout, off_flag.stdout,
        "wrap_comments must actually change this document, otherwise the \
         fixture is wrong and the assertions above are vacuous"
    );
}
