use crate::span::Span;

/// One SATySFi token. Variant names and payloads mirror the token declarations
/// of the v0.0.6 `parser.mly` (`LETNONREC` = `Let`, `LAMBDA` = `Fun`, ...).
/// Command payloads keep their sigil, exactly like the OCaml lexer
/// (`HorzCmd("\\emph")`, `VertCmd("+p")`).
///
/// The variants listed in the `token_leaves!` block at the bottom of this file
/// get a matching leaf struct (`KwLet`, `VarTok`, ...) with a peek → match →
/// push-back-on-mismatch `Parse`/`Unparse`/`Spanned`. Multi-field and
/// multi-variant-matching leaves (`LengthTok`, `LiteralTok`, `BinOpTok`,
/// `VarInHorzTok`, ...) stay hand-written in [`crate::leaf`].
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    // ---- headers ----
    HeaderRequire(String),
    HeaderImport(String),
    HeaderStage0,
    HeaderStage1,
    HeaderPersistent0,

    // ---- identifiers & constants (program mode) ----
    Var(String),
    VarWithMod(Vec<String>, String),
    Constructor(String),
    TypeVar(String),
    OpenModule(String), // `Mod.(`
    /// `M.N.P` — a dotted module path ending in an UPPER segment
    /// (upstream `LONG_UPPER`, `lexer_v1.mll:357-363`). SATySFi 0.1-only:
    /// under 0.0.6 this spelling is a lex error ("module path must end
    /// with a variable name"), unchanged. Payload mirrors
    /// [`Token::VarWithMod`]: `mods = ["M","N"]`, final segment `"P"` —
    /// upstream's `(modidents, modident0)` pair carries the same split
    /// (`parser_v1.mly:407-413` reassembles the chain from it).
    LongUpper(Vec<String>, String),
    IntConst(i64),
    FloatConst(f64),
    LengthConst(f64, String),
    /// Backtick string literal. `omit_pre`/`omit_post` mirror the OCaml
    /// `LITERAL(_, s, pre, post)` flags (whether an adjacent space is trimmed).
    Literal {
        body: String,
        omit_pre: bool,
        omit_post: bool,
    },

    // ---- keywords ----
    //
    // NOTE: `not` deliberately has NO reserved keyword token here. It lexes
    // as an ordinary `Var`/`VarTok`, exactly like any other primitive name
    // (`arabic`, `floor`, ...), so `not expr` works through the existing
    // `AppExpr` application machinery with no grammar rule of its own. A
    // reserved `Not` token would have none: nothing in the grammar could
    // consume it, which is what made the registered `"not"` primitive
    // (`primitives.rs`) unreachable from real syntax until `picture.satyh`'s
    // `not (x1r >' x2r)`/`not reversed` surfaced it.
    Mod,
    If,
    Then,
    Else,
    Let, // `let` (LETNONREC)
    LetRec, // `let-rec`
    LetAnd, // `and`
    In,
    Fun, // `fun` (LAMBDA)
    True,
    False,
    Before,
    While,
    Do,
    LetMutable, // `let-mutable`
    Match,
    With,
    When,
    As,
    Type,
    Of,
    Module,
    Struct,
    Sig,
    Val,
    End,
    Direct,
    Constraint,
    LetHorz, // `let-inline`
    LetVert, // `let-block`
    LetMath, // `let-math`
    Controls,
    Cycle,
    HorzCmdType, // `inline-cmd`
    VertCmdType, // `block-cmd`
    MathCmdType, // `math-cmd`
    Command,
    Open,
    /// `rec` — SATySFi 0.1-only keyword (`val rec`/`let rec`); under 0.0.6
    /// this word stays a plain identifier (see `lexer.rs`'s version-gated
    /// keyword table).
    Rec,
    /// `inline` — SATySFi 0.1-only keyword (`val inline \cmd = ...`).
    Inline,
    /// `block` — SATySFi 0.1-only keyword (`val block +cmd = ...`).
    Block,
    /// `mutable` — SATySFi 0.1-only keyword (`val mutable x <- e`/`let
    /// mutable x <- e in ..`); under 0.0.6 this word stays a plain
    /// identifier (see `lexer.rs`'s version-gated keyword table).
    Mutable,
    /// `signature` — SATySFi 0.1-only keyword (`signature S = sig … end`,
    /// upstream `lexer_v1.mll:348`); a plain identifier under 0.0.6.
    Signature,
    /// `include` — SATySFi 0.1-only keyword (`include M` binds /
    /// `include S` decls, `lexer_v1.mll:335`); a plain identifier under
    /// 0.0.6.
    Include,
    /// `use` — SATySFi 0.1-only keyword (Envelopes packaging headers `use
    /// package …` / `use … of <path>`, `saphe-split:parser.mly:371-380 @
    /// b836d512`); a plain identifier under 0.0.6 (no 0.0.6 grammar reserves
    /// it — see `lexer.rs`'s version-gated keyword table).
    Use,
    /// `package` — SATySFi 0.1-only keyword (`use package …`); a plain
    /// identifier under 0.0.6.
    Package,
    /// `math` — SATySFi 0.1-only keyword (`val math <ctx> \cmd … = …`,
    /// `parser_v1.mly:452-453` dispatch, MATH reserved at :240); a plain
    /// identifier under 0.0.6 (0.0.6 has no `math` keyword — the word
    /// survives only as the surface name of `BaseType::MathText`).
    Math,
    /// `persistent` — SATySFi 0.1-only keyword, the stage qualifier of a
    /// `val persistent ~x = e` binding (`parser_v1.mly:420-421`, decl form
    /// `:602-603`; reserved at `:241`, lexed at `lexer_v1.mll:345`). A plain
    /// identifier under 0.0.6, which spells the same idea per FILE with a
    /// `@stage:` header rather than per binding.
    Persistent,

    // ---- grouping delimiters ----
    LParen,
    RParen,
    BRecord, // `(|`
    ERecord, // `|)`
    BList, // `[`
    EList, // `]`
    BHorzGrp, // `{` opening inline text
    EHorzGrp, // `}` closing inline text
    BVertGrp, // `'<` / `<` opening block text
    EVertGrp, // `>` closing block text
    BMathGrp, // `${` / `{` opening math
    EMathGrp, // `}` closing math
    BPath, // `<[`
    EPath, // `]>`

    // ---- punctuation & operators (program mode) ----
    ListPunct, // `;`
    Access, // `#`
    Arrow, // `->`
    OverwriteEq, // `<-`
    Bar, // `|`
    Wildcard, // `_`
    Colon,
    Comma,
    Cons, // `::`
    /// `:>` — signature coercion/ascription (COERCE, `lexer_v1.mll:280`).
    /// SATySFi 0.1-only: under 0.0.6 the same two characters lex as
    /// `Colon` + `BinopGt(">")`, unchanged.
    Coerce,
    ExactMinus,
    DefEq, // `=`
    ExactTimes, // `*`
    ExactAmp,   // `&`
    ExactTilde, // `~`
    PathCurve,  // `..`
    PathLine,   // `--`
    BinopPlus(String),
    BinopMinus(String),
    BinopTimes(String),
    BinopDivides(String),
    BinopEq(String),
    BinopLt(String),
    BinopGt(String),
    BinopAmp(String),
    BinopBar(String),
    BinopHat(String),
    UnopExclam(String),
    OptionalType, // `?`
    OptionalArrow, // `?->`
    Optional, // `?:`
    Omission, // `?*`
    /// `?'r` — a SATySFi 0.1 row variable (upstream `ROWVAR`,
    /// `lexer_v1.mll:310-311`). `RowVarTok`
    /// carries the name sans sigil, exactly like [`Token::TypeVar`]/
    /// `TypeVarTok`. V0_1-only: the lexer (`lexer.rs`'s `'?'` arm) only ever
    /// mints this under `RustyfiVersion::V0_1`; under `V0_0`, `?'r` stays
    /// two tokens (`OptionalType` then `TypeVar`), byte-identical to before
    /// this addition.
    RowVar(String), // `?'r`

    // ---- commands (payload includes the `\`/`+` sigil) ----
    HorzCmd(String),
    HorzCmdWithMod(Vec<String>, String),
    HorzMacro(String),
    VertCmd(String),
    VertCmdWithMod(Vec<String>, String),
    VertMacro(String),
    MathCmd(String),
    MathCmdWithMod(Vec<String>, String),

    // ---- text-mode content ----
    VarInHorz(Vec<String>, String),
    VarInVert(Vec<String>, String),
    VarInMath(Vec<String>, String),
    Char(String),
    /// A backtick literal written INSIDE inline text (`` `…` ``). Distinct from
    /// `Char` because upstream keeps it distinct: it reaches the evaluator as
    /// `ImInputHorzEmbeddedCodeText` and is dispatched to the context's
    /// `code_text_command` (`evaluator.cppo.ml:768-779`), which is how a doc
    /// class sets code spans in a monospace face. Do NOT lex it to a plain
    /// `Char` run: that erases the distinction irrecoverably, and the literal
    /// then inherits whatever face surrounds it (italic, inside `\emph`).
    CodeText(String),
    Space,
    Break,
    Item(usize), // `*`+ with depth
    Sep, // `|`
    EndActive, // `;` closing an active area

    // ---- math-mode content ----
    MathChar(String),
    Superscript, // `^`
    Subscript, // `_`
    Primes(usize),

    Eoi,
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use Token::*;
        match self {
            HeaderRequire(s) => write!(f, "@require: {s}"),
            HeaderImport(s) => write!(f, "@import: {s}"),
            HeaderStage0 => write!(f, "@stage: 0"),
            HeaderStage1 => write!(f, "@stage: 1"),
            HeaderPersistent0 => write!(f, "@stage: persistent"),
            Var(s) | Constructor(s) => write!(f, "{s}"),
            VarWithMod(m, s) | LongUpper(m, s) => write!(f, "{}.{s}", m.join(".")),
            TypeVar(s) => write!(f, "'{s}"),
            OpenModule(s) => write!(f, "{s}.("),
            IntConst(n) => write!(f, "{n}"),
            FloatConst(x) => write!(f, "{x}"),
            LengthConst(x, u) => write!(f, "{x}{u}"),
            Literal { body, .. } => write!(f, "`{body}`"),
            Mod => write!(f, "mod"),
            If => write!(f, "if"),
            Then => write!(f, "then"),
            Else => write!(f, "else"),
            Let => write!(f, "let"),
            LetRec => write!(f, "let-rec"),
            LetAnd => write!(f, "and"),
            In => write!(f, "in"),
            Fun => write!(f, "fun"),
            True => write!(f, "true"),
            False => write!(f, "false"),
            Before => write!(f, "before"),
            While => write!(f, "while"),
            Do => write!(f, "do"),
            LetMutable => write!(f, "let-mutable"),
            Match => write!(f, "match"),
            With => write!(f, "with"),
            When => write!(f, "when"),
            As => write!(f, "as"),
            Type => write!(f, "type"),
            Of => write!(f, "of"),
            Module => write!(f, "module"),
            Struct => write!(f, "struct"),
            Sig => write!(f, "sig"),
            Val => write!(f, "val"),
            End => write!(f, "end"),
            Direct => write!(f, "direct"),
            Constraint => write!(f, "constraint"),
            LetHorz => write!(f, "let-inline"),
            LetVert => write!(f, "let-block"),
            LetMath => write!(f, "let-math"),
            Controls => write!(f, "controls"),
            Cycle => write!(f, "cycle"),
            HorzCmdType => write!(f, "inline-cmd"),
            VertCmdType => write!(f, "block-cmd"),
            MathCmdType => write!(f, "math-cmd"),
            Command => write!(f, "command"),
            Open => write!(f, "open"),
            Rec => write!(f, "rec"),
            Inline => write!(f, "inline"),
            Block => write!(f, "block"),
            Mutable => write!(f, "mutable"),
            Signature => write!(f, "signature"),
            Include => write!(f, "include"),
            Use => write!(f, "use"),
            Package => write!(f, "package"),
            Math => write!(f, "math"),
            Persistent => write!(f, "persistent"),
            LParen => write!(f, "("),
            RParen => write!(f, ")"),
            BRecord => write!(f, "(|"),
            ERecord => write!(f, "|)"),
            BList => write!(f, "["),
            EList => write!(f, "]"),
            BHorzGrp => write!(f, "{{"),
            EHorzGrp => write!(f, "}}"),
            BVertGrp => write!(f, "'<"),
            EVertGrp => write!(f, ">"),
            BMathGrp => write!(f, "${{"),
            EMathGrp => write!(f, "}}"),
            BPath => write!(f, "<["),
            EPath => write!(f, "]>"),
            ListPunct => write!(f, ";"),
            Access => write!(f, "#"),
            Arrow => write!(f, "->"),
            OverwriteEq => write!(f, "<-"),
            Bar => write!(f, "|"),
            Wildcard => write!(f, "_"),
            Colon => write!(f, ":"),
            Comma => write!(f, ","),
            Cons => write!(f, "::"),
            Coerce => write!(f, ":>"),
            ExactMinus => write!(f, "-"),
            DefEq => write!(f, "="),
            ExactTimes => write!(f, "*"),
            ExactAmp => write!(f, "&"),
            ExactTilde => write!(f, "~"),
            PathCurve => write!(f, ".."),
            PathLine => write!(f, "--"),
            BinopPlus(s) | BinopMinus(s) | BinopTimes(s) | BinopDivides(s) | BinopEq(s)
            | BinopLt(s) | BinopGt(s) | BinopAmp(s) | BinopBar(s) | BinopHat(s)
            | UnopExclam(s) => write!(f, "{s}"),
            OptionalType => write!(f, "?"),
            OptionalArrow => write!(f, "?->"),
            Optional => write!(f, "?:"),
            Omission => write!(f, "?*"),
            RowVar(s) => write!(f, "?'{s}"),
            HorzCmd(s) | VertCmd(s) | MathCmd(s) | HorzMacro(s) | VertMacro(s) => {
                write!(f, "{s}")
            }
            HorzCmdWithMod(m, s) | VertCmdWithMod(m, s) | MathCmdWithMod(m, s) => {
                let (sigil, name) = s.split_at(1);
                write!(f, "{sigil}{}.{name}", m.join("."))
            }
            VarInHorz(m, s) | VarInVert(m, s) | VarInMath(m, s) => {
                if m.is_empty() {
                    write!(f, "#{s}")
                } else {
                    write!(f, "#{}.{s}", m.join("."))
                }
            }
            Char(s) => write!(f, "{s}"),
            CodeText(s) => write!(f, "`{s}`"),
            Space => write!(f, " "),
            Break => writeln!(f),
            Item(n) => write!(f, "{}", "*".repeat(*n)),
            Sep => write!(f, "|"),
            EndActive => write!(f, ";"),
            MathChar(s) => write!(f, "{s}"),
            Superscript => write!(f, "^"),
            Subscript => write!(f, "_"),
            Primes(n) => write!(f, "{}", "'".repeat(*n)),
            Eoi => Ok(()),
        }
    }
}

