//! The clap *builder*-style dispatch tree. Builder rather than
//! derive because `Command::multicall(true)` composes with builder
//! `Command`s but not with the derive macro. The tree:
//!
//! ```text
//! rustyfi                (multicall root)
//! ├── rustyfi  <compile args> [satyrographos …] [multicall …]
//! ├── rustyfi       <compile args>
//! └── satyrographos <install|uninstall|list|status>
//! ```
//!
//! `multicall(true)` selects the top-level subcommand from `argv[0]`'s
//! basename, so the same binary is `rustyfi` or `satyrographos` depending on
//! the name it is invoked under (see the alias-install helper). Under
//! `rustyfi` the compile args and the
//! `satyrographos`/`multicall` subcommand trees coexist via
//! `args_conflicts_with_subcommands` + `subcommand_negates_reqs`, so the
//! required positional `input` is only demanded when no subcommand is used.
//!
//! ## Leading global flags (`rustyfi --config F install NAME`)
//!
//! `args_conflicts_with_subcommands` is exactly what makes the positional
//! `input` and the subcommand tree coexist on one `Command` — but clap
//! implements it with a single per-parse `valid_arg_found` bit that does not
//! distinguish "a flag matched" from "a positional matched": once ANY
//! argument matches at this level, clap stops treating later bare words as
//! subcommand names at all (`clap_builder`'s `Parser::possible_subcommand`).
//! So `rustyfi --config F install NAME` matches `--config` on the `rustyfi`
//! node, and from that point on "install" can no longer start a subcommand —
//! it instead fills the compile positional `input`, and `NAME` (or a
//! subcommand-only flag like `--dest`) then fails to parse against the
//! compile arg set.
//!
//! Turning `args_conflicts_with_subcommands` off doesn't work either: without
//! it, clap will ALSO treat a bare word as a subcommand name AFTER an
//! unrelated word has already filled `input` (that's the documented default,
//! "arguments between subcommands") — which resurrects the exact nesting the
//! `package_commands_are_top_level` test deliberately closed off
//! (`rustyfi satyrographos list --dest X` would start "succeeding" again, by
//! filling `input` with `"satyrographos"` and then dispatching `list`
//! anyway). There is no third setting that means "flags before the
//! subcommand are fine, bare words are not" — clap's model genuinely has
//! nothing finer-grained than the one bit.
//!
//! [`get_matches`] is the fix: parse `argv` once, normally. If that FAILS, it
//! looks for a subcommand name anywhere in the tail, moves everything before
//! it to just after it, and retries — i.e. rewrites
//! `rustyfi --config F install NAME` to `rustyfi install --config F NAME`
//! and parses that instead. This is deliberately a pre-pass, not a clap
//! setting, because no clap setting expresses the distinction above.
//!
//! The tail walk is value-aware (`find_subcommand_split`): a flag that TAKES
//! a value has its value skipped rather than considered as a candidate split
//! point, so `rustyfi --lib-root search install foo` is not split at
//! `search` (`--lib-root`'s value) — it hoists at `install`, the real
//! subcommand. The set of value-taking flags is read off the actual `Command`
//! tree (`value_taking_flag_spellings`, via `get_num_args`/`ArgAction`), not
//! hand-maintained, so a flag added later stays covered automatically.
//!
//! A second, narrower case: the first parse can also SUCCEED with the wrong
//! meaning instead of failing outright, when nothing follows the swallowed
//! word to expose the mistake — `rustyfi --config F install` (no PATH) parses
//! as compile mode with `input = "install"` (a document literally named
//! `install`) rather than the `install` subcommand, because `--config`
//! already set clap's "an arg matched" bit before `install` was reached. This
//! is the SAME ambiguity `rustyfi install` (no leading flag) already resolves
//! in the subcommand's favor — a leading flag must not flip that resolution —
//! so `get_matches` also re-checks a successful compile-mode parse whose
//! `input` string is *itself* exactly a subcommand name, and prefers the
//! hoisted reading when that also parses. An explicit path (`./install`)
//! escapes this by no longer string-matching a bare name.
//!
//! Both retries are gated on the plain parse already being unusable (an
//! error, or this specific swallowed-subcommand shape) so they can never
//! change the outcome of anything else that parses correctly — compile mode
//! on a real document is untouched, byte for byte.
//!
//! A literal `--` in the tail before any candidate word disables hoisting
//! entirely, since at that point the user has explicitly said "nothing after
//! this is a flag".
//!
//! Remaining edge case: the value-taking set is a union across the WHOLE
//! command tree (every personality, every subcommand), because the walk runs
//! before we know which subcommand — and therefore which node's arg set —
//! actually applies. Today no flag spelling is declared value-taking in one
//! place and boolean in another, so this union is exact; a future flag that
//! reused a spelling with different arity across subcommands would get
//! whichever arity the walk saw ANYWHERE in the tree, everywhere. A flag not
//! in the tree at all (a typo) isn't recognized as value-taking either, so
//! its value could still be mistaken for a split. Both are bounded: the
//! rewrite is only trusted when it goes on to parse cleanly (or resolves to a
//! MORE specific `--help`/`--version`), so the worst case is the user's
//! ORIGINAL error reported unchanged — never a silently wrong compile or
//! install.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;

