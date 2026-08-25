use rustyfi_lsp::{build_model, completions, record_label_slot, RustyfiVersion};
#[test]
fn p() {
    let src = "@require: stdja-mini\ndocument (| ti\n";
    let byte = src.find("(| ti").unwrap() + "(| ti".len();
    println!("slot? {}", record_label_slot(src, RustyfiVersion::V0_0, byte));
    let m = build_model(src, Some(RustyfiVersion::V0_0));
    println!("labels: {:?}", completions(&m, byte).into_iter().map(|c| c.label).collect::<Vec<_>>());
    println!("field refs in buffer: {:?}", m.refs.iter().filter(|r| format!("{:?}", r.ns) == "Field").map(|r| &r.name).collect::<Vec<_>>());
}
