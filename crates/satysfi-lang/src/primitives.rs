//! The primitive registry. Shaped so the ~300 vminst instructions can be
//! ported one `prims!` line at a time; primitives are registered under their
//! real v0.0.6 names so later stdlib loading finds them.
//!
//! Milestone 1 also hardcodes `document`, `+p`, and `\emph` as natives —
//! placeholders for the real stdlib definitions that phase 4 loads from
//! `dist/` (at which point they are deleted from here).

use crate::ast::{BText, IText};
use crate::eval::{eval_error, EvalError, Interp};
use crate::value::{DocumentValue, Env, Value};
use satysfi_backend::{
    break_into_lines, break_pages, Context, FontKey, HorzBox, HorzStringInfo, PageGeometry,
    PureHorzBox, VertBox,
};
use std::rc::Rc;

/// Font keys agreed with the milestone-1 base-14 metrics provider.
pub const FONT_REGULAR: FontKey = FontKey(0);
pub const FONT_BOLD: FontKey = FontKey(1);
pub const FONT_OBLIQUE: FontKey = FontKey(2);

pub struct PrimDef {
    pub name: &'static str,
    pub arity: usize,
    pub run: fn(&mut Interp, Vec<Value>) -> Result<Value, EvalError>,
}

impl std::fmt::Debug for PrimDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PrimDef({}/{})", self.name, self.arity)
    }
}

macro_rules! prims {
    ($($name:literal ($arity:literal) => $f:path;)*) => {
        static PRIM_DEFS: &[PrimDef] = &[
            $(PrimDef { name: $name, arity: $arity, run: $f },)*
        ];
    };
}

prims! {
    "read-inline" (2) => prim_read_inline;
    "read-block" (2) => prim_read_block;
    "line-break" (2) => prim_line_break;
    "page-break" (2) => prim_page_break;
    "document" (2) => prim_document;
    "+p" (2) => prim_cmd_p;
    "\\emph" (2) => prim_cmd_emph;
}

/// The base environment `document` programs start in.
pub fn base_env() -> Env {
    let env = Env::root();
    for def in PRIM_DEFS {
        env.define(
            def.name,
            Value::Prim {
                def,
                applied: Vec::new(),
            },
        );
    }
    env.define(
        "inline-fil",
        Value::InlineBoxes(vec![HorzBox::Pure(PureHorzBox::OuterFil)]),
    );
    env
}

// ---- argument extractors ------------------------------------------------------

fn as_context(v: Value) -> Result<Context, EvalError> {
    match v {
        Value::Context(c) => Ok(*c),
        other => eval_error(format!("expected a context, got {}", other.type_name())),
    }
}

fn as_inline_text(v: Value) -> Result<(Rc<Vec<IText>>, Env), EvalError> {
    match v {
        Value::InlineText { elems, env } => Ok((elems, env)),
        other => eval_error(format!("expected inline-text, got {}", other.type_name())),
    }
}

fn as_block_text(v: Value) -> Result<(Rc<Vec<BText>>, Env), EvalError> {
    match v {
        Value::BlockText { elems, env } => Ok((elems, env)),
        other => eval_error(format!("expected block-text, got {}", other.type_name())),
    }
}

fn as_inline_boxes(v: Value) -> Result<Vec<HorzBox>, EvalError> {
    match v {
        Value::InlineBoxes(b) => Ok(b),
        other => eval_error(format!(
            "expected inline-boxes, got {}",
            other.type_name()
        )),
    }
}

fn as_block_boxes(v: Value) -> Result<Vec<VertBox>, EvalError> {
    match v {
        Value::BlockBoxes(b) => Ok(b),
        other => eval_error(format!("expected block-boxes, got {}", other.type_name())),
    }
}

// ---- text conversion ----------------------------------------------------------

/// Convert quoted inline text to boxes under `ctx` (the core of
/// `read-inline`): words become measured `InnerString`s, whitespace becomes
/// glue, embedded commands are applied to `ctx` and their arguments.
pub fn read_inline(
    interp: &mut Interp,
    ctx: &Context,
    elems: &[IText],
    env: &Env,
) -> Result<Vec<HorzBox>, EvalError> {
    let mut out = Vec::new();
    for elem in elems {
        match elem {
            IText::Text(text) => text_to_boxes(interp, ctx, text, &mut out)?,
            IText::Cmd { name, span, args } => {
                let cmd = env.lookup(name).ok_or_else(|| EvalError {
                    span: Some(*span),
                    msg: format!("unbound inline command '{name}' at run time"),
                })?;
                let mut v = interp.apply(cmd, Value::Context(Box::new(ctx.clone())))?;
                for arg in args {
                    let arg_v = interp.eval(env, arg)?;
                    v = interp.apply(v, arg_v)?;
                }
                out.extend(as_inline_boxes(v)?);
            }
        }
    }
    Ok(out)
}

/// Convert quoted block text to vertical boxes (the core of `read-block`).
pub fn read_block(
    interp: &mut Interp,
    ctx: &Context,
    elems: &[BText],
    env: &Env,
) -> Result<Vec<VertBox>, EvalError> {
    let mut out = Vec::new();
    for BText::Cmd { name, span, args } in elems {
        let cmd = env.lookup(name).ok_or_else(|| EvalError {
            span: Some(*span),
            msg: format!("unbound block command '{name}' at run time"),
        })?;
        let mut v = interp.apply(cmd, Value::Context(Box::new(ctx.clone())))?;
        for arg in args {
            let arg_v = interp.eval(env, arg)?;
            v = interp.apply(v, arg_v)?;
        }
        out.extend(as_block_boxes(v)?);
    }
    Ok(out)
}

