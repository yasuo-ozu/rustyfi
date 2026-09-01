use rustyfi_lsp::{format_cst_outcome, CstOptions, CstOutcome, RustyfiVersion};
fn main() {
    let src = std::io::read_to_string(std::io::stdin()).unwrap();
    let v = match std::env::args().nth(1).as_deref() {
        Some("v01") => RustyfiVersion::V0_1,
        _ => RustyfiVersion::V0_0,
    };
    match format_cst_outcome(&src, v, &CstOptions::default()) {
        CstOutcome::Formatted(s) => print!("{s}"),
        o => eprintln!("NOT FORMATTED: {o:?}"),
    }
}
