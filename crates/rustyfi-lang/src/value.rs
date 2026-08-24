//! Runtime values (a subset of `syntactic_value`).

use crate::compile::CompiledExpr;
use crate::primitives::PrimDef;
use crate::quoted::{BText, IText, MathElem};
use rustyfi_backend::{
    AnnotAction, Color, Context, DecoId, DocExtras, FrameDecoration, HorzBox, HyphenLang, ImageId,
    ImageResource, Length, MathCharClass, MathKind, Page, PageGeometry, VertBox,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

// `Value::CompiledClosure` carries a crate-internal `CompiledExpr` body.
// External code can obtain such a value but cannot name, construct, or
// inspect its body, which is the intent — so `private_interfaces` is
// deliberately allowed for that one field.
#[allow(private_interfaces)]
#[derive(Clone, Debug)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Length(Length),
    Str(String),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    /// A variant constructor value, optionally carrying a payload
    /// (`None` / `Some 3`).
    Ctor(String, Option<Box<Value>>),
    Record(BTreeMap<String, Value>),
    Context(Box<Context>),
    /// Quoted inline text with its captured environment
    /// (`InputHorzWithEnvironment`). `elems` is the COMPILED element tree
    /// ([`crate::quoted`]): command names and embedded expressions were
    /// resolved at compile time, so nothing here is looked up by name at
    /// layout time — the environment is still captured because a compiled
    /// node resolves its *locals* against the environment it runs in.
    InlineText {
        elems: Rc<Vec<IText>>,
        env: Env,
    },
    /// Quoted block text with its captured environment.
    BlockText {
        elems: Rc<Vec<BText>>,
        env: Env,
    },
    /// Quoted math text with its captured environment (mirrors
    /// `InlineText`/`BlockText`); typesetting is deferred, so this
    /// is carried opaquely for now.
    MathText {
        elems: Rc<Vec<MathElem>>,
        env: Env,
    },
    /// The faithful `math` value — what every `math-*` primitive
    /// (`math-char`, `math-concat`, `math-sup`, …) builds and consumes, as
    /// opposed to `MathText`'s elaborator-fused literal form. A `math`
    /// value is always a *sequence* of atoms (mirroring upstream `MathValue
    /// of math list`, `types.cppo.ml:888` — each `Math` here is one
    /// already-classed atom, not a further list), so `math-concat` is a
    /// plain `Vec` append and `math-group`/`math-sup`/… each wrap the whole
    /// inner `Vec` as ONE new atom. Both this and `MathText` type as
    /// `"math"` — a `${…}` literal and a `math-*`-primitive-built value are
    /// interchangeable wherever a `math`-typed argument is expected (see
    /// `primitives.rs`'s `as_math`, which accepts either).
    Math(Rc<Vec<Math>>),
    /// `math-boxes` (V0_1 only) — the evaluated math tree `read-math`
    /// produces, wrapping the SAME `Math` atom tree `Value::Math` uses so
    /// every layout/primitive helper is shared unchanged. Distinct from
    /// `Value::Math`: no V0_0 primitive ever produces or consumes this
    /// variant, and no V0_1 primitive ever produces `Value::Math` — kept
    /// apart so a V0_1 program can't silently pass a `math-text` where
    /// `math-boxes` is required (`as_math_boxes` is strict).
    MathBoxes(Rc<Vec<Math>>),
    /// A mutable cell (`let-mutable`'s binding; v0.0.6's `Location`/store
    /// entry). This port uses a directly-shared `RefCell` instead of an
    /// indirection through a separate store table.
    Ref(Rc<RefCell<Value>>),
    /// `inline-boxes` (the `Horz` base constant).
    InlineBoxes(Vec<HorzBox>),
    /// `block-boxes` (the `Vert` base constant).
    BlockBoxes(Vec<VertBox>),
    /// `image` (`load-image`'s result): an index into the document-wide
    /// image table (`Interp::images`), moved into `DocumentValue::images`
    /// once `page-break` packages the final document. Carrying just the
    /// index (not the decoded bytes) keeps this value cheap to clone, same
    /// as `Value::Ref`'s `Rc`.
    Image(ImageId),
    Document(Rc<DocumentValue>),
    /// A closure. Its body is an already-compiled `CompiledExpr`, run
    /// directly by [`crate::eval::Interp::apply`] — the only closure
    /// representation.
    CompiledClosure {
        /// SATySFi 0.1 labeled optional LABELS, in binder order; empty for
        /// every 0.0.6-built closure. Each receives an `option`-typed value at
        /// application (`Some v` when the call supplies `?(label = v)`, `None`
        /// otherwise). Only the labels survive — a call site matches against
        /// them by name — while the binders they bind to are slots `0..n` of
        /// the frame application pushes, so their names are gone.
        opt_labels: Vec<String>,
        /// The positional parameter's slot is `opt_labels.len()`, immediately
        /// after the optional binders, so it needs no field of its own.
        body: CompiledExpr,
        env: Env,
    },
    /// `&e` — a quoted expression awaiting the next stage, with the
    /// environment it was quoted in. Typed `code ty` ([`crate::types::MonoType::Code`]).
    ///
    /// The same shape as [`Value::CompiledClosure`] minus a parameter, and
    /// for the same reason: this evaluator compiles to slot-indexed
    /// closures, so a fragment cannot be carried as a re-compilable syntax
    /// tree the way upstream's `code_value` is — its variable references
    /// are already bound to the frames of the scope it was written in.
    /// Carrying the compiled body with its environment keeps those
    /// references meaning what they said, which is what `~` then forces.
    Code {
        body: CompiledExpr,
        env: Env,
    },
    /// A (possibly partially applied) native primitive.
    Prim {
        def: &'static PrimDef,
        applied: Vec<Value>,
    },
    /// `pre-path` (`start-path`/`line-to`'s result).
    PrePath(rustyfi_backend::PrePath),
    /// `path` (`terminate-path`/`close-with-line`'s result).
    Path(rustyfi_backend::Path),
    /// `graphics` — one resolved drawing element (`fill`/`stroke`'s result);
    /// a `graphics list` is just `Value::List` of these, same as upstream.
    Graphics(rustyfi_backend::GraphicsElem),
    /// `font` (**V0_1 only**; upstream `saphe-split`'s `BCFontKey of
    /// FontKey.t`) — an OPAQUE handle on one loaded face, already resolved
    /// through the metrics provider's font store at the point the value was
    /// minted. Upstream mints one per `files[]` entry of a FONT ENVELOPE
    /// (`envelopeChecker.ml`'s `check_font_envelope`, evaluating the
    /// internal `LoadSingleFont{path}`/`LoadCollectionFont{path;index}`
    /// node to `BCFontKey`); this port's bundled 0.1 font envelopes are
    /// `.satyh` stand-ins that mint theirs through the LOCAL
    /// `load-single-font` primitive instead.
    ///
    /// Deliberately carries NO abbrev/name/path: it is a store INDEX, the
    /// same thing upstream's `FontKey.t` is, and nothing in the language can
    /// map it back. That opacity is load-bearing for the cross-version
    /// boundary: 0.0.6's font-consuming primitives want an ABBREV naming a
    /// row of `dist/hash/fonts.satysfi-hash`, and no such name is
    /// recoverable from a key.
    Font(rustyfi_backend::FontKey),
    /// `text-info` (the text-mode-context sliver).
    TextInfo(TextInfo),
    /// `hyphenation` (`load-hyphenation-dictionary`'s result) — the tag
    /// `set-hyphenation-dictionary` writes into
    /// `Context::hyphen_dictionary`, naming which dictionary was requested.
    Hyphenation(HyphenLang),
}