use clap::{value_parser, Arg, ArgAction, ArgGroup, ArgMatches, Command};

/// Parse `std::env::args_os()`, working around the leading-global-flag
/// limitation documented above. Behaves exactly like `build_cli().get_matches()`
/// when the argv already parses (the overwhelmingly common case, and the
/// entirety of compile mode); only reaches for the hoist-and-retry fallback
/// when the direct parse fails.
pub fn get_matches() -> ArgMatches {
    let argv: Vec<OsString> = std::env::args_os().collect();
    match build_cli().try_get_matches_from(argv.clone()) {
        Ok(m) => {
            // The parse succeeded, but may have swallowed a subcommand name
            // into the compile positional `input` because a leading flag
            // already matched first (module doc, second case). Only retry
            // when that specific, narrow shape is detected; any other
            // successful parse is returned exactly as clap produced it.
            if compile_mode_input(&m).is_some_and(|input| is_hoistable_name(input)) {
                if let Some(reordered) = hoist_leading_subcommand(&argv) {
                    if let Ok(m2) = build_cli().try_get_matches_from(reordered) {
                        return m2;
                    }
                }
            }
            m
        }
        Err(original_err) => {
            if let Some(reordered) = hoist_leading_subcommand(&argv) {
                match build_cli().try_get_matches_from(reordered) {
                    Ok(m) => return m,
                    // The rewrite still fails, but more specifically as a
                    // `--help`/`--version` request scoped to the subcommand
                    // the user actually meant (e.g. `rustyfi --config F
                    // install --help` should show `install`'s help, not the
                    // compile personality's) — strictly more useful than the
                    // original error, so prefer it.
                    Err(e)
                        if matches!(
                            e.kind(),
                            clap::error::ErrorKind::DisplayHelp
                                | clap::error::ErrorKind::DisplayVersion
                                | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                        ) =>
                    {
                        e.exit()
                    }
                    // Any other failure means the hoist guessed wrong (or the
                    // command was simply broken to start with); the original
                    // error is at least about what the user actually typed.
                    Err(_) => {}
                }
            }
            // Same behavior as `Command::get_matches`: print and exit with
            // clap's own formatting and exit code.
            original_err.exit()
        }
    }
}

/// The compile positional `input`'s value, but only when compile mode is
/// actually what was parsed (the `rustyfi` personality, no subcommand
/// dispatched under it) — `None` for the `satyrographos` personality (which
/// has no compile mode at all) and for a real subcommand dispatch.
fn compile_mode_input(m: &ArgMatches) -> Option<&PathBuf> {
    let (name, inner) = m.subcommand()?;
    if name != "rustyfi" || inner.subcommand().is_some() {
        return None;
    }
    inner.get_one::<PathBuf>("input")
}

/// Whether `input` is, verbatim, the name of a subcommand reachable under
/// either personality — the telltale of the "successful but wrong" case
/// (module doc). An explicit path (`./install`) never matches, by design.
fn is_hoistable_name(input: &PathBuf) -> bool {
    input
        .to_str()
        .is_some_and(|s| hoistable_subcommand_names().iter().any(|n| n == s))
}

