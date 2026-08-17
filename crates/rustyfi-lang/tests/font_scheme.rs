//! Group D, D1b (`docs/plans/text-rendering.md` §1b): per-script font scheme
//! + `set-font` real wiring + script-segmented `text_to_boxes`. Drives
//! `set-font`/`primitives::read_inline` directly through `Interp::apply`
//! (like `tests/text_info.rs`'s "eval half"), sidestepping the parser.

use rustyfi_backend::{Context, FontKey, FontMetrics, Length, PureHorzBox, Script};
use rustyfi_lang::ast::IText;
use rustyfi_lang::eval::Interp;
use rustyfi_lang::primitives;
use rustyfi_lang::value::{Env, Value};

/// Every glyph is half an em wide; `resolve_font_abbrev` names two registry
/// abbrevs (the D1a upgrade `set-font`/`set-math-font` now consult before
/// falling back to the milestone-1 3-face heuristic).
struct Stub;

impl FontMetrics for Stub {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.8
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.2
    }
    fn resolve_font_abbrev(&self, abbrev: &str) -> Option<FontKey> {
        match abbrev {
            "mykana" => Some(FontKey(7)),
            "mylatin" => Some(FontKey(9)),
            _ => None,
        }
    }
}

fn set_font(
    interp: &mut Interp,
    env: &Env,
    script: &str,
    abbrev: &str,
    ratio: f64,
    rising: f64,
    ctx: Context,
) -> Context {
    let prim = env.lookup("set-font").expect("set-font is registered");
    let script_v = Value::Ctor(script.to_string(), None);
    let font_v = Value::Tuple(vec![
        Value::Str(abbrev.to_string()),
        Value::Float(ratio),
        Value::Float(rising),
    ]);
    let ctx_v = Value::Context(Box::new(ctx));
    let v1 = interp.apply(prim, script_v).unwrap();
    let v2 = interp.apply(v1, font_v).unwrap();
    match interp.apply(v2, ctx_v).unwrap() {
        Value::Context(c) => *c,
        other => panic!("set-font did not return a context: {other:?}"),
    }
}

#[test]
fn set_font_kana_changes_only_scheme_kana() {
    let stub = Stub;
    let mut interp = Interp::new(&stub);
    let env = primitives::base_env();
    let ctx = Context::initial(Length::pt(400.0));
    let before_font = ctx.font;
    let before_latin = ctx.font_scheme[Script::Latin as usize];

    let after = set_font(&mut interp, &env, "Kana", "mykana", 0.88, 0.0, ctx);

    assert_eq!(after.font, before_font, "set-font Kana must not move ctx.font");
    assert_eq!(
        after.font_scheme[Script::Latin as usize], before_latin,
        "set-font Kana must not touch the Latin slot"
    );
    let kana = after.font_scheme[Script::Kana as usize];
    assert_eq!(kana.font, FontKey(7));
    assert_eq!(kana.ratio, 0.88);
    assert_eq!(kana.rising, 0.0);
}

#[test]
fn set_font_latin_also_moves_ctx_font() {
    let stub = Stub;
    let mut interp = Interp::new(&stub);
    let env = primitives::base_env();
    let ctx = Context::initial(Length::pt(400.0));

    let after = set_font(&mut interp, &env, "Latin", "mylatin", 1.0, 0.0, ctx);

    assert_eq!(after.font, FontKey(9), "set-font Latin must move ctx.font");
    assert_eq!(after.font_scheme[Script::Latin as usize].font, FontKey(9));
}

#[test]
fn set_font_unknown_abbrev_falls_back_to_heuristic() {
    // No registry entry for "boldish" -> the milestone-1 3-face name
    // heuristic (`resolve_font_abbrev` free fn in primitives.rs) fires
    // instead of erroring (this port's existing accept-and-degrade stance).
    let stub = Stub;
    let mut interp = Interp::new(&stub);
    let env = primitives::base_env();
    let ctx = Context::initial(Length::pt(400.0));

    let after = set_font(&mut interp, &env, "OtherScript", "myboldish", 1.0, 0.0, ctx);
    // "myboldish" contains "bold" -> heuristic resolves to FONT_BOLD (key 1).
    assert_eq!(after.font_scheme[Script::OtherScript as usize].font, FontKey(1));
}