impl Value {
    /// A short type name for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Unit => "unit",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Length(_) => "length",
            Value::Str(_) => "string",
            Value::List(_) => "list",
            Value::Tuple(_) => "tuple",
            Value::Ctor(_, _) => "variant",
            Value::Record(_) => "record",
            Value::Context(_) => "context",
            Value::InlineText { .. } => "inline-text",
            Value::BlockText { .. } => "block-text",
            Value::MathText { .. } => "math",
            Value::Math(_) => "math",
            Value::MathBoxes(_) => "math-boxes",
            Value::Ref(_) => "mutable",
            Value::InlineBoxes(_) => "inline-boxes",
            Value::BlockBoxes(_) => "block-boxes",
            Value::Image(_) => "image",
            Value::Document(_) => "document",
            Value::CompiledClosure { .. } => "function",
            Value::Code { .. } => "code",
            Value::Prim { .. } => "function",
            Value::PrePath(_) => "pre-path",
            Value::Path(_) => "path",
            Value::Graphics(_) => "graphics",
            Value::Font(_) => "font",
            Value::TextInfo(_) => "text-info",
            Value::Hyphenation(_) => "hyphenation",
        }
    }
}

/// One atom of a faithful `math` value (`Value::Math`'s element type) —
/// trimmed mirror of upstream `math` (`types.cppo.ml:1024`). Every
/// closure-typed field upstream carries (kern functions, a paren pair's
/// sizing closures, `math-pull-in-scripts`' resolver, `text-in-math`'s
/// embedded-box callback) is stored here OPAQUELY as a plain `Value` —
/// constructing one of these variants never *calls* such a closure, exactly
/// like upstream, where a `math` value is inert data until the real layout
/// engine walks it.
#[derive(Clone, Debug)]
pub enum Math {
    /// One base atom — a char run, a styled char, or embedded text. See
    /// [`MathElement`].
    Pure(MathElement),
    /// `math-group`: override the left/right math-class of a sub-`math`
    /// (`\mathbin`, `\mathrel`, …) — the two classes can differ (unlike
    /// every other variant here, which presents one class on both sides),
    /// which is exactly why upstream gives it its own node rather than
    /// folding it into `ChangeContext`.
    Group(MathKind, MathKind, Vec<Math>),
    /// `math-sup`: `base ^ script`.
    Sup(Vec<Math>, Vec<Math>),
    /// `math-sub`: `base _ script`.
    Sub(Vec<Math>, Vec<Math>),
    /// `math-color`.
    ChangeColor(Color, Vec<Math>),
    /// `math-char-class` (`\mathrm`/`\mathbf`/…) — the resolved
    /// [`MathCharClass`] a `math-char-class` primitive call named (`\mathrm`
    /// -> `MathRoman` -> `MathCharClass::Roman`, …). Its layout arm
    /// (`primitives.rs`) sets `Context::math_char_class` to this while
    /// laying out the inner list, which is what makes `VariantCharPending`'s
    /// per-char remap style-sensitive.
    ChangeCharClass(MathCharClass, Vec<Math>),
    /// `math-frac`: numerator, denominator.
    Fraction(Vec<Math>, Vec<Math>),
    /// `math-radical`: `\sqrt[degree]{radicand}` — `None` degree is the
    /// common `\sqrt` case (`math-radical None radicand`); upstream's own
    /// `MathRadicalWithDegree` is `failwith`-unimplemented too
    /// (`math.ml:886`), so a `Some` degree here is carried faithfully but
    /// never rendered specially (matches upstream by parity).
    Radical(Option<Vec<Math>>, Vec<Math>),
    /// `math-paren`: left/right paren-sizing closures (each a `paren =
    /// length -> length -> length -> length -> color -> inline-boxes *
    /// (length -> length)`, carried opaquely) plus the bracketed content.
    Paren(Box<Value>, Box<Value>, Vec<Math>),
    /// `math-paren-with-middle`: left/right/middle paren closures plus the
    /// `\setsep`-style list of bracketed sub-`math`s.
    ParenWithMiddle(Box<Value>, Box<Value>, Box<Value>, Vec<Vec<Math>>),
    /// `math-upper`: base with an over-script (`\overline`-adjacent, big-
    /// operator upper limit).
    UpperLimit(Vec<Math>, Vec<Math>),
    /// `math-lower`: base with an under-script (big-operator lower limit).
    LowerLimit(Vec<Math>, Vec<Math>),
    /// `math-pull-in-scripts`: a big operator's own left/right class plus
    /// the `(math option -> math option -> math)` resolver closure that
    /// routes an eventual `^`/`_` into limits instead of corner scripts
    /// (`\sum^n_i`-style). The closure is carried opaquely, same as
    /// `Paren`'s; only actually invoked by the real layout engine.
    PullInScripts(MathKind, MathKind, Box<Value>),
    /// V0_1 only: `read-math`'s captured reading context — the port's
    /// coarse-grained stand-in for upstream's
    /// per-node `context` fields (`types.cppo.ml:1051-1110`). Constructed
    /// ONLY by the V0_1 primitive `read-math`; no V0_0 path ever builds
    /// or matches this variant. Its layout arm (`primitives.rs`'s
    /// `layout_math_list`) lays `inner` out with ambient context = `*ctx`
    /// and size = `ctx.font_size` as an ABSOLUTE override — a `WithContext`
    /// produced under an `enter_script`ed context already carries the
    /// script-shrunk size, so the engine's own Sup/Sub shrink never
    /// double-applies to it.
    WithContext(Box<Context>, Vec<Math>),
}

