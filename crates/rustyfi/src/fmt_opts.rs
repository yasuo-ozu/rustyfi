//! Where `rustyfi fmt`'s five formatting options come from.
//!
//! # Precedence
//!
//! ```text
//!   CLI flag  >  environment variable  >  CstOptions::default()
//! ```
//!
//! Per option, not per surface: `--max-width 90` with
//! `RUSTYFI_FMT_TAB_SPACES=4` set uses both. An option nobody names keeps its
//! default, so the whole mechanism is invisible to a run that passes nothing.
//!
//! # Why flags AND an environment variable
//!
//! They are for different callers and neither substitutes for the other. A
//! flag is what a person types and what `--help` can show; an environment
//! variable is what a CI job, a pre-commit hook or an editor integration sets
//! once for every invocation underneath it, including the ones it does not
//! construct the argv for. rustfmt reaches the same two places with
//! `--config` and `RUSTFMT` respectively.
//!
//! # Why not `--config-path` or a config FILE
//!
//! Still deliberately absent, for the reason [`crate::fmt`]'s module doc
//! gives: no config file format is specified, and shipping a flag that reads
//! a format nobody has written down would be worse than not having it. These
//! are per-option overrides, which need no format at all — the option's name
//! IS the interface, and it is the same name in all three places (the TOML
//! key the playground accepts, the flag, and the tail of the environment
//! variable). When the loader lands it is `--config-path`, and it slots in
//! BELOW these: an explicit `--max-width` should still beat a file.
//!
//! # Range errors REFUSE here, and CLAMP in the playground
//!
//! A deliberate divergence, not drift. The playground's parser clamps because
//! its caller is a settings panel a user drags: a value out of range is a
//! gesture that overshot, and the useful response is the nearest legal one.
//! Here the caller typed it, and a `--max-width 5` that silently became 20
//! would reformat their whole tree at a width they never asked for. So this
//! refuses, with the range in the message, and the run exits 2 having written
//! nothing.

use clap::ArgMatches;
use rustyfi_lsp::CstOptions;

/// Every option's identity, in one place, so the three spellings cannot drift
/// apart: the flag, the environment variable, and the TOML key the playground
/// and any future config file use.
///
/// The flag is the TOML key with `_` -> `-`; the variable is `RUSTYFI_FMT_`
/// plus the key uppercased. Both derivations are mechanical, and
/// `spellings_are_mechanical` in the tests below asserts they stay that way
/// rather than trusting this comment.
pub(crate) const OPTIONS: &[(&str, &str, &str)] = &[
    ("max_width", "max-width", "RUSTYFI_FMT_MAX_WIDTH"),
    ("tab_spaces", "tab-spaces", "RUSTYFI_FMT_TAB_SPACES"),
    (
        "max_blank_lines",
        "max-blank-lines",
        "RUSTYFI_FMT_MAX_BLANK_LINES",
    ),
    ("wrap_comments", "wrap-comments", "RUSTYFI_FMT_WRAP_COMMENTS"),
    (
        "wrap_inline_text",
        "wrap-inline-text",
        "RUSTYFI_FMT_WRAP_INLINE_TEXT",
    ),
];

/// Accepted ranges for the three numeric options, shared with the error
/// messages so a refusal cannot quote a bound the check does not apply.
///
/// The same bounds the playground's `parse_format_config` clamps to. A width
/// under 20 cannot hold an indented construct at all; over 1000 is past any
/// display. `tab_spaces` 0 would make nesting invisible, and `max_blank_lines`
/// 0 is meaningful (collapse every blank line) so only that one starts at 0.
const MAX_WIDTH: (usize, usize) = (20, 1000);
const TAB_SPACES: (usize, usize) = (1, 16);
const MAX_BLANK_LINES: (usize, usize) = (0, 32);