#[test]
fn mixed_script_paragraph_produces_two_font_keys_and_scaled_size() {
    let stub = Stub;
    let mut interp = Interp::new(&stub);
    let mut ctx = Context::initial(Length::pt(400.0));
    // Configure Kana at 0.88x, a distinct FontKey — without going through
    // `set-font` (this test only cares about `text_to_boxes`'s consumption
    // of an already-configured scheme).
    ctx.font_scheme[Script::Kana as usize] = rustyfi_backend::ScriptFont {
        font: FontKey(7),
        ratio: 0.88,
        rising: 0.0,
    };

    let elems = vec![IText::Text("hi あ".to_string())];
    let boxes = primitives::read_inline(&mut interp, &ctx, &elems, &Env::root())
        .expect("read_inline should succeed");

    let mut saw_latin = false;
    let mut saw_kana = false;
    for hb in &boxes {
        let rustyfi_backend::HorzBox::Pure(PureHorzBox::InnerString { info, .. }) = hb else {
            continue;
        };
        if info.font == FontKey(0) {
            saw_latin = true;
            assert_eq!(info.size, ctx.font_size, "Latin run keeps ctx.font_size (ratio 1.0)");
        } else if info.font == FontKey(7) {
            saw_kana = true;
            assert_eq!(
                info.size,
                ctx.font_size * 0.88,
                "Kana run is scaled by font_scheme[Kana].ratio"
            );
        }
    }
    assert!(saw_latin, "expected a Latin-script InnerString (FontKey(0))");
    assert!(saw_kana, "expected a Kana-script InnerString (FontKey(7))");
}

/// `set-manual-rising` must actually reach `HorzStringInfo.rising` (the
/// silent-field fix): `text_to_boxes` adds `ctx.manual_rising` on top of the
/// script-font's own rising. Default `manual_rising == ZERO` ⇒ unchanged.
#[test]
fn manual_rising_is_added_to_inner_string_rising() {
    let stub = Stub;
    let mut interp = Interp::new(&stub);
    let elems = vec![IText::Text("hi".to_string())];

    // Baseline: default context (manual_rising == ZERO) — Latin sf.rising is
    // 0.0, so the run's rising is exactly ZERO (byte-identity control).
    let base_ctx = Context::initial(Length::pt(400.0));
    let base_boxes = primitives::read_inline(&mut interp, &base_ctx, &elems, &Env::root())
        .expect("read_inline should succeed");
    let base_rising = base_boxes
        .iter()
        .find_map(|hb| match hb {
            rustyfi_backend::HorzBox::Pure(PureHorzBox::InnerString { info, .. }) => {
                Some(info.rising)
            }
            _ => None,
        })
        .expect("expected an InnerString");
    assert_eq!(base_rising, Length::ZERO, "default manual_rising leaves rising at ZERO");

    // With a manual rise installed, the run's rising picks it up exactly.
    let mut risen_ctx = Context::initial(Length::pt(400.0));
    risen_ctx.manual_rising = Length::pt(3.0);
    let risen_boxes = primitives::read_inline(&mut interp, &risen_ctx, &elems, &Env::root())
        .expect("read_inline should succeed");
    let risen = risen_boxes
        .iter()
        .find_map(|hb| match hb {
            rustyfi_backend::HorzBox::Pure(PureHorzBox::InnerString { info, .. }) => {
                Some(info.rising)
            }
            _ => None,
        })
        .expect("expected an InnerString");
    assert_eq!(risen, Length::pt(3.0), "set-manual-rising must reach HorzStringInfo.rising");
}
