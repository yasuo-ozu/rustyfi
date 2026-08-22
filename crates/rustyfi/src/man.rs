//! `rustyfi man` — the man page, rendered from the same `clap::Command` the
//! CLI parses with.
//!
//! Generated rather than checked in so the page cannot drift: every option and
//! subcommand comes out of clap, so adding a flag documents it. What clap
//! cannot know — the environment variables the port reads, the files it looks
//! for, worked examples — is hand-written in `man_extra.roff` and spliced in
//! after the generated sections.

use std::io::Write;

/// The sections clap has no way to produce, in the order a reader expects
/// them: after the options and subcommands, before the version footer.
const EXTRA: &str = include_str!("man_extra.roff");

pub fn render(out: &mut dyn Write) -> std::io::Result<()> {
    // `build_cli()` is a multicall root whose subcommands are the personalities
    // (`rustyfi`, `satyrographos`), so rendering it directly would produce a
    // synopsis of personalities rather than of the compiler. The page is for
    // the compiler personality, which carries the real arguments.
    let cli = crate::dispatch::build_cli();
    let cmd = cli
        .find_subcommand("rustyfi")
        .expect("the compiler personality")
        .clone()
        .version(env!("CARGO_PKG_VERSION"));

    // `.TH` takes title, section, DATE, source, manual in that order, and an
    // empty field is dropped rather than kept as a placeholder — leaving the
    // date empty shifts `manual` into `source` and the page renders with the
    // wrong header. A blank-but-present date keeps the fields aligned, and
    // stays reproducible in a way that today's date would not.
    let man = clap_mangen::Man::new(cmd)
        .title("RUSTYFI")
        .section("1")
        .date(" ")
        .manual("User Commands");

    let mut buf = Vec::new();
    man.render_title(&mut buf)?;
    man.render_name_section(&mut buf)?;
    man.render_synopsis_section(&mut buf)?;
    man.render_description_section(&mut buf)?;
    man.render_options_section(&mut buf)?;
    man.render_subcommands_section(&mut buf)?;
    buf.extend_from_slice(EXTRA.as_bytes());
    man.render_version_section(&mut buf)?;

    out.write_all(collapse_preamble(&buf).as_bytes())
}

/// Rendering section by section is what lets the hand-written sections sit in
/// the middle, but each call re-emits roff's quote-character preamble. A page
/// needs it once, at the top.
fn collapse_preamble(buf: &[u8]) -> String {
    const PREAMBLE: [&str; 2] = [r".ie \n(.g .ds Aq \(aq", r".el .ds Aq '"];
    let text = String::from_utf8_lossy(buf);
    let mut seen = false;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if PREAMBLE.contains(&line) {
            if seen {
                continue;
            }
            if line == PREAMBLE[1] {
                seen = true;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
