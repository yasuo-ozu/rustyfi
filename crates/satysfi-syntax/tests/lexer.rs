use satysfi_syntax::{lex, Token};
use Token::*;

fn toks(src: &str) -> Vec<Token> {
    lex(src)
        .unwrap_or_else(|e| panic!("lex failed on {src:?}: {e}"))
        .into_iter()
        .map(|a| a.slot)
        .collect()
}

fn var(s: &str) -> Token {
    Var(s.into())
}
fn ch(s: &str) -> Token {
    Char(s.into())
}

#[test]
fn program_basics() {
    assert_eq!(
        toks("let x = 3 in x"),
        vec![Let, var("x"), DefEq, IntConst(3), In, var("x"), Eoi]
    );
    assert_eq!(
        toks("let-inline f = fun y -> y"),
        vec![LetHorz, var("f"), DefEq, Fun, var("y"), Arrow, var("y"), Eoi]
    );
    assert_eq!(toks("% only a comment\n"), vec![Eoi]);
}

#[test]
fn field_access_token() {
    // `#` in program mode lexes as a bare `Access` token (not merged with
    // the following name).
    assert_eq!(toks("x#y"), vec![var("x"), Access, var("y"), Eoi]);
    assert_eq!(
        toks("x#y#z"),
        vec![var("x"), Access, var("y"), Access, var("z"), Eoi]
    );
}

#[test]
fn numbers_and_lengths() {
    assert_eq!(
        toks("3 3.5 .5 -3pt 3pt 3.5cm -.5in 0x1F"),
        vec![
            IntConst(3),
            FloatConst(3.5),
            FloatConst(0.5),
            LengthConst(-3.0, "pt".into()),
            LengthConst(3.0, "pt".into()),
            LengthConst(3.5, "cm".into()),
            LengthConst(-0.5, "in".into()),
            IntConst(31),
            Eoi,
        ]
    );
    // `-3` without a unit is minus, then int.
    assert_eq!(toks("-3"), vec![ExactMinus, IntConst(3), Eoi]);
}

#[test]
fn operators() {
    assert_eq!(
        toks("+ - * / = < > & | ^ ! -> <- :: : .. -- ?: ?* ?-> ?"),
        vec![
            BinopPlus("+".into()),
            ExactMinus,
            ExactTimes,
            BinopDivides("/".into()),
            DefEq,
            BinopLt("<".into()),
            BinopGt(">".into()),
            ExactAmp,
            Bar,
            BinopHat("^".into()),
            UnopExclam("!".into()),
            Arrow,
            OverwriteEq,
            Cons,
            Colon,
            PathCurve,
            PathLine,
            Optional,
            Omission,
            OptionalArrow,
            OptionalType,
            Eoi,
        ]
    );
    assert_eq!(
        toks("a +. b"),
        vec![var("a"), BinopPlus("+.".into()), var("b"), Eoi]
    );
}

#[test]
fn headers() {
    assert_eq!(
        toks("@require: stdjabook\n@import: local\n3"),
        vec![
            HeaderRequire("stdjabook".into()),
            HeaderImport("local".into()),
            IntConst(3),
            Eoi,
        ]
    );
    assert_eq!(toks("@stage: 1\n3"), vec![HeaderStage1, IntConst(3), Eoi]);
}

#[test]
fn string_literals() {
    assert_eq!(
        toks("`plain`"),
        vec![
            Literal {
                body: "plain".into(),
                omit_pre: true,
                omit_post: true
            },
            Eoi
        ]
    );
    // ``..`..`` : inner single backtick kept; `#`-decorations flip the flags.
    assert_eq!(
        toks("``a ` b``"),
        vec![
            Literal {
                body: "a ` b".into(),
                omit_pre: true,
                omit_post: true
            },
            Eoi
        ]
    );
    assert_eq!(
        toks("#`x`#"),
        vec![
            Literal {
                body: "x".into(),
                omit_pre: false,
                omit_post: false
            },
            Eoi
        ]
    );
}

