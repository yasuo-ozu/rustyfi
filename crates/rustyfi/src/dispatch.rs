//! The clap *builder*-style dispatch tree (plan §7.4). Builder rather than
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
//! basename, so the same binary is `rustyfi` or
//! `satyrographos` depending on the name it is invoked under (see plan §4.5's
//! alias-install helper). Under `rustyfi` the compile args and the
//! `satyrographos`/`multicall` subcommand trees coexist via
//! `args_conflicts_with_subcommands` + `subcommand_negates_reqs`, so the
//! required positional `input` is only demanded when no subcommand is used.

use std::path::PathBuf;

use clap::{value_parser, Arg, ArgAction, ArgGroup, Command};

/// Build the full multicall dispatch tree.
pub fn build_cli() -> Command {
    Command::new("rustyfi")
        .multicall(true)
        .subcommand_required(true)
        .arg_required_else_help(true)
        // `rustyfi` personality: compiler + nested package manager. There used
        // to be a second, compile-only `rustyfi` personality beside the full
        // `rustyfi-rust` one; with the binary renamed they are the same name,
        // so they are the same personality — `rustyfi` now carries the compile
        // args AND the subcommand trees.
        .subcommand(
            package_subcommands(compile_command("rustyfi"))
                .subcommand(multicall_command())
                .subcommand(man_command())
                .args_conflicts_with_subcommands(true)
                .subcommand_negates_reqs(true),
        )
        // `satyrographos` personality: the same package commands under the
        // name real Satyrographos users type. The compiler personality carries
        // them directly now, so this is an alias, not a separate tree.
        .subcommand(satyrographos_command())
}

/// The compile-mode argument set — the pre-chimera derive `Args`, restated as
/// builder `Arg`s so behavior is preserved byte-for-byte (positional input,
/// `-o/--output`, `--lib-root`, `--lang`).
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
                    "Output format: pdf (default), html, or html-reflow. HTML is a \
                     faithful, non-reflowing serialization of the same laid-out \
                     pages the PDF writer renders — a preview/visual-diff \
                     aid, not reflowable web output. html-reflow is a \
                     SEPARATE, semantic/reflowable serialization — real flowing \
                     paragraphs and CSS layout, no fixed positions — a readable \
                     approximation, not layout-faithful.",
                )
                .value_parser(["pdf", "html", "html-reflow"])
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

/// The shared `--registry URL` flag (plan §5.4 step 1), attached to the
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

/// The shared `--lib-root` / `--dest` root-selection flags (plan §4): raw
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

/// The `satyrographos` subcommand tree (plan §4.1-4.4).
/// `build [PATH]` — run a `(libraryDoc ...)`'s own build commands. No root
/// flags: it runs a program and installs nothing.
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
                .help("Library root for the build commands ($RUSTYFI_LIB_ROOT).")
                .value_parser(value_parser!(PathBuf)),
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
    // It must come after the subcommand (`list --config F`, not `--config F
    // list`): the compiler personality takes a positional document and sets
    // `args_conflicts_with_subcommands`, so an argument given before the
    // subcommand puts clap in compile mode.
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
}

/// The `satyrographos` personality: the same commands, under that name.
fn satyrographos_command() -> Command {
    package_subcommands(
        Command::new("satyrographos")
            .about("SATySFi package manager (this port's Satyrographos analog).")
            .subcommand_required(true)
            .arg_required_else_help(true),
    )
}

/// The hidden `multicall install --dir DIR` alias helper (plan §4.5).
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