/// The set of words that could plausibly be "the subcommand", read off the
/// real tree rather than hand-duplicated, so a subcommand added later is
/// covered automatically. `install`/`uninstall`/`build`/`list`/`status`/
/// `search`/`update` are shared by both personalities; `multicall`/`man` are
/// `rustyfi`-only, and included for the same reason: they sit on the same
/// conflicted `Command` node.
fn hoistable_subcommand_names() -> Vec<String> {
    let cli = build_cli();
    let mut names: Vec<String> = Vec::new();
    for personality in ["rustyfi", "satyrographos"] {
        if let Some(p) = cli.find_subcommand(personality) {
            names.extend(p.get_subcommands().map(|sc| sc.get_name().to_string()));
        }
    }
    names
}

/// If some tail token (after `argv[0]`, the multicall personality selector)
/// is exactly the name of a subcommand reachable under either personality,
/// move every token before it to just after it and return the rewritten
/// argv. Returns `None` when there is nothing to hoist — no candidate word
/// found, the first tail token already IS one (nothing precedes it to move),
/// or a literal `--` appears first (see the module doc's edge-case note).
fn hoist_leading_subcommand(argv: &[OsString]) -> Option<Vec<OsString>> {
    let names = hoistable_subcommand_names();
    let value_flags = value_taking_flag_spellings();

    let rest = argv.get(1..)?;
    let split = find_subcommand_split(rest, &names, &value_flags)?;
    if split == 0 {
        // Already `SUBCOMMAND ...`; there is nothing before it to hoist, and
        // if this shape still fails to parse, reordering cannot help.
        return None;
    }

    let mut out = Vec::with_capacity(argv.len());
    out.push(argv[0].clone());
    out.push(rest[split].clone());
    out.extend_from_slice(&rest[..split]);
    out.extend_from_slice(&rest[split + 1..]);
    Some(out)
}