/// The base-atom payload of [`Math::Pure`] — mirrors upstream
/// `math_element_main` (`types.cppo.ml:1009`), flattened (the math-class
/// lives directly on each variant here, rather than in a separate wrapping
/// `MathElement(kind, math_char_main)` layer) since nothing else needs the
/// undecorated `math_char_main` on its own.
#[derive(Clone, Debug)]
pub enum MathElement {
    /// `math-char` / `math-big-char`: a run of math characters, one atom.
    /// `big` selects the large-operator size class (`\sum`/`\int`-style;
    /// layout does not yet upscale it).
    Char {
        class: MathKind,
        big: bool,
        chars: String,
    },
    /// `math-char-with-kern` / `math-big-char-with-kern`: like `Char`, plus
    /// opaque left/right kern-function closures (each `length -> length ->
    /// length`, fontsize/y-position -> kern amount; `\int`'s
    /// italic-correction kern is the motivating case). Not yet consulted by
    /// layout.
    CharWithKern {
        class: MathKind,
        big: bool,
        chars: String,
        kern_l: Box<Value>,
        kern_r: Box<Value>,
    },
    /// `text-in-math` (`\text`, `\cases`): an embedded `context ->
    /// inline-boxes` closure, carried opaquely — the box it eventually
    /// produces isn't yet nestable into a math run's glyph model,
    /// so this is stored faithfully but not rendered.
    EmbeddedText { class: MathKind, body: Box<Value> },
    /// `math-variant-char` (`primitives.cppo.ml`'s `MathVariantCharDirect`)
    /// — one atom with a per-style codepoint set (Greek letters, `math.
    /// satyh`'s `greek-lowercase`/`greek-uppercase`). `big` mirrors `Char`'s
    /// (unused upstream for variant chars in practice, kept for shape
    /// parity).
    VariantChar {
        class: MathKind,
        big: bool,
        style: Box<MathVariantStyle>,
    },
    /// One MATHCHAR token from a `${…}` literal, not yet resolved to a
    /// `MathKind`/codepoint — `reflect_math_elem`'s `MathElem::Chars` arm
    /// pushes exactly one of these per token, deferring both the
    /// whole-token class-map lookup and the
    /// per-char variant remap to layout time, where the current
    /// `Context::font`/`math_char_class` are available to metrics-probe
    /// the remap (`resolve_variant_char`).
    VariantCharPending(String),
    /// V0_1 only: `embed-inline-to-math`'s payload — already-evaluated
    /// inline boxes carrying an explicit math class. Contrast
    /// `EmbeddedText`'s 0.0.6 closure (evaluated lazily at layout time
    /// under a `context`); this is eager, already-materialized data,
    /// matching upstream's `embed_inline_to_math` (which has no context to
    /// re-apply a closure under). Layout: the same stand-in rendering path
    /// `EmbeddedText` gets — `math_glyphs_of_inline_boxes` over
    /// `boxes` directly, no closure application.
    EmbeddedBoxes {
        class: MathKind,
        boxes: Vec<HorzBox>,
    },
}