/// A token together with its source span: the atom type fed to the syan parser.
pub type Atom = syan::span::WithSpan<Token, Span>;

// Leaf structs + `Parse`/`Unparse`/`Spanned`, one per listed variant. Stands
// in for syan's `#[derive(TokenLeaves)]`, which exists only on that crate's
// api-ergonomics line and is absent from main; see `leaf_macro.rs`.
token_leaves! {
    atom = Atom, span = Span, read_span = |a| a.span;
    (HeaderRequire(String) => HeaderRequireTok, "'@require:'", field = content);
    (HeaderImport(String) => HeaderImportTok, "'@import:'", field = content);
    (Var(String) => VarTok, "a variable name", field = name);
    (Constructor(String) => CtorTok, "a constructor name", field = name);
    (TypeVar(String) => TypeVarTok, "a type variable", field = name);
    (OpenModule(String) => OpenModuleTok, "'Mod.('", field = name);
    (IntConst(i64) => IntTok, "an integer constant", field = value);
    (FloatConst(f64) => FloatTok, "a float constant", field = value);
    (If => KwIf, "'if'");
    (Then => KwThen, "'then'");
    (Else => KwElse, "'else'");
    (Let => KwLet, "'let'");
    (LetRec => KwLetRec, "'let-rec'");
    (LetAnd => KwAnd, "'and'");
    (In => KwIn, "'in'");
    (Fun => KwFun, "'fun'");
    (True => KwTrue, "'true'");
    (False => KwFalse, "'false'");
    (Before => KwBefore, "'before'");
    (While => KwWhile, "'while'");
    (Do => KwDo, "'do'");
    (LetMutable => KwLetMutable, "'let-mutable'");
    (Match => KwMatch, "'match'");
    (With => KwWith, "'with'");
    (When => KwWhen, "'when'");
    (As => KwAs, "'as'");
    (Type => KwType, "'type'");
    (Of => KwOf, "'of'");
    (Module => KwModule, "'module'");
    (Struct => KwStruct, "'struct'");
    (Sig => KwSig, "'sig'");
    (Val => KwVal, "'val'");
    (End => KwEnd, "'end'");
    (Direct => KwDirect, "'direct'");
    (Constraint => ConstraintTok, "'constraint'");
    (LetHorz => KwLetHorz, "'let-inline'");
    (LetVert => KwLetVert, "'let-block'");
    (LetMath => KwLetMath, "'let-math'");
    (HorzCmdType => HorzCmdTypeTok, "'inline-cmd'");
    (VertCmdType => VertCmdTypeTok, "'block-cmd'");
    (MathCmdType => MathCmdTypeTok, "'math-cmd'");
    (Command => CommandTok, "'command'");
    (Open => KwOpen, "'open'");
    (Rec => KwRec, "'rec'");
    (Inline => KwInline, "'inline'");
    (Block => KwBlock, "'block'");
    (Mutable => KwMutable, "'mutable'");
    (Signature => KwSignature, "'signature'");
    (Include => KwInclude, "'include'");
    (Use => KwUse, "'use'");
    (Package => KwPackage, "'package'");
    (Math => KwMath, "'math'");
    (Persistent => KwPersistent, "'persistent'");
    (LParen => LParenTok, "'('");
    (RParen => RParenTok, "')'");
    (BRecord => BRecordTok, "'(|'");
    (ERecord => ERecordTok, "'|)'");
    (BList => BListTok, "'['");
    (EList => EListTok, "']'");
    (BHorzGrp => BHorzGrpTok, "'{'");
    (EHorzGrp => EHorzGrpTok, "'}'");
    (BVertGrp => BVertGrpTok, "'<'");
    (EVertGrp => EVertGrpTok, "'>'");
    (BMathGrp => BMathGrpTok, "'${'");
    (EMathGrp => EMathGrpTok, "'}'");
    (ListPunct => ListPunctTok, "';'");
    (Access => AccessTok, "'#'");
    (Arrow => ArrowTok, "'->'");
    (OverwriteEq => OverwriteEqTok, "'<-'");
    (Bar => BarTok, "'|'");
    (Wildcard => WildcardTok, "'_'");
    (Colon => ColonTok, "':'");
    (Comma => CommaTok, "','");
    (Cons => ConsTok, "'::'");
    (Coerce => CoerceTok, "':>'");
    (ExactMinus => ExactMinusTok, "'-'");
    (DefEq => DefEqTok, "'='");
    (ExactTimes => ExactTimesTok, "'*'");
    (ExactAmp => ExactAmpTok, "'&'");
    (ExactTilde => ExactTildeTok, "'~'");
    (UnopExclam(String) => UnopExclamTok, "a '!' operator", field = text);
    (OptionalType => OptionalTypeTok, "'?'");
    (OptionalArrow => OptionalArrowTok, "'?->'");
    (Optional => OptionalTok, "'?:'");
    (Omission => OmissionTok, "'?*'");
    (RowVar(String) => RowVarTok, "a row variable (\"?'r\")", field = name);
    (HorzCmd(String) => HorzCmdTok, "an inline command", field = name);
    (VertCmd(String) => VertCmdTok, "a block command", field = name);
    (MathCmd(String) => MathCmdTok, "a math command", field = name);
    (Char(String) => CharTok, "inline text", field = text);
    (CodeText(String) => CodeTextTok, "an inline code literal", field = text);
    (Space => SpaceTok, "a space");
    (Break => BreakTok, "a line break");
    (Item(usize) => ItemTok, "an itemize bullet", field = depth);
    (Sep => SepTok, "'|' separator");
    (EndActive => EndActiveTok, "';'");
    (MathChar(String) => MathCharTok, "a math character", field = text);
    (Superscript => SuperscriptTok, "'^'");
    (Subscript => SubscriptTok, "'_'");
    (Primes(usize) => PrimesTok, "a primes mark", field = count);
    (Eoi => EoiTok, "end of input");
}