#[test]
fn inline_text_mode() {
    assert_eq!(
        toks("{ Hello, world! }"),
        vec![BHorzGrp, ch("Hello,"), Space, ch("world!"), EHorzGrp, Eoi]
    );
    // Leading spaces after `{` are skipped; spaces before `}` are swallowed.
    assert_eq!(
        toks("{  a  }"),
        vec![BHorzGrp, ch("a"), EHorzGrp, Eoi]
    );
    // Breaks between words produce Break.
    assert_eq!(
        toks("{a\nb}"),
        vec![BHorzGrp, ch("a"), Break, ch("b"), EHorzGrp, Eoi]
    );
    // Escaped symbol characters become plain chars.
    assert_eq!(
        toks(r"{a\{b\}c\\d}"),
        vec![
            BHorzGrp,
            ch("a"),
            ch("{"),
            ch("b"),
            ch("}"),
            ch("c"),
            ch("\\"),
            ch("d"),
            EHorzGrp,
            Eoi
        ]
    );
}

#[test]
fn inline_command_with_args() {
    assert_eq!(
        toks(r"{a \emph{b} c}"),
        vec![
            BHorzGrp,
            ch("a"),
            Space,
            HorzCmd(r"\emph".into()),
            BHorzGrp,
            ch("b"),
            EHorzGrp,
            Space,
            ch("c"),
            EHorzGrp,
            Eoi
        ]
    );
    // Program-mode arguments to a command in active mode, terminated by `;`.
    assert_eq!(
        toks(r"{\skip(3pt);x}"),
        vec![
            BHorzGrp,
            HorzCmd(r"\skip".into()),
            LParen,
            LengthConst(3.0, "pt".into()),
            RParen,
            EndActive,
            ch("x"),
            EHorzGrp,
            Eoi
        ]
    );
    // `#var;` embedding in inline text.
    assert_eq!(
        toks("{#name;}"),
        vec![
            BHorzGrp,
            VarInHorz(vec![], "name".into()),
            EndActive,
            EHorzGrp,
            Eoi
        ]
    );
}

#[test]
fn block_mode_and_active() {
    assert_eq!(
        toks("'< +p { hi } >"),
        vec![
            BVertGrp,
            VertCmd("+p".into()),
            BHorzGrp,
            ch("hi"),
            EHorzGrp,
            EVertGrp,
            Eoi
        ]
    );
    // A block command taking a program arg then a block-text arg.
    assert_eq!(
        toks("'<+sec(1)<+p{a}>>"),
        vec![
            BVertGrp,
            VertCmd("+sec".into()),
            LParen,
            IntConst(1),
            RParen,
            BVertGrp,
            VertCmd("+p".into()),
            BHorzGrp,
            ch("a"),
            EHorzGrp,
            EVertGrp,
            EVertGrp,
            Eoi
        ]
    );
    // A block command with no args, terminated by `;`.
    assert_eq!(
        toks("'<+clear;>"),
        vec![BVertGrp, VertCmd("+clear".into()), EndActive, EVertGrp, Eoi]
    );
}

#[test]
fn records_and_lists() {
    assert_eq!(
        toks("(| title = {T}; ok = true |)"),
        vec![
            BRecord,
            var("title"),
            DefEq,
            BHorzGrp,
            ch("T"),
            EHorzGrp,
            ListPunct,
            var("ok"),
            DefEq,
            True,
            ERecord,
            Eoi
        ]
    );
    assert_eq!(
        toks("[1; 2]"),
        vec![BList, IntConst(1), ListPunct, IntConst(2), EList, Eoi]
    );
}

#[test]
fn math_mode() {
    assert_eq!(
        toks(r"${x^2 + \frac{a}{b}}"),
        vec![
            BMathGrp,
            MathChar("x".into()),
            Superscript,
            MathChar("2".into()),
            MathChar("+".into()),
            MathCmd(r"\frac".into()),
            BMathGrp,
            MathChar("a".into()),
            EMathGrp,
            BMathGrp,
            MathChar("b".into()),
            EMathGrp,
            EMathGrp,
            Eoi
        ]
    );
    assert_eq!(
        toks("${a_1'}"),
        vec![
            BMathGrp,
            MathChar("a".into()),
            Subscript,
            MathChar("1".into()),
            Primes(1),
            EMathGrp,
            Eoi
        ]
    );
}