/// The flag and variable spellings for one option, looked up by its key.
///
/// [`resolve`] goes through here rather than repeating the strings, so
/// [`OPTIONS`] is the single source of truth at RUNTIME and not merely a table
/// the tests check against itself. Renaming a flag in one place now breaks the
/// lookup instead of silently giving that option two spellings.
///
/// Panics on a key that is not in the table. Unreachable from outside: the
/// only callers are the five literal keys in [`resolve`], and
/// `every_key_resolves` walks the table to prove each one arrives.
fn spec(key: &str) -> (&'static str, &'static str) {
    let (_, flag, env) = OPTIONS
        .iter()
        .find(|(k, _, _)| *k == key)
        .unwrap_or_else(|| panic!("`{key}` is not in OPTIONS"));
    (flag, env)
}

/// Which surface a value came from, so a refusal names the thing the caller
/// has to go and change.
///
/// Worth the enum: told only "max_width wants 20..=1000", someone with
/// `RUSTYFI_FMT_MAX_WIDTH` exported in their shell profile would look at their
/// command line, find no `--max-width` on it, and have nowhere to go next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Flag(&'static str),
    Env(&'static str),
}

impl Source {
    fn describe(self) -> String {
        match self {
            Source::Flag(f) => format!("--{f}"),
            Source::Env(v) => format!("{v} (environment)"),
        }
    }
}

/// The flag if it was given, else the variable if it is set and non-empty,
/// else nothing.
///
/// An empty variable counts as unset. `RUSTYFI_FMT_MAX_WIDTH=` in a CI script
/// is how a shell spells "I am not setting this", and refusing it as a
/// malformed number would break a job that is asking for the default.
fn lookup(m: &ArgMatches, flag: &'static str, env: &'static str) -> Option<(String, Source)> {
    if let Some(v) = m.get_one::<String>(flag) {
        return Some((v.clone(), Source::Flag(flag)));
    }
    match std::env::var(env) {
        Ok(v) if !v.trim().is_empty() => Some((v.trim().to_string(), Source::Env(env))),
        _ => None,
    }
}

fn number(
    m: &ArgMatches,
    flag: &'static str,
    env: &'static str,
    (lo, hi): (usize, usize),
    current: usize,
) -> Result<usize, String> {
    let Some((raw, src)) = lookup(m, flag, env) else {
        return Ok(current);
    };
    let n: usize = raw
        .parse()
        .map_err(|_| format!("{}: wants a whole number, found `{raw}`", src.describe()))?;
    if n < lo || n > hi {
        return Err(format!(
            "{}: {n} is out of range; wants {lo}..={hi}",
            src.describe()
        ));
    }
    Ok(n)
}

fn boolean(
    m: &ArgMatches,
    flag: &'static str,
    env: &'static str,
    current: bool,
) -> Result<bool, String> {
    let Some((raw, src)) = lookup(m, flag, env) else {
        return Ok(current);
    };
    match raw.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!(
            "{}: wants `true` or `false`, found `{other}`",
            src.describe()
        )),
    }
}