/// Walk `rest` (argv minus the multicall selector) left to right for the
/// first token that is exactly one of `names`, treating a value-taking
/// flag's value as opaque — never itself a candidate, and never even
/// inspected for being `--` — so a flag value that happens to collide with a
/// subcommand name (`--lib-root search install foo`, `search` being
/// `--lib-root`'s value) is never mistaken for the split. `--flag value` is
/// two tokens, so the value token after a bare `--flag`/`-f` is skipped;
/// `--flag=value` is already one token, so nothing extra is skipped. A
/// literal `--` reached before any candidate stops the walk with no split
/// (see the module doc's edge-case note).
fn find_subcommand_split(
    rest: &[OsString],
    names: &[String],
    value_flags: &HashSet<String>,
) -> Option<usize> {
    let mut i = 0;
    while i < rest.len() {
        let Some(tok) = rest[i].to_str() else {
            i += 1;
            continue;
        };
        if tok == "--" {
            return None;
        }
        if tok.starts_with('-') {
            if !tok.contains('=') && value_flags.contains(tok) {
                i += 2; // this flag AND its separate value token
            } else {
                i += 1;
            }
            continue;
        }
        if names.iter().any(|n| n == tok) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The flag spellings (`--long`, `-s`) that consume the FOLLOWING argv token
/// as their value, read off every `Arg` in the whole command tree (both
/// personalities, every subcommand, recursively) rather than hand-maintained
/// — a flag added anywhere stays covered automatically. `Command::build`
/// performs the normalization clap otherwise defers until a real parse
/// (filling in each arg's default `num_args` from its `ArgAction`), which
/// introspection needs done up front.
fn value_taking_flag_spellings() -> HashSet<String> {
    let mut cli = build_cli();
    cli.build();
    let mut out = HashSet::new();
    collect_value_taking_flags(&cli, &mut out);
    out
}

fn collect_value_taking_flags(cmd: &Command, out: &mut HashSet<String>) {
    for arg in cmd.get_arguments() {
        if arg.is_positional() {
            continue;
        }
        if !arg.get_num_args().is_some_and(|r| r.takes_values()) {
            continue;
        }
        if let Some(long) = arg.get_long() {
            out.insert(format!("--{long}"));
        }
        if let Some(short) = arg.get_short() {
            out.insert(format!("-{short}"));
        }
    }
    for sub in cmd.get_subcommands() {
        collect_value_taking_flags(sub, out);
    }
}

pub fn build_cli() -> Command {
    Command::new("rustyfi")
        .multicall(true)
        .subcommand_required(true)
        .arg_required_else_help(true)
        // `rustyfi` personality: compiler + nested package manager — one node
        // carrying the compile args AND the subcommand trees.
        .subcommand(
            package_subcommands(compile_command("rustyfi"))
                .subcommand(multicall_command())
                .subcommand(man_command())
                .args_conflicts_with_subcommands(true)
                .subcommand_negates_reqs(true),
        )
        // `satyrographos` personality: the same package commands under the
        // name real Satyrographos users type. The compiler personality carries
        // them directly, so this is an alias, not a separate tree.
        .subcommand(satyrographos_command())
}

fn compile_command(name: &'static str) -> Command {
    Command::new(name)
        .version(env!("CARGO_PKG_VERSION"))
        .about("Compile a SATySFi (.saty) document to PDF.")
        .arg(
            Arg::new("input")
                .help("Input .saty file.")
                .required(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .help("Output PDF path (defaults to the input with a .pdf extension).")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("lib_root")
                .long("lib-root")
                .help(
                    "Library root for `@require:` resolution (packages under \
                     <lib-root>/dist/packages/). Falls back to $RUSTYFI_LIB_ROOT, \
                     then to every root found from the document's directory, nearest first: \
                     a `lib-rustyfi/` above it, a `.rustyfi/` beside a Satyristes, ~/.local/lib/rustyfi, /usr/local/lib/rustyfi, /usr/lib/rustyfi.",
                )
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("lang")
                .long("lang")
                .value_name("VERSION")
                .help(
                    "SATySFi language generation: 0.0 (default) or 0.1. \
                     When omitted, a 0.1-style `use` header is auto-detected. \
                     Same spelling as `install --lang`.",
                ),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .value_name("FORMAT")
                .help(
                    "Output format: pdf.",
                )
                .value_parser(["pdf"])
                .default_value("pdf"),
        )
        .arg(
            Arg::new("deps")
                .long("deps")
                .value_name("FILE")
                .help(
                    "Pin Envelopes packaging mode (Axis B) and consume a \
                     pre-resolved rustyfi-deps.yaml at FILE. Requires \
                     --lang 0.1. Ld3a: local `use … of` dependencies \
                     resolve; supplying a deps FILE errors (its consumption is \
                     Ld3b). Without this flag, a `use` header still auto-pins \
                     Envelopes mode.",
                )
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("no_cache")
                .long("no-cache")
                .help(
                    "Bypass the content-addressed compile cache (neither read \
                     nor write it). Caching is on by default and makes an \
                     unchanged recompile near-instant.",
                )
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("timing")
                .long("timing")
                .help(
                    "Print a per-phase wall-clock breakdown to stderr \
                     (load / elaborate / typecheck / eval trials / render), for \
                     performance evaluation. Implies --no-cache so every phase \
                     actually runs (it does NOT imply --no-aux: an aux file \
                     skips no phase, it only changes the fixpoint trial count).",
                )
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("no_aux")
                .long("no-aux")
                .help(
                    "Do not read or write the auxiliary cross-reference file.                      By default a compile seeds its cross-reference fixpoint                      from `<doc>.satysfi-aux` (same name and JSON format                      upstream SATySFi uses, so the two interoperate) and                      rewrites it afterwards, which lets a forward reference                      resolve on the first trial instead of forcing a second.                      Output is identical either way. Implied by --timing.",
                )
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("aux_file")
                .long("aux-file")
                .value_name("FILE")
                .help(
                    "Override the auxiliary cross-reference file's path                      (default: the input document's path with its extension                      replaced by `.satysfi-aux`).",
                )
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("cache_dir")
                .long("cache-dir")
                .value_name("DIR")
                .help(
                    "Override the compile-cache directory (default: \
                     $XDG_CACHE_HOME/rustyfi, then ~/.cache/rustyfi, \
                     then a temp-dir fallback).",
                )
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("font_dir")
                .long("font-dir")
                .value_name("DIR")
                .help(
                    "Font root for dist/hash/fonts.satysfi-hash (+ \
                     default-font.satysfi-hash) discovery — same layout \
                     convention as --lib-root's dist/packages/. Falls back to \
                     $RUSTYFI_FONT_DIR, then to --lib-root itself. With no font \
                     configured anywhere, text is set in the built-in base-14 \
                     fonts (this is the milestone-1 default and stays exactly \
                     as before).",
                )
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("font")
                .long("font")
                .value_name("FILE")
                .help(
                    "Use this TrueType/OpenType file as the regular face \
                     directly, bypassing fonts.satysfi-hash discovery (a \
                     config-less one-off). Takes precedence over --font-dir \
                     and $RUSTYFI_FONT_DIR.",
                )
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("font_bold")
                .long("font-bold")
                .value_name("FILE")
                .help("Bold face for --font (defaults to --font itself when omitted).")
                .value_parser(value_parser!(PathBuf))
                .requires("font"),
        )
        .arg(
            Arg::new("font_oblique")
                .long("font-oblique")
                .value_name("FILE")
                .help(
                    "Oblique/italic face for --font (defaults to --font itself \
                     when omitted).",
                )
                .value_parser(value_parser!(PathBuf))
                .requires("font"),
        )
}

/// The shared `--registry URL` flag, attached to the
/// registry-aware subcommands (`install`, `search`, `update`). Overrides
/// `$RUSTYFI_REGISTRY` and any `Satyristes` `[registry]` url.
fn registry_flag(cmd: Command) -> Command {
    cmd.arg(
        Arg::new("registry")
            .long("registry")
            .value_name("URL")
            .help(
                "Registry index URL (a git repo or a local/`file://` directory index). \
                 Overrides $RUSTYFI_REGISTRY and Satyristes's [registry] url.",
            ),
    )
    .arg(
        Arg::new("offline")
            .long("offline")
            .help(
                "Never make a network request (saphe phase 7d slice S2): resolve \
                 entirely from the archive cache and (for the index) an already-cloned \
                 registry, erroring cleanly if something needed is not cached. Same \
                 effect as $RUSTYFI_OFFLINE=1.",
            )
            .action(ArgAction::SetTrue),
    )
}

/// The shared `--lib-root` / `--dest` root-selection flags: raw
/// `--dest` override vs. the discovery-chain `--lib-root`, mutually exclusive.
fn root_flags(cmd: Command) -> Command {
    cmd.arg(
        Arg::new("lib_root")
            .long("lib-root")
            .value_name("DIR")
            .help(
                "Library root. Falls back to $RUSTYFI_LIB_ROOT, then to the \
                 first root found from the working directory (the same chain \
                 compile mode runs from the document's directory).",
            )
            .value_parser(value_parser!(PathBuf)),
    )
    .arg(
        Arg::new("dest")
            .long("dest")
            .value_name("DIR")
            .help("Raw root override, used verbatim, bypassing discovery.")
            .value_parser(value_parser!(PathBuf)),
    )
    .group(ArgGroup::new("root").args(["lib_root", "dest"]))
}

/// `build [PATH]` — run a `(libraryDoc ...)`'s own build commands, and, with
/// `--install`, materialise its declared products into the resolved root
/// (`dist/doc/<name>/<dst>`) the same way any other package installs.
fn build_command() -> Command {
    Command::new("build")
        .about("Build a `(libraryDoc ...)` target by running its own build commands.")
        .arg(
            Arg::new("path")
                .value_name("PATH")
                .help("Directory holding the Satyristes (default: the current one).")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("lang")
                .long("lang")
                .value_name("VERSION")
                .value_parser(["0.0", "0.1"])
                .help("Restrict to doc targets for this SATySFi generation."),
        )
        .arg(
            Arg::new("doc")
                .long("doc")
                .value_name("NAME")
                .action(ArgAction::Append)
                .help("Restrict to the named doc target (repeatable)."),
        )
        .arg(
            Arg::new("lib_root")
                .long("lib-root")
                .value_name("DIR")
                .help(
                    "Library root for the build commands ($RUSTYFI_LIB_ROOT), and — \
                     with --install — where its products are installed.",
                )
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("dest")
                .long("dest")
                .value_name("DIR")
                .help("Raw root override for --install, used verbatim, bypassing discovery.")
                .value_parser(value_parser!(PathBuf)),
        )
        .group(ArgGroup::new("root").args(["lib_root", "dest"]))
        .arg(
            Arg::new("install")
                .long("install")
                .action(ArgAction::SetTrue)
                .help(
                    "After a successful build, also install each doc's declared \
                     `(sources ((doc \"dst\" \"src\") ...))` products \
                     (dist/doc/<name>/<dst>), replacing any previous install of \
                     the same doc target.",
                ),
        )
        .arg(
            Arg::new("quiet")
                .long("quiet")
                .short('q')
                .action(ArgAction::SetTrue)
                .help("Do not echo each command before running it."),
        )
}

/// The package-manager commands, attached to whichever command they are given
/// — the compiler personality carries them at top level, and the
/// `satyrographos` personality carries the same set under its own name.
fn package_subcommands(cmd: Command) -> Command {
    // `global(true)`: accepted on EVERY subcommand, not just the ones that
    // read it — "which config file" is a property of the run, and a flag
    // refused depending on which subcommand precedes it is a worse surprise
    // than one accepted and unused.
    //
    // Either order works (`list --config F` and `--config F list`): a flag
    // given BEFORE the subcommand would, on its own, put clap in compile
    // mode (the compiler personality takes a positional document and sets
    // `args_conflicts_with_subcommands`) — `dispatch::get_matches`'s
    // hoist-and-retry pre-pass is what makes the leading form work anyway.
    // See this module's top doc comment for why that is a pre-pass and not a
    // clap setting.
    cmd.arg(
        Arg::new("config")
            .long("config")
            .value_name("FILE")
            .global(true)
            .help("Read this config file instead of the discovered one.")
            .value_parser(value_parser!(PathBuf)),
    )
    .subcommand(root_flags(registry_flag(
        Command::new("install")
            .about(
                "Install packages from directories, .tar.gz files, URLs, or registry names; \
                 with no PATH, reconcile the project's Satyristes.",
            )
            .arg(
                Arg::new("path")
                    .value_name("PATH")
                    .num_args(1..)
                    .action(ArgAction::Append)
                    .help(
                        "Package sources, installed in order: a directory, a .tar.gz, \
                         an http(s) URL of one (optionally #sha256=HEX), or a registry \
                         NAME[@VERSION] (repeatable).",
                    )
                    .value_parser(value_parser!(String)),
            )
            .arg(
                Arg::new("lang")
                    .long("lang")
                    .value_name("VERSION")
                    .value_parser(["0.0", "0.1"])
                    .help("Restrict to blocks written for this SATySFi generation."),
            )
            .arg(
                Arg::new("library")
                    .short('l')
                    .long("library")
                    .value_name("NAME")
                    .help("Restrict to the named library (repeatable).")
                    .action(ArgAction::Append),
            )
            .arg(
                Arg::new("force")
                    .long("force")
                    .action(ArgAction::SetTrue)
                    .help("Overwrite an existing receipted install."),
            ),
    )))
    .subcommand(root_flags(
        Command::new("uninstall")
            .about("Remove a receipted package.")
            .arg(
                Arg::new("name")
                    .required(true)
                    .num_args(1..)
                    .action(ArgAction::Append)
                    .help("Package names, removed in order (repeatable)."),
            ),
    ))
    .subcommand(build_command())
    .subcommand(root_flags(
        Command::new("list").about("List installed packages."),
    ))
    .subcommand(root_flags(
        Command::new("status")
            .about("Report installed-file presence (exit 1 if any missing).")
            .arg(Arg::new("name").help("Only this package (default: all).")),
    ))
    .subcommand(registry_flag(
        Command::new("search")
            .about("Search the package repository for keywords (name and description).")
            .arg(
                Arg::new("term")
                    .value_name("KEYWORD")
                    .required(true)
                    .num_args(1..)
                    .action(ArgAction::Append)
                    .help(
                        "Keywords to look for, matched against name and description. \
                         Several NARROW the search: every keyword must match.",
                    ),
            ),
    ))
    .subcommand(root_flags(registry_flag(Command::new("update").about(
        "Re-fetch the registry index and report available upgrades vs Satyristes.lock.",
    ))))
    .subcommand(publish_command())
}

/// `publish` — write this project's `Satyristes` library into a package
/// repository as an installable definition, the way `opam publish` does for an
/// OCaml package. `--url` names an already-released tarball: nothing here
/// builds or uploads one, and nothing pushes.
fn publish_command() -> Command {
    registry_flag(
        Command::new("publish")
            .about("Write this project's library into a package repository as a release.")
            .arg(
                Arg::new("url")
                    .long("url")
                    .value_name("URL")
                    .required(true)
                    .help(
                        "The released source tarball's URL, recorded verbatim and never \
                         fetched. Hosting it is yours, as with `opam publish`.",
                    ),
            )
            .arg(Arg::new("sha256").long("sha256").value_name("HEX").help(
                "The tarball's SHA-256. Required unless --archive supplies it; \
                 given with --archive, the two must agree.",
            ))
            .arg(
                Arg::new("archive")
                    .long("archive")
                    .value_name("PATH")
                    .help("A local copy of the tarball at --url, hashed to supply --sha256.")
                    .value_parser(value_parser!(PathBuf)),
            )
            .arg(
                Arg::new("project")
                    .long("project")
                    .value_name("DIR")
                    .help("Find the Satyristes upward from DIR (default: the current directory).")
                    .value_parser(value_parser!(PathBuf)),
            )
            .arg(
                Arg::new("library")
                    .short('l')
                    .long("library")
                    .value_name("NAME")
                    .help("Which `(library ...)` block to publish, if the manifest has several."),
            )
            .arg(
                Arg::new("shape")
                    .long("shape")
                    .value_name("SHAPE")
                    .value_parser(["opam", "toml"])
                    .help(
                        "Repository layout: `opam` (packages/<id>/<id>.<v>/opam, \
                         Satyrographos' own) or `toml` (packages/<id>.toml, this port's \
                         index). Detected from what the repository already holds when \
                         omitted; required when that is undecidable.",
                    ),
            )
            .arg(
                Arg::new("package_name")
                    .long("package-name")
                    .value_name("ID")
                    .help(
                        "Publish under this package id instead of the derived one \
                         (`satysfi-<name>` for an opam repository, `<name>` for the \
                         TOML index).",
                    ),
            )
            .arg(
                Arg::new("description")
                    .long("description")
                    .value_name("TEXT")
                    .help("One-line description (opam `synopsis:`, or the TOML `description`)."),
            )
            .arg(
                Arg::new("maintainer")
                    .long("maintainer")
                    .value_name("TEXT")
                    .help("opam `maintainer:` (the TOML index has no such field)."),
            )
            .arg(
                Arg::new("commit")
                    .long("commit")
                    .action(ArgAction::SetTrue)
                    .help("git add + git commit the written file in the repository checkout."),
            )
            .arg(
                Arg::new("branch")
                    .long("branch")
                    .value_name("NAME")
                    .help("Commit on this branch, creating it if needed (with --commit)."),
            )
            .arg(
                Arg::new("force")
                    .long("force")
                    .action(ArgAction::SetTrue)
                    .help("Replace an already-published <name>.<version>."),
            )
            .arg(
                Arg::new("dry_run")
                    .long("dry-run")
                    .action(ArgAction::SetTrue)
                    .help("Compose and check everything, print the definition, write nothing."),
            ),
    )
}

fn satyrographos_command() -> Command {
    package_subcommands(
        Command::new("satyrographos")
            .about("SATySFi package manager (this port's Satyrographos analog).")
            .subcommand_required(true)
            .arg_required_else_help(true),
    )
}

/// `rustyfi man` — write this program's man page, in roff, to stdout. Hidden
/// because it is a packaging step (the release archive pipes it into
/// `share/man/man1/rustyfi.1`), not something a user needs in `--help`.
fn man_command() -> Command {
    Command::new("man")
        .about("Write the man page (roff) to stdout.")
        .hide(true)
}

fn multicall_command() -> Command {
    Command::new("multicall")
        .about("Install `rustyfi`/`satyrographos` aliases of this binary.")
        .hide(true)
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("install")
                .about("Hardlink (or copy) this exe to <DIR>/rustyfi and <DIR>/satyrographos.")
                .arg(
                    Arg::new("dir")
                        .long("dir")
                        .value_name("DIR")
                        .required(true)
                        .value_parser(value_parser!(PathBuf)),
                ),
        )
}
