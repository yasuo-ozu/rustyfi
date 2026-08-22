//! Per-script font scheme
//! + `set-font` real wiring + script-segmented `text_to_boxes`. Drives
//! `set-font`/`primitives::read_inline` directly through `Interp::apply`
//! (like `tests/text_info.rs`'s "eval half"), sidestepping the parser.

use rustyfi_backend::{Context, FontKey, FontMetrics, Length, PureHorzBox, Script};
use rustyfi_lang::eval::Interp;
use rustyfi_lang::primitives;
use rustyfi_lang::quoted::IText;
use rustyfi_lang::value::{BaseEnv, Env, Value};

/// Every glyph is half an em wide; `resolve_font_abbrev` names two registry
/// abbrevs (`set-font`/`set-math-font` now consult before
/// falling back to the 3-face heuristic).
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
    env: &BaseEnv,
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

/// `get-font script ctx` -> `(abbrev, ratio, rising)`.
fn get_font(
    interp: &mut Interp,
    env: &BaseEnv,
    script: &str,
    ctx: Context,
) -> (String, f64, f64) {
    let prim = env.lookup("get-font").expect("get-font is registered");
    let script_v = Value::Ctor(script.to_string(), None);
    let v1 = interp.apply(prim, script_v).unwrap();
    match interp.apply(v1, Value::Context(Box::new(ctx))).unwrap() {
        Value::Tuple(vs) => match &vs[..] {
            [Value::Str(a), Value::Float(r), Value::Float(ri)] => (a.clone(), *r, *ri),
            other => panic!("get-font returned an unexpected triple: {other:?}"),
        },
        other => panic!("get-font did not return a tuple: {other:?}"),
    }
}

/// The ratio/rising round-trip is what every real caller uses — `ruby`'s
/// `let (_, ratio, _) = get-font HanIdeographic ctx`, `quotation`'s two-em
/// Japanese indent, and upstream's own `convertText.ml:78`.
#[test]
fn get_font_reads_back_what_set_font_wrote() {
    let stub = Stub;
    let mut interp = Interp::new(&stub);
    let env = primitives::base_env();
    let ctx = Context::initial(Length::pt(400.0));

    let after = set_font(&mut interp, &env, "Kana", "mykana", 0.88, 0.25, ctx);
    let (abbrev, ratio, rising) = get_font(&mut interp, &env, "Kana", after.clone());

    assert_eq!(ratio, 0.88, "the size ratio must round-trip exactly");
    assert_eq!(rising, 0.25, "so must the rising ratio");
    // `Stub` implements `resolve_font_abbrev` but not its inverse
    // `font_abbrev`, so the head comes back empty — the documented
    // "resolved eagerly, name recoverable only from the store that minted the
    // key" case. Nothing in the corpus reads this slot.
    assert_eq!(abbrev, "", "no reverse map on this provider");

    // A script nobody set still answers, with the initial scheme's values,
    // rather than erroring the way upstream's `failwith` would.
    let (_, latin_ratio, latin_rising) = get_font(&mut interp, &env, "Latin", after);
    assert_eq!((latin_ratio, latin_rising), (1.0, 0.0));
}