/// Fold the flags and the environment onto [`CstOptions::default`].
///
/// Returns the message for a usage error (exit 2) rather than printing it, so
/// the caller keeps one place where `fmt`'s diagnostics are written.
pub(crate) fn resolve(m: &ArgMatches) -> Result<CstOptions, String> {
    let d = CstOptions::default();
    let (wf, we) = spec("max_width");
    let (tf, te) = spec("tab_spaces");
    let (bf, be) = spec("max_blank_lines");
    let (cf, ce) = spec("wrap_comments");
    let (if_, ie) = spec("wrap_inline_text");
    Ok(CstOptions {
        max_width: number(m, wf, we, MAX_WIDTH, d.max_width)?,
        tab_spaces: number(m, tf, te, TAB_SPACES, d.tab_spaces)?,
        max_blank_lines: number(m, bf, be, MAX_BLANK_LINES, d.max_blank_lines)?,
        wrap_comments: boolean(m, cf, ce, d.wrap_comments)?,
        wrap_inline_text: boolean(m, if_, ie, d.wrap_inline_text)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse an argv into `fmt`'s matches, so these tests go through the real
    /// clap definition rather than a hand-built `ArgMatches` that could drift
    /// from it.
    ///
    /// The CLI is a **multicall**: argv[0] selects a personality, which is
    /// itself a subcommand, and `fmt` hangs off the `rustyfi` one. So the
    /// matches nest twice and the obvious one-level `subcommand_matches("fmt")`
    /// panics with "`fmt` is not a name of a subcommand" -- which is how this
    /// helper got written wrong the first time.
    fn fmt_matches(args: &[&str]) -> ArgMatches {
        let mut argv = vec!["rustyfi", "fmt"];
        argv.extend_from_slice(args);
        crate::dispatch::build_cli()
            .try_get_matches_from(argv)
            .expect("argv should parse")
            .subcommand_matches("rustyfi")
            .expect("rustyfi personality")
            .subcommand_matches("fmt")
            .expect("fmt subcommand")
            .clone()
    }

    /// The three spellings of an option are derivations of one name, and this
    /// is what keeps them that way. Rename a flag without renaming its key and
    /// this fails; the alternative is three lists that agree only by habit.
    #[test]
    fn spellings_are_mechanical() {
        for (key, flag, env) in OPTIONS {
            assert_eq!(*flag, key.replace('_', "-"), "flag for {key}");
            assert_eq!(
                *env,
                format!("RUSTYFI_FMT_{}", key.to_uppercase()),
                "env var for {key}"
            );
        }
    }

    /// Every option in the table is really a flag on the command. Catches an
    /// option added to `resolve` and to the table but never given a flag,
    /// which would otherwise only show up as "it silently ignores me".
    #[test]
    fn every_option_has_a_flag() {
        let cmd = crate::dispatch::build_cli();
        let fmt = cmd
            .get_subcommands()
            .find(|c| c.get_name() == "rustyfi")
            .expect("rustyfi personality exists")
            .get_subcommands()
            .find(|c| c.get_name() == "fmt")
            .expect("fmt subcommand exists");
        for (key, flag, _) in OPTIONS {
            assert!(
                fmt.get_arguments().any(|a| a.get_id() == *flag),
                "no --{flag} for {key}"
            );
        }
    }

    /// And that the count matches `CstOptions`' own field count, so an option
    /// added to the struct without a flag fails here rather than being
    /// unreachable from the CLI forever. Destructured rather than counted, so
    /// the compiler is what notices the new field.
    #[test]
    fn the_table_covers_every_field() {
        let CstOptions {
            max_width: _,
            tab_spaces: _,
            max_blank_lines: _,
            wrap_comments: _,
            wrap_inline_text: _,
        } = CstOptions::default();
        assert_eq!(OPTIONS.len(), 5);
    }

    /// `spec` panics on an unknown key, so this is what proves the five
    /// literals in `resolve` are all really in the table.
    #[test]
    fn every_key_resolves() {
        for (key, flag, env) in OPTIONS {
            assert_eq!(spec(key), (*flag, *env));
        }
        // And that resolve's own keys are among them -- a typo there would
        // panic at run time, which is exactly what this catches at test time.
        for key in [
            "max_width",
            "tab_spaces",
            "max_blank_lines",
            "wrap_comments",
            "wrap_inline_text",
        ] {
            let _ = spec(key);
        }
    }

    #[test]
    fn no_flags_is_exactly_the_default() {
        let got = resolve(&fmt_matches(&[])).expect("no flags resolves");
        let d = CstOptions::default();
        assert_eq!(got.max_width, d.max_width);
        assert_eq!(got.tab_spaces, d.tab_spaces);
        assert_eq!(got.max_blank_lines, d.max_blank_lines);
        assert_eq!(got.wrap_comments, d.wrap_comments);
        assert_eq!(got.wrap_inline_text, d.wrap_inline_text);
    }

    #[test]
    fn each_flag_reaches_its_own_field_and_no_other() {
        let o = resolve(&fmt_matches(&["--max-width", "60"])).unwrap();
        assert_eq!(o.max_width, 60);
        assert_eq!(o.tab_spaces, CstOptions::default().tab_spaces);

        let o = resolve(&fmt_matches(&["--tab-spaces", "4"])).unwrap();
        assert_eq!(o.tab_spaces, 4);
        assert_eq!(o.max_width, CstOptions::default().max_width);

        let o = resolve(&fmt_matches(&["--max-blank-lines", "0"])).unwrap();
        assert_eq!(o.max_blank_lines, 0);

        let o = resolve(&fmt_matches(&["--wrap-comments", "false"])).unwrap();
        assert!(!o.wrap_comments);
        assert!(o.wrap_inline_text, "the other bool is untouched");

        let o = resolve(&fmt_matches(&["--wrap-inline-text", "false"])).unwrap();
        assert!(!o.wrap_inline_text);
        assert!(o.wrap_comments);
    }

    /// `--wrap-comments` with no value means true — so that turning an option
    /// ON reads like a flag, while turning it off still needs the word.
    #[test]
    fn a_bare_bool_flag_means_true() {
        let o = resolve(&fmt_matches(&["--wrap-comments"])).unwrap();
        assert!(o.wrap_comments);
        // And it is not merely inheriting the default: prove the same argv
        // shape carries `false` too.
        let o = resolve(&fmt_matches(&["--wrap-comments=false"])).unwrap();
        assert!(!o.wrap_comments);
    }

    /// Out of range REFUSES rather than clamping, and the message carries the
    /// bound. See the module doc for why this differs from the playground.
    #[test]
    fn a_width_out_of_range_is_refused_with_its_bound() {
        let e = resolve(&fmt_matches(&["--max-width", "5"])).unwrap_err();
        assert!(e.contains("--max-width"), "{e}");
        assert!(e.contains("20..=1000"), "{e}");
        assert!(e.contains('5'), "{e}");

        let e = resolve(&fmt_matches(&["--max-width", "1001"])).unwrap_err();
        assert!(e.contains("20..=1000"), "{e}");

        // The bounds themselves are legal -- an off-by-one in the check would
        // otherwise pass every test above.
        assert_eq!(resolve(&fmt_matches(&["--max-width", "20"])).unwrap().max_width, 20);
        assert_eq!(
            resolve(&fmt_matches(&["--max-width", "1000"])).unwrap().max_width,
            1000
        );
    }

    #[test]
    fn tab_spaces_and_blank_lines_have_their_own_bounds() {
        // `tab_spaces` starts at 1: 0 would make nesting invisible.
        assert!(resolve(&fmt_matches(&["--tab-spaces", "0"])).is_err());
        assert_eq!(resolve(&fmt_matches(&["--tab-spaces", "1"])).unwrap().tab_spaces, 1);
        assert!(resolve(&fmt_matches(&["--tab-spaces", "17"])).is_err());

        // `max_blank_lines` starts at 0, which MEANS something: drop every
        // blank line. So 0 must be accepted here and refused above.
        assert_eq!(
            resolve(&fmt_matches(&["--max-blank-lines", "0"]))
                .unwrap()
                .max_blank_lines,
            0
        );
        assert!(resolve(&fmt_matches(&["--max-blank-lines", "33"])).is_err());
    }

    #[test]
    fn a_non_numeric_width_is_refused_by_name() {
        let e = resolve(&fmt_matches(&["--max-width", "wide"])).unwrap_err();
        assert!(e.contains("--max-width"), "{e}");
        assert!(e.contains("whole number"), "{e}");
        assert!(e.contains("wide"), "{e}");
    }

    /// A bad boolean never reaches `boolean()` from the flag surface — clap's
    /// `value_parser` refuses it first. Pinned because it is the reason the
    /// "wants true or false" message is only reachable from the environment,
    /// and someone deleting that parser should be told what they lost.
    #[test]
    fn clap_refuses_a_bad_bool_before_we_do() {
        let bad = crate::dispatch::build_cli()
            .try_get_matches_from(["rustyfi", "fmt", "--wrap-comments", "yes"]);
        // (multicall: this argv is personality `rustyfi` then subcommand `fmt`)
        assert!(bad.is_err(), "clap should reject a non-bool");
    }
}