fn text_to_boxes(
    interp: &mut Interp,
    ctx: &Context,
    text: &str,
    out: &mut Vec<HorzBox>,
) -> Result<(), EvalError> {
    let space_width = interp
        .metrics
        .advance(ctx.font, ' ', ctx.font_size)
        .unwrap_or(ctx.font_size * 0.33);
    let mut word = String::new();
    let flush_word = |word: &mut String, out: &mut Vec<HorzBox>| -> Result<(), EvalError> {
        if word.is_empty() {
            return Ok(());
        }
        let width = interp
            .metrics
            .text_width(ctx.font, word, ctx.font_size)
            .ok_or_else(|| EvalError {
                span: None,
                msg: format!(
                    "text '{word}' contains characters not available in the \
                     milestone-1 base fonts (WinAnsi only)"
                ),
            })?;
        out.push(HorzBox::Pure(PureHorzBox::InnerString {
            info: HorzStringInfo {
                font: ctx.font,
                size: ctx.font_size,
            },
            text: std::mem::take(word),
            width,
            height: interp.metrics.ascender(ctx.font, ctx.font_size),
            depth: interp.metrics.descender(ctx.font, ctx.font_size),
        }));
        Ok(())
    };
    for c in text.chars() {
        if c == ' ' || c == '\n' {
            flush_word(&mut word, out)?;
            // Avoid piling up doubled glue at text-run boundaries.
            if !matches!(
                out.last(),
                Some(HorzBox::Pure(PureHorzBox::OuterEmpty { .. }))
            ) {
                out.push(HorzBox::Pure(PureHorzBox::OuterEmpty {
                    natural: space_width,
                    shrinkable: space_width * 0.25,
                    stretchable: space_width * 0.5,
                }));
            }
        } else {
            word.push(c);
        }
    }
    flush_word(&mut word, out)
}

// ---- primitive bodies ----------------------------------------------------------

fn prim_read_inline(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let it = args.pop().unwrap();
    let ctx = as_context(args.pop().unwrap())?;
    let (elems, env) = as_inline_text(it)?;
    Ok(Value::InlineBoxes(read_inline(interp, &ctx, &elems, &env)?))
}

fn prim_read_block(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let bt = args.pop().unwrap();
    let ctx = as_context(args.pop().unwrap())?;
    let (elems, env) = as_block_text(bt)?;
    Ok(Value::BlockBoxes(read_block(interp, &ctx, &elems, &env)?))
}

/// NOTE: the real v0.0.6 `line-break` takes two leading breakability bools;
/// milestone 1 registers the simplified `context -> inline-boxes` form.
fn prim_line_break(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let ib = as_inline_boxes(args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    Ok(Value::BlockBoxes(break_into_lines(&ctx, ib)))
}

fn prim_page_break(_interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let bb = as_block_boxes(args.pop().unwrap())?;
    let ctx = as_context(args.pop().unwrap())?;
    let geometry = PageGeometry::default();
    let pages = break_pages(&geometry, ctx.leading, bb);
    Ok(Value::Document(Rc::new(DocumentValue { geometry, pages })))
}

/// `document : record -> block-text -> document` (milestone-1 native; the
/// record's `title`/`author` are accepted but not yet rendered).
fn prim_document(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let bt = args.pop().unwrap();
    let record = args.pop().unwrap();
    if !matches!(record, Value::Record(_)) {
        return eval_error(format!(
            "document expects a record, got {}",
            record.type_name()
        ));
    }
    let geometry = PageGeometry::default();
    let ctx = Context::initial(geometry.text_width);
    let (elems, env) = as_block_text(bt)?;
    let bb = read_block(interp, &ctx, &elems, &env)?;
    let pages = break_pages(&geometry, ctx.leading, bb);
    Ok(Value::Document(Rc::new(DocumentValue { geometry, pages })))
}

/// `+p : context -> inline-text -> block-boxes` — a paragraph: read the
/// inline text, append `inline-fil`, break into lines.
fn prim_cmd_p(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let it = args.pop().unwrap();
    let ctx = as_context(args.pop().unwrap())?;
    let (elems, env) = as_inline_text(it)?;
    let mut boxes = read_inline(interp, &ctx, &elems, &env)?;
    boxes.push(HorzBox::Pure(PureHorzBox::OuterFil));
    Ok(Value::BlockBoxes(break_into_lines(&ctx, boxes)))
}

/// `\emph : context -> inline-text -> inline-boxes` — re-read the argument
/// in the oblique face.
fn prim_cmd_emph(interp: &mut Interp, mut args: Vec<Value>) -> Result<Value, EvalError> {
    let it = args.pop().unwrap();
    let ctx = as_context(args.pop().unwrap())?;
    let emph_ctx = Context {
        font: FONT_OBLIQUE,
        ..ctx
    };
    let (elems, env) = as_inline_text(it)?;
    Ok(Value::InlineBoxes(read_inline(
        interp, &emph_ctx, &elems, &env,
    )?))
}