/// `math-variant-char`'s 9-field per-style codepoint record
/// (`math.satyh`'s `greek-lowercase`/`greek-uppercase` build one per Greek
/// letter). Field order/names mirror the record literal math.satyh
/// constructs.
#[derive(Clone, Debug)]
pub struct MathVariantStyle {
    pub italic: String,
    pub bold_italic: String,
    pub roman: String,
    pub bold_roman: String,
    pub script: String,
    pub bold_script: String,
    pub fraktur: String,
    pub bold_fraktur: String,
    pub double_struck: String,
}

/// `text-info` (v0.0.6 `BCTextModeContext` carrying
/// `TextBackend.text_mode_context`, src/text-mode/textBackend.ml:1-5).
/// PDF-port sliver: upstream's second field, `escape_list`, is omitted —
/// no v0.0.6 primitive can set it (TextBackend.set_escape_list has no
/// vminst.ml caller), so it is invariantly `[]` upstream. `indent` is
/// invariantly >= 0 (`deepen_indent` clamps the increment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextInfo {
    pub indent: i64,
}

/// The final result of evaluating a document.
#[derive(Clone, Debug)]
pub struct DocumentValue {
    pub geometry: PageGeometry,
    pub pages: Vec<Page>,
    /// Every image `load-image` decoded while evaluating this document,
    /// indexed by `ImageId` (`PureHorzBox::Image::image` / `Value::Image`
    /// point in here). Moved out of `eval::Interp::images` by `page-break`
    /// when it packages the final document; threaded to
    /// `rustyfi_pdf::render_pdf`/`render_pdf_ttf` so the PDF writer can emit
    /// one Image XObject per image actually used.
    pub images: Vec<ImageResource>,
    /// Extras (annotations / destinations / outline / per-page deco
    /// overlays), attached by the compile driver AFTER the final trial's
    /// `fire_hooks` — `prim_page_break` cannot fill this (hooks/decos fire
    /// only after placement), so it packages `DocExtras::default()` and
    /// `compile_document_cst_with_trials` overwrites it on the winning trial.
    pub extras: DocExtras,
    /// Reflowable/semantic HTML side-channel: a clone of the
    /// flat `Vec<VertBox>` as it existed just BEFORE `page_break_core`
    /// handed it to `chop_page` — the document's natural linear flow, with
    /// paragraph boundaries (`Skip`), frame nesting (`FrameStart`/`FrameEnd`
    /// marker pairs), and `ClearPage` all intact, not yet sliced into pages
    /// or carrying injected headers/footers/footnotes. Populated
    /// unconditionally (one negligible `Vec<VertBox>` clone per compile,
    /// rather than threading a `want_reflow` flag through every entry
    /// point). `Option` keeps the field's meaning ("present only for a
    /// reflow-capable compile") self-documenting even though every current
    /// producer fills it.
    ///
    /// **Purely additive.** Neither `rustyfi_pdf::render_pdf*` nor the
    /// `html-support` branch's faithful HTML backend reads this field —
    /// only that branch's REFLOWABLE backend does.
    pub reflow_source: Option<Vec<VertBox>>,
    /// Links: one `(DecoId, action)` per `register-link-to-uri`/
    /// `-to-location` call made from inside a firing deco closure — needed
    /// because (unlike `extras.annotations`, which is page-absolute with no
    /// `DecoId`) this is what lets the reflow backend find which
    /// `PureHorzBox::Frame` in `reflow_source` a link belongs to.
    ///
    /// NOTE for whoever merges `html-support`: `\href`/`\ref` go through
    /// `inline-frame-breakable`, which no longer builds a `Frame` at all —
    /// it splices its contents between a `PureHorzBox::InlineFrameMarker`
    /// pair — so the reflow walker has to match the MARKER's `DecoId` (and
    /// wrap the boxes between the pair) rather than a `Frame`'s, or every
    /// link silently stops being wrapped. Filled in by `eval_document_trials`
    /// AFTER `fire_hooks`. Empty by default, same "purely additive" policy
    /// as `reflow_source`.
    pub reflow_links: Vec<(DecoId, AnnotAction)>,
    /// Same idea as `reflow_links`, for `register-destination`
    /// (`annot.satyh`'s `register-location-frame` idiom): `(DecoId, name)`.
    pub reflow_dests: Vec<(DecoId, String)>,
    /// Each frame's own decoration at its natural size, box-local
    /// (`FrameDecoration`). Same provenance and lifetime as `reflow_dests`:
    /// filled by `fire_hooks`, drained by `eval_document_trials`, read only
    /// by the reflowable HTML backend — which has no page grid and so cannot
    /// re-run a deco callback itself.
    ///
    /// BLOCK and INLINE frames both, in one table; `FrameDecoration::depth`
    /// says which, and the two are placed differently enough that a renderer
    /// must look. An inline frame's decoration IS the visual for a whole class
    /// of markup — `railway`'s `\uwave` is an `inline-frame-breakable` whose
    /// deco strokes the wave and whose contents are just the text under it —
    /// so recording only the block kind left those documents blank.
    pub reflow_frame_decos: Vec<(DecoId, FrameDecoration)>,
}