/// `get-font OtherScript` follows `set-dominant-narrow-script` the same way
/// glyph measurement does — both go through `script_font`, which IS upstream's
/// `get_font_with_ratio` (normalize the script, then read the scheme slot).
#[test]
fn get_font_normalizes_other_script_through_the_dominant_narrow_script() {
    let stub = Stub;
    let mut interp = Interp::new(&stub);
    let env = primitives::base_env();
    let ctx = Context::initial(Length::pt(400.0));

    let mut ctx = set_font(&mut interp, &env, "Kana", "mykana", 0.88, 0.0, ctx);
    let (_, before, _) = get_font(&mut interp, &env, "OtherScript", ctx.clone());
    assert_eq!(before, 1.0, "unset OtherScript reads its own (initial) slot");

    ctx.dominant_narrow_script = Script::Kana;
    let (_, after, _) = get_font(&mut interp, &env, "OtherScript", ctx);
    assert_eq!(
        after, 0.88,
        "with a dominant narrow script, OtherScript resolves to ITS slot"
    );
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

    assert_eq!(
        after.font, before_font,
        "set-font Kana must not move ctx.font"
    );
    assert_eq!(
        after.font_scheme[Script::Latin as usize],
        before_latin,
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
    // No registry entry for "boldish" -> the 3-face name
    // heuristic (`resolve_font_abbrev` free fn in primitives.rs) fires
    // instead of erroring (this port's existing accept-and-degrade stance).
    let stub = Stub;
    let mut interp = Interp::new(&stub);
    let env = primitives::base_env();
    let ctx = Context::initial(Length::pt(400.0));

    let after = set_font(&mut interp, &env, "OtherScript", "myboldish", 1.0, 0.0, ctx);
    // "myboldish" contains "bold" -> heuristic resolves to FONT_BOLD (key 1).
    assert_eq!(
        after.font_scheme[Script::OtherScript as usize].font,
        FontKey(1)
    );
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
            assert_eq!(
                info.size, ctx.font_size,
                "Latin run keeps ctx.font_size (ratio 1.0)"
            );
        } else if info.font == FontKey(7) {
            saw_kana = true;
            assert_eq!(
                info.size,
                ctx.font_size * 0.88,
                "Kana run is scaled by font_scheme[Kana].ratio"
            );
        }
    }
    assert!(
        saw_latin,
        "expected a Latin-script InnerString (FontKey(0))"
    );
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
    assert_eq!(
        base_rising,
        Length::ZERO,
        "default manual_rising leaves rising at ZERO"
    );

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
    assert_eq!(
        risen,
        Length::pt(3.0),
        "set-manual-rising must reach HorzStringInfo.rising"
    );
}

/// `normalize_script` (`horzBox.ml:472`): a character in the `OtherScript`
/// bucket — this port's stand-in for upstream's `CommonNarrow` — resolves its
/// font through `ctx.dominant_narrow_script`, not through the `OtherScript`
/// scheme slot.
///
/// `□` (U+25A1) is the case that matters: `set-dominant-narrow-script Kana` is
/// how enumitem's document asks for it in the CJK face, and reading the
/// `OtherScript` slot instead put it in a Latin face with no such glyph.
#[test]
fn other_script_resolves_through_the_dominant_narrow_script() {
    let stub = Stub;
    let mut interp = Interp::new(&stub);
    let mut ctx = Context::initial(Length::pt(400.0));
    ctx.font_scheme[Script::Kana as usize] = rustyfi_backend::ScriptFont {
        font: FontKey(7),
        ratio: 0.88,
        rising: 0.0,
    };
    ctx.font_scheme[Script::OtherScript as usize] = rustyfi_backend::ScriptFont {
        font: FontKey(3),
        ratio: 1.0,
        rising: 0.0,
    };
    let elems = vec![IText::Text("\u{25A1}".to_string())];

    let boxes = primitives::read_inline(&mut interp, &ctx, &elems, &Env::root()).unwrap();
    let (font, size) = boxes
        .iter()
        .find_map(|hb| match hb {
            rustyfi_backend::HorzBox::Pure(PureHorzBox::InnerString { info, .. }) => {
                Some((info.font, info.size))
            }
            _ => None,
        })
        .expect("expected an InnerString");
    assert_eq!(
        font,
        FontKey(3),
        "default must still read the OtherScript slot"
    );
    assert_eq!(size, ctx.font_size);

    ctx.dominant_narrow_script = Script::Kana;
    let boxes = primitives::read_inline(&mut interp, &ctx, &elems, &Env::root()).unwrap();
    let (font, size) = boxes
        .iter()
        .find_map(|hb| match hb {
            rustyfi_backend::HorzBox::Pure(PureHorzBox::InnerString { info, .. }) => {
                Some((info.font, info.size))
            }
            _ => None,
        })
        .expect("expected an InnerString");
    assert_eq!(
        font,
        FontKey(7),
        "OtherScript must follow dominant_narrow_script"
    );
    assert_eq!(
        size,
        ctx.font_size * 0.88,
        "and pick up that script's ratio too"
    );
}
