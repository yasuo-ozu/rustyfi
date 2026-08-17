use crate::span::Span;

/// One SATySFi token. Variant names and payloads mirror the token declarations
/// of the v0.0.6 `parser.mly` (`LETNONREC` = `Let`, `LAMBDA` = `Fun`, ...).
/// Command payloads keep their sigil, exactly like the OCaml lexer
/// (`HorzCmd("\\emph")`, `VertCmd("+p")`).
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
    Not,
    Mod,
    If,
    Then,
    Else,
    Let,        // `let` (LETNONREC)
    LetRec,     // `let-rec`
    LetAnd,     // `and`
    In,
    Fun,        // `fun` (LAMBDA)
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
    LetHorz,    // `let-inline`
    LetVert,    // `let-block`
    LetMath,    // `let-math`
    Controls,
    Cycle,
    HorzCmdType, // `inline-cmd`
    VertCmdType, // `block-cmd`
    MathCmdType, // `math-cmd`
    Command,
    Open,

    // ---- grouping delimiters ----
    LParen,
    RParen,
    BRecord, // `(|`
    ERecord, // `|)`
    BList,   // `[`
    EList,   // `]`
    BHorzGrp, // `{` opening inline text
    EHorzGrp, // `}` closing inline text
    BVertGrp, // `'<` / `<` opening block text
    EVertGrp, // `>` closing block text
    BMathGrp, // `${` / `{` opening math
    EMathGrp, // `}` closing math
    BPath,   // `<[`
    EPath,   // `]>`

    // ---- punctuation & operators (program mode) ----
    ListPunct, // `;`
    Access,    // `#`
    Arrow,     // `->`
    OverwriteEq, // `<-`
    Bar,       // `|`
    Wildcard,  // `_`
    Colon,
    Comma,
    Cons, // `::`
    ExactMinus,
    DefEq,      // `=`
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
    OptionalType,  // `?`
    OptionalArrow, // `?->`
    Optional,      // `?:`
    Omission,      // `?*`

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
    Space,
    Break,
    Item(usize), // `*`+ with depth
    Sep,         // `|`
    EndActive,   // `;` closing an active area

    // ---- math-mode content ----
    MathChar(String),
    Superscript, // `^`
    Subscript,   // `_`
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
            VarWithMod(m, s) => write!(f, "{}.{s}", m.join(".")),
            TypeVar(s) => write!(f, "'{s}"),
            OpenModule(s) => write!(f, "{s}.("),
            IntConst(n) => write!(f, "{n}"),
            FloatConst(x) => write!(f, "{x}"),
            LengthConst(x, u) => write!(f, "{x}{u}"),
            Literal { body, .. } => write!(f, "`{body}`"),
            Not => write!(f, "not"),
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