/// FxHash — the fast, NON-cryptographic hasher `rustc` uses (rustc-hash),
/// reimplemented here dependency-free. Variable lookup walks the environment
/// frame chain probing each frame's map by name (~192M probes on a graphics-
/// heavy doc); std's default SipHash is DoS-resistant but slow for these short,
/// non-adversarial identifier keys, and dominated the interpreter's runtime.
/// Processing 8/4/2/1 bytes at a step with a rotate-xor-multiply is ~3-5x
/// faster and is exactly what an internal, trusted env map wants.
#[derive(Default)]
struct FxHasher {
    hash: usize,
}

/// rustc-hash's multiplier, per pointer width — the golden ratio scaled to
/// 2^64 and to 2^32 respectively, which is the pair rustc-hash itself uses.
///
/// Spelled per width because the 64-bit literal does not FIT a 32-bit `usize`:
/// it silently truncates to a different (and much worse) constant, and rustc
/// rejects it outright. `wasm32-unknown-unknown` is a 32-bit target, which is
/// where this first mattered.
#[cfg(target_pointer_width = "64")]
const FX_SEED: usize = 0x51_7c_c1_b7_27_22_0a_95;
#[cfg(not(target_pointer_width = "64"))]
const FX_SEED: usize = 0x9e_37_79_b9;