#[test]
fn math_mode_qualified_command() {
    assert_eq!(
        toks(r"${\Mod.cmd}"),
        vec![
            BMathGrp,
            MathCmdWithMod(vec!["Mod".into()], r"\cmd".into()),
            EMathGrp,
            Eoi
        ]
    );
}

/// Language-completeness sweep gap 4: `command \Mod.cmd` in PROGRAM
/// position (e.g. `(command \Mod.cmd)`, a first-class `command`-value) must
/// lex the module-qualified name as one token, exactly like inline-text
/// mode's own `\Mod.cmd` ([`math_mode_qualified_command`]'s twin) already
/// does — not split into a bare `\Mod` command followed by a stray `.cmd`.
/// An unqualified `\cmd` in the same position is unaffected (still a plain
/// `HorzCmd`, same as [`inline_command_with_args`]'s `\emph`/`\skip`).
#[test]
fn program_mode_qualified_command() {
    assert_eq!(
        toks(r"(command \Mod.cmd)"),
        vec![
            LParen,
            Command,
            HorzCmdWithMod(vec!["Mod".into()], r"\cmd".into()),
            RParen,
            Eoi
        ]
    );
    assert_eq!(
        toks(r"(command \cmd)"),
        vec![LParen, Command, HorzCmd(r"\cmd".into()), RParen, Eoi]
    );
}

#[test]
fn items() {
    assert_eq!(
        toks("{* a\n** b}"),
        vec![
            BHorzGrp,
            Item(1),
            ch("a"),
            Item(2),
            ch("b"),
            EHorzGrp,
            Eoi
        ]
    );
}

#[test]
fn comments_in_text() {
    // A comment plus following indentation collapses into the preceding space.
    assert_eq!(
        toks("{a % note\n  b}"),
        vec![BHorzGrp, ch("a"), Space, ch("b"), EHorzGrp, Eoi]
    );
}

#[test]
fn module_paths() {
    assert_eq!(
        toks("List.map Math.pi"),
        vec![
            VarWithMod(vec!["List".into()], "map".into()),
            VarWithMod(vec!["Math".into()], "pi".into()),
            Eoi
        ]
    );
    assert_eq!(toks("Some 1"), vec![Constructor("Some".into()), IntConst(1), Eoi]);
}

#[test]
fn unicode_text() {
    assert_eq!(
        toks("{こんにちは 世界}"),
        vec![
            BHorzGrp,
            ch("こんにちは"),
            Space,
            ch("世界"),
            EHorzGrp,
            Eoi
        ]
    );
}

#[test]
fn lex_errors() {
    assert!(lex("{ a ").is_err(), "unterminated inline text");
    assert!(lex("(").is_err(), "unclosed paren at eof");
    assert!(lex(")").is_err(), "too many closing");
    assert!(lex("${ x").is_err(), "unterminated math");
    assert!(lex("`abc").is_err(), "unterminated literal");
}

#[test]
fn use_and_package_are_identifiers_under_v0_0_6() {
    use satysfi_syntax::{lex_with_version, SatysfiVersion};
    // Under 0.0.6 (the base `lex`), `use`/`package` are plain identifiers —
    // there is no `use`/`package` keyword. This is the Axis-B keyword-gating
    // guard (the differential lexer test proves the whole corpus, this pins
    // the two new words specifically).
    let v006: Vec<Token> = lex_with_version("use package foo", SatysfiVersion::V0_0_6)
        .unwrap()
        .into_iter()
        .map(|a| a.slot)
        .collect();
    assert_eq!(v006, vec![var("use"), var("package"), var("foo"), Eoi]);

    // Under 0.1 the same words are keywords.
    let v01: Vec<Token> = lex_with_version("use package Foo", SatysfiVersion::V0_1)
        .unwrap()
        .into_iter()
        .map(|a| a.slot)
        .collect();
    assert_eq!(v01, vec![Use, Package, Constructor("Foo".into()), Eoi]);
}