impl FxHasher {
    #[inline]
    fn add(&mut self, i: usize) {
        self.hash = (self.hash.rotate_left(5) ^ i).wrapping_mul(FX_SEED);
    }
}

impl std::hash::Hasher for FxHasher {
    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        // One `usize` per step: 8 bytes on a 64-bit target, 4 on a 32-bit one.
        // The width was hard-coded as 8, which on a 32-bit build both
        // over-advanced the cursor and panicked in `try_into` — a 4-byte
        // `usize` cannot be built from an 8-byte slice. On 64-bit this is the
        // same sequence of `add` calls as before, byte for byte.
        const WIDE: usize = std::mem::size_of::<usize>();
        while bytes.len() >= WIDE {
            let (head, rest) = bytes.split_at(WIDE);
            self.add(usize::from_le_bytes(head.try_into().unwrap()));
            bytes = rest;
        }
        // Skipped where `usize` is already 4 bytes: the loop above consumed
        // every whole 4-byte group there.
        if WIDE > 4 && bytes.len() >= 4 {
            self.add(u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize);
            bytes = &bytes[4..];
        }
        if bytes.len() >= 2 {
            self.add(u16::from_le_bytes(bytes[..2].try_into().unwrap()) as usize);
            bytes = &bytes[2..];
        }
        if let Some(&b) = bytes.first() {
            self.add(b as usize);
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as usize);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash as u64
    }
}

#[derive(Default, Clone)]
struct FxBuild;
impl std::hash::BuildHasher for FxBuild {
    type Hasher = FxHasher;
    #[inline]
    fn build_hasher(&self) -> FxHasher {
        FxHasher::default()
    }
}

type FxMap = HashMap<Rc<str>, Value, FxBuild>;

/// The **compile-time** environment: the flat name -> value table of
/// primitives and base constants that `crate::compile` folds unshadowed
/// references against, and whose [`BaseEnv::names`] seed the elaborator's
/// scope.
///
/// This is deliberately NOT the runtime environment, and is not the root of
/// the runtime frame chain. Nothing resolves a name at run time —
/// top-level bindings go through the compiler's `Globals` table, locals
/// through slot indices, and unshadowed base names are constant-folded at
/// compile time — so the two are what they actually are: a name map used
/// while compiling, and a stack of positional frames used while running.
#[derive(Clone, Debug, Default)]
pub struct BaseEnv {
    vars: FxMap,
}

impl BaseEnv {
    pub fn new() -> BaseEnv {
        BaseEnv::default()
    }

    /// A copy that can be extended without disturbing this one. There is no
    /// frame chain here — shadowing is just overwriting in the copy.
    pub fn child(&self) -> BaseEnv {
        self.clone()
    }

    pub fn define(&mut self, name: impl Into<Rc<str>>, value: Value) {
        self.vars.insert(name.into(), value);
    }

    pub fn lookup(&self, name: &str) -> Option<Value> {
        self.vars.get(name).cloned()
    }

    /// Every name bound here (feeds the elaborator's scope).
    pub fn names(&self) -> Vec<String> {
        self.vars.keys().map(|k| k.to_string()).collect()
    }
}

/// The **runtime** environment: a chain of positional frames.
///
/// A frame is a plain `Vec<Value>`, and a compiled variable reference
/// is a `(depth, index)` pair resolved at compile time — walk `depth` parents,
/// index the vector. There are no names here at all: the compiler's scope
/// stack is 1:1 with this chain (it pushes exactly where a frame is created),
/// so every local is a static coordinate.
///
/// `RefCell` because `let rec` back-patches its siblings into a shared frame
/// one at a time: the frame is created pre-sized with placeholders and filled
/// in order, and a closure that captured it sees the later fills.
#[derive(Clone, Debug)]
pub struct Env(Rc<Frame>);

#[derive(Debug)]
struct Frame {
    slots: RefCell<Vec<Value>>,
    parent: Option<Env>,
}

impl Env {
    /// The empty root frame every program runs in.
    pub fn root() -> Env {
        Env(Rc::new(Frame {
            slots: RefCell::new(Vec::new()),
            parent: None,
        }))
    }

    /// Push a frame holding `slots`, in the order the compiler assigned them.
    pub fn child(&self, slots: Vec<Value>) -> Env {
        Env(Rc::new(Frame {
            slots: RefCell::new(slots),
            parent: Some(self.clone()),
        }))
    }

    #[inline]
    fn frame_at(&self, depth: u16) -> &Frame {
        let mut f = self;
        for _ in 0..depth {
            f =
                f.0.parent
                    .as_ref()
                    .expect("compiled slot depth exceeds the runtime frame chain");
        }
        &f.0
    }

    /// Read the local at `(depth, index)`.
    #[inline]
    pub fn slot(&self, depth: u16, index: u16) -> Value {
        self.frame_at(depth).slots.borrow()[index as usize].clone()
    }

    /// Overwrite the local at `(depth, index)` — `let rec`'s back-patch.
    #[inline]
    pub fn set_slot(&self, depth: u16, index: u16, value: Value) {
        self.frame_at(depth).slots.borrow_mut()[index as usize] = value;
    }
}
