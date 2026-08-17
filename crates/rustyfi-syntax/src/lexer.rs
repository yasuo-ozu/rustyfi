//! Stateful mode-stack lexer, a direct port of the v0.0.6 `lexer.mll`.
//!
//! The transitions between the five states are driven entirely by the lexer's
//! own stack (see the transition table in the OCaml header comment); the whole
//! file is lexed eagerly so that parser backtracking can never desynchronize
//! the mode stack.

use crate::span::{Loc, Span};
use crate::token::{Atom, Token};
use crate::version::RustyfiVersion;

#[derive(Debug, thiserror::Error)]
#[error("{span}: {msg}")]
pub struct LexError {
    pub span: Span,
    pub msg: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Program,
    Vertical,
    Horizontal,
    Active,
    Math,
}

/// Lex a whole source file under SATySFi 0.0.6's grammar (a `.saty` document,
/// i.e. program mode at the top). This is the crate's original, frozen entry
/// point: unchanged behavior, now a thin wrapper over [`lex_with_version`].
pub fn lex(src: &str) -> Result<Vec<Atom>, LexError> {
    lex_with_version(src, RustyfiVersion::V0_0_6)
}

/// Lex a whole source file under an explicit target version. `V0_0_6`
/// through this entry point is byte-for-byte [`lex`]'s own behavior — pinned
/// by a differential test that lexes every vendored 0.0.6 package and
/// fixture through both `lex(src)` and `lex_with_version(src, V0_0_6)`,
/// asserting identical token streams
/// (`crates/rustyfi-syntax/tests/lex_with_version_differential.rs`).
pub fn lex_with_version(src: &str, version: RustyfiVersion) -> Result<Vec<Atom>, LexError> {
    Lexer::new(src, Mode::Program, version).run()
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    byte: usize,
    stack: Vec<Mode>,
    out: Vec<Atom>,
    version: RustyfiVersion,
}

fn is_small(c: char) -> bool {
    c.is_ascii_lowercase()
}
fn is_capital(c: char) -> bool {
    c.is_ascii_uppercase()
}
fn is_digit(c: char) -> bool {
    c.is_ascii_digit()
}
fn is_hex(c: char) -> bool {
    c.is_ascii_digit() || ('A'..='F').contains(&c)
}
fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}
fn is_space(c: char) -> bool {
    c == ' ' || c == '\t'
}
fn is_break(c: char) -> bool {
    c == '\n' || c == '\r'
}
fn is_opsymbol(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '^' | '&' | '|' | '!' | ':' | '=' | '<' | '>' | '~' | '\'' | '.'
            | '?'
    )
}
/// The `symbol` class: printable ASCII that `\`-escapes to itself in text.
fn is_symbol_char(c: char) -> bool {
    matches!(c, ' '..='@' | '['..='`' | '{'..='~')
}
/// The `str` class: a plain inline-text character.
fn is_str_char(c: char) -> bool {
    !matches!(
        c,
        ' ' | '\t' | '\n' | '\r' | '@' | '`' | '\\' | '{' | '}' | '<' | '>' | '%' | '|' | '*'
            | '$' | '#' | ';'
    )
}
fn is_mathsymbol_top(c: char) -> bool {
    matches!(c, '+' | '-' | '*' | '/' | ':' | '=' | '<' | '>' | '~' | '.' | ',' | '`')
}
fn is_mathsymbol(c: char) -> bool {
    is_mathsymbol_top(c) || c == '?'
}

impl Lexer {
    fn new(src: &str, initial: Mode, version: RustyfiVersion) -> Self {
        Lexer {
            chars: src.chars().collect(),
            pos: 0,
            line: 1,
            col: 0,
            byte: 0,
            stack: vec![initial],
            out: Vec::new(),
            version,
        }
    }

    /// Look up a scanned identifier-shaped word against the keyword table.
    /// The base table (every 0.0.6 keyword) is version-independent — under
    /// `V0_1` these words still lex the same way (e.g. `let-rec`/`when`/
    /// `while`/`before` simply have no corresponding grammar rule in
    /// `cst_v1.rs`, so using them there is a *parse* error, not a lex
    /// error). Only the nine Slice-1/Sub-slice-2b/Sub-slice-2c/Axis-B/
    /// math-split additions (`rec`/`inline`/`block`/`mutable`/`signature`/
    /// `include`/`use`/`package`/`math`) are version-gated: SATySFi 0.1's
    /// `val rec`/`val inline`/`val block`/`val mutable`/`val math` binds,
    /// `signature …`/`include …` binds/decls, and `use …`/`use package …`
    /// headers need them as keywords, but 0.0.6 source may use any of them
    /// as an ordinary identifier (there is no 0.0.6 grammar that would want
    /// them as keywords), so gating keeps `lex`/`lex_with_version(_,
    /// V0_0_6)` byte-identical to before this change.
    fn keyword(&self, s: &str) -> Option<Token> {
        use Token::*;
        if let Some(tok) = match s {
            // `not` is deliberately NOT reserved — it lexes as an ordinary
            // `Var`/`VarTok` (see `token.rs`'s `Token::Mod` doc comment), so
            // `not expr` parses through the existing `AppExpr` application
            // chain in both generations.
            "mod" => Some(Mod),
            "if" => Some(If),
            "then" => Some(Then),
            "else" => Some(Else),
            "let" => Some(Let),
            "let-rec" => Some(LetRec),
            "and" => Some(LetAnd),
            "in" => Some(In),
            "fun" => Some(Fun),
            "true" => Some(True),
            "false" => Some(False),
            "before" => Some(Before),
            "while" => Some(While),
            "do" => Some(Do),
            "let-mutable" => Some(LetMutable),
            "match" => Some(Match),
            "with" => Some(With),
            "when" => Some(When),
            "as" => Some(As),
            "type" => Some(Type),
            "of" => Some(Of),
            "module" => Some(Module),
            "struct" => Some(Struct),
            "sig" => Some(Sig),
            "val" => Some(Val),
            "end" => Some(End),
            "direct" => Some(Direct),
            "constraint" => Some(Constraint),
            "let-inline" => Some(LetHorz),
            "let-block" => Some(LetVert),
            "let-math" => Some(LetMath),
            "controls" => Some(Controls),
            "cycle" => Some(Cycle),
            "inline-cmd" => Some(HorzCmdType),
            "block-cmd" => Some(VertCmdType),
            "math-cmd" => Some(MathCmdType),
            "command" => Some(Command),
            "open" => Some(Open),
            _ => None,
        } {
            return Some(tok);
        }
        if self.version == RustyfiVersion::V0_1 {
            return match s {
                "rec" => Some(Rec),
                "inline" => Some(Inline),
                "block" => Some(Block),
                "mutable" => Some(Mutable),
                "signature" => Some(Signature),
                "include" => Some(Include),
                "use" => Some(Use),
                "package" => Some(Package),
                "math" => Some(Math),
                _ => None,
            };
        }
        None
    }

    fn loc(&self) -> Loc {
        Loc {
            line: self.line,
            col: self.col,
            byte: self.byte,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, k: usize) -> Option<char> {
        self.chars.get(self.pos + k).copied()
    }

    fn bump(&mut self) -> char {
        let c = self.chars[self.pos];
        self.pos += 1;
        self.byte += c.len_utf8();
        if c == '\n' || (c == '\r' && self.peek() != Some('\n')) {
            self.line += 1;
            self.col = 0;
        } else {
            self.col += 1;
        }
        c
    }

    fn bump_n(&mut self, n: usize) {
        for _ in 0..n {
            self.bump();
        }
    }

    fn scan_while(&mut self, pred: impl Fn(char) -> bool) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if pred(c) {
                s.push(self.bump());
            } else {
                break;
            }
        }
        s
    }

    /// Number of chars from `pos + k` matching `pred` (pure lookahead).
    fn count_at(&self, k: usize, pred: impl Fn(char) -> bool) -> usize {
        let mut n = 0;
        while self.peek_at(k + n).is_some_and(&pred) {
            n += 1;
        }
        n
    }

    fn emit(&mut self, start: Loc, tok: Token) {
        self.out.push(Atom {
            slot: tok,
            span: Span::new(start, self.loc()),
        });
    }

    fn error<T>(&self, start: Loc, msg: impl Into<String>) -> Result<T, LexError> {
        Err(LexError {
            span: Span::new(start, self.loc()),
            msg: msg.into(),
        })
    }

    fn push_mode(&mut self, m: Mode) {
        self.stack.push(m);
    }

    fn pop_mode(&mut self, start: Loc, errmsg: &str) -> Result<(), LexError> {
        if self.stack.len() > 1 {
            self.stack.pop();
            Ok(())
        } else {
            self.error(start, errmsg)
        }
    }

    /// `comment`: skip `%` up to and including the line break (or EOF).
    fn comment(&mut self) {
        while let Some(c) = self.peek() {
            self.bump();
            if is_break(c) {
                break;
            }
        }
    }

    /// `skip_spaces`: skip spaces, breaks, and `%` comments.
    fn skip_spaces(&mut self) {
        while let Some(c) = self.peek() {
            if is_space(c) || is_break(c) {
                self.bump();
            } else if c == '%' {
                self.bump();
                self.comment();
            } else {
                break;
            }
        }
    }

    fn run(mut self) -> Result<Vec<Atom>, LexError> {
        loop {
            match self.stack.last().copied().expect("mode stack never empty") {
                Mode::Program => self.lex_program()?,
                Mode::Vertical => self.lex_vertical()?,
                Mode::Horizontal => self.lex_horizontal()?,
                Mode::Active => self.lex_active()?,
                Mode::Math => self.lex_math()?,
            }
            if matches!(self.out.last(), Some(a) if a.slot == Token::Eoi) {
                return Ok(self.out);
            }
        }
    }

    // ---- shared sub-scanners -------------------------------------------------

    /// `identifier | constructor` at offset `k`: returns its char length.
    fn name_len_at(&self, k: usize) -> Option<usize> {
        let c = self.peek_at(k)?;
        if is_small(c) || is_capital(c) {
            Some(1 + self.count_at(k + 1, is_ident_char))
        } else {
            None
        }
    }

    /// `identifier` (lowercase-initial only) at offset `k`.
    fn ident_len_at(&self, k: usize) -> Option<usize> {
        let c = self.peek_at(k)?;
        if is_small(c) {
            Some(1 + self.count_at(k + 1, is_ident_char))
        } else {
            None
        }
    }

    fn take(&mut self, n: usize) -> String {
        let mut s = String::with_capacity(n);
        for _ in 0..n {
            s.push(self.bump());
        }
        s
    }

    /// `(constructor ".")* (identifier | constructor)` after a sigil: the
    /// dotted command/variable path. Returns `(modules, last, last_is_ctor)`.
    fn scan_dotted(&mut self) -> Option<(Vec<String>, String, bool)> {
        let mut mods = Vec::new();
        loop {
            let len = self.name_len_at(0)?;
            let is_ctor = is_capital(self.peek().unwrap());
            // A constructor followed by `.` and another name continues the path.
            if is_ctor
                && self.peek_at(len) == Some('.')
                && self.name_len_at(len + 1).is_some()
            {
                let seg = self.take(len);
                self.bump(); // `.`
                mods.push(seg);
                continue;
            }
            let last = self.take(len);
            return Some((mods, last, is_ctor));
        }
    }

    /// Read a backtick literal body, after the opening backticks are consumed:
    /// returns the raw body and whether trailing space is omitted (a `#`
    /// immediately after the closing backticks sets `omit_post = false`).
    fn read_literal_body(&mut self, start: Loc, quote_len: usize) -> Result<(String, bool), LexError> {
        let mut body = String::new();
        loop {
            match self.peek() {
                None => return self.error(start, "unexpected end of input while reading literal area"),
                Some('`') => {
                    let run = self.scan_while(|c| c == '`');
                    if run.len() < quote_len {
                        body.push_str(&run);
                    } else if run.len() > quote_len {
                        return self.error(start, "literal area was closed with too many '`'s");
                    } else {
                        let omit_post = if self.peek() == Some('#') {
                            self.bump();
                            false
                        } else {
                            true
                        };
                        return Ok((body, omit_post));
                    }
                }
                Some(c) => {
                    body.push(c);
                    self.bump();
                }
            }
        }
    }

    /// Program- / math-mode backtick string literal → `Token::Literal`.
    fn literal(&mut self, start: Loc, quote_len: usize, omit_pre: bool) -> Result<(), LexError> {
        let (body, omit_post) = self.read_literal_body(start, quote_len)?;
        self.emit(start, Token::Literal { body, omit_pre, omit_post });
        Ok(())
    }

    /// Horizontal-mode (inline-text) backtick literal. Upstream treats
    /// `` `…` `` written inside inline text as literal (un-escaped) character
    /// content, so — rather than a distinct `InlineElem` grammar arm (which
    /// the `Vec<InlineElem>` collection parser cannot host without a manual,
    /// non-peek `Parse` impl that breaks the repetition's termination — see the
    /// `inline-backtick-literal-gap` note) — it lexes to a plain `Token::Char`
    /// run, exactly as if the same characters had been typed directly. The
    /// `#`-controlled flags trim the body's leading (`omit_pre`) / trailing
    /// (`omit_post`) spaces, mirroring the elaborator's `omit_pre_spaces`/
    /// `omit_post_spaces` (the multi-line indent shave `omit_spaces` also does
    /// is unnecessary for the single-line inline spans this path handles).
    fn literal_horz(&mut self, start: Loc, quote_len: usize, omit_pre: bool) -> Result<(), LexError> {
        let (body, omit_post) = self.read_literal_body(start, quote_len)?;
        let mut text: &str = &body;
        if omit_pre {
            text = text.trim_start_matches(' ');
        }
        if omit_post {
            text = text.trim_end_matches(' ');
        }
        self.emit(start, Token::Char(text.to_string()));
        Ok(())
    }

    /// Try to match a length constant `-? (digit+ | digit+ "." digit* | "." digit+) identifier`
    /// as pure lookahead. Returns `(total_len, numeric_len, unit_len)`.
    fn length_lookahead(&self) -> Option<(usize, usize, usize)> {
        let mut k = 0;
        if self.peek_at(k) == Some('-') {
            k += 1;
        }
        let int_digits = self.count_at(k, is_digit);
        k += int_digits;
        let mut frac_digits = 0;
        let mut has_dot = false;
        if self.peek_at(k) == Some('.') {
            has_dot = true;
            frac_digits = self.count_at(k + 1, is_digit);
            k += 1 + frac_digits;
        }
        if int_digits == 0 && frac_digits == 0 {
            return None;
        }
        // `.5` needs digits after the dot; `5.` is fine.
        if int_digits == 0 && !has_dot {
            return None;
        }
        let numeric_len = k;
        let unit_len = self.ident_len_at(k)?;
        Some((numeric_len + unit_len, numeric_len, unit_len))
    }

    // ---- program mode (progexpr) ---------------------------------------------

    fn lex_program(&mut self) -> Result<(), LexError> {
        loop {
            let start = self.loc();
            let Some(c) = self.peek() else {
                if self.stack.len() == 1 {
                    self.emit(start, Token::Eoi);
                    return Ok(());
                }
                return self.error(start, "text input ended while reading a program area");
            };
            match c {
                '%' => {
                    self.bump();
                    self.comment();
                }
                _ if is_space(c) || is_break(c) => {
                    self.bump();
                }
                '@' => {
                    self.bump();
                    return self.lex_header(start);
                }
                '(' => {
                    self.bump();
                    if self.peek() == Some('|') {
                        self.bump();
                        self.push_mode(Mode::Program);
                        self.emit(start, Token::BRecord);
                    } else {
                        self.push_mode(Mode::Program);
                        self.emit(start, Token::LParen);
                    }
                    return Ok(());
                }
                ')' => {
                    self.bump();
                    self.pop_mode(start, "too many closing")?;
                    self.emit(start, Token::RParen);
                    return Ok(());
                }
                '[' => {
                    self.bump();
                    self.push_mode(Mode::Program);
                    self.emit(start, Token::BList);
                    return Ok(());
                }
                ']' => {
                    self.bump();
                    if self.peek() == Some('>') {
                        self.bump();
                        self.emit(start, Token::EPath);
                    } else {
                        self.pop_mode(start, "too many closing")?;
                        self.emit(start, Token::EList);
                    }
                    return Ok(());
                }
                ';' => {
                    self.bump();
                    self.emit(start, Token::ListPunct);
                    return Ok(());
                }
                '{' => {
                    self.bump();
                    self.push_mode(Mode::Horizontal);
                    self.skip_spaces();
                    self.emit(start, Token::BHorzGrp);
                    return Ok(());
                }
                '\'' => {
                    self.bump();
                    if self.peek() == Some('<') {
                        self.bump();
                        self.push_mode(Mode::Vertical);
                        self.emit(start, Token::BVertGrp);
                    } else if let Some(len) = self.ident_len_at(0) {
                        let name = self.take(len);
                        self.emit(start, Token::TypeVar(name));
                    } else {
                        return self.error(start, "illegal token '''");
                    }
                    return Ok(());
                }
                '$' => {
                    self.bump();
                    if self.peek() == Some('{') {
                        self.bump();
                        self.push_mode(Mode::Math);
                        self.emit(start, Token::BMathGrp);
                        return Ok(());
                    }
                    return self.error(start, "illegal token '$' in a program area");
                }
                '`' => {
                    let quotes = self.scan_while(|c| c == '`');
                    self.literal(start, quotes.len(), true)?;
                    return Ok(());
                }
                '#' => {
                    self.bump();
                    if self.peek() == Some('`') {
                        let quotes = self.scan_while(|c| c == '`');
                        self.literal(start, quotes.len(), false)?;
                    } else {
                        self.emit(start, Token::Access);
                    }
                    return Ok(());
                }
                '\\' => {
                    // `command \cmd` / `command \Mod.cmd` (gap 4 of the
                    // V0_1 language-completeness sweep): a first-class
                    // `command`-value in program position must accept a
                    // module-qualified name exactly like inline-text mode's
                    // own `\` handling does (`lex_horizontal`, below) —
                    // switched from a plain `name_len_at` scan to the same
                    // `scan_dotted` dotted-path scanner, emitting
                    // `Token::HorzCmdWithMod` when a `Mod.` prefix was
                    // present. No mode-stack change either way (program
                    // mode never pushes a new mode for a bare `\cmd`
                    // atomic, unlike inline-text mode) — the fix is
                    // confined to this one arm.
                    self.bump();
                    let Some((mods, name, _)) = self.scan_dotted() else {
                        return self.error(start, "illegal token '\\' in a program area");
                    };
                    let cmd_name = format!("\\{name}");
                    if mods.is_empty() {
                        if self.peek() == Some('@') {
                            self.bump();
                            self.emit(start, Token::HorzMacro(format!("{cmd_name}@")));
                        } else {
                            self.emit(start, Token::HorzCmd(cmd_name));
                        }
                    } else {
                        self.emit(start, Token::HorzCmdWithMod(mods, cmd_name));
                    }
                    return Ok(());
                }
                '+' => {
                    self.bump();
                    if let Some(len) = self.name_len_at(0) {
                        let name = format!("+{}", self.take(len));
                        if self.peek() == Some('@') {
                            self.bump();
                            self.emit(start, Token::VertMacro(format!("{name}@")));
                        } else {
                            self.emit(start, Token::VertCmd(name));
                        }
                    } else {
                        let run = format!("+{}", self.scan_while(is_opsymbol));
                        self.emit(start, Token::BinopPlus(run));
                    }
                    return Ok(());
                }
                ',' => {
                    self.bump();
                    self.emit(start, Token::Comma);
                    return Ok(());
                }
                '_' => {
                    self.bump();
                    self.emit(start, Token::Wildcard);
                    return Ok(());
                }
                '-' => {
                    if let Some((total, num_len, unit_len)) = self.length_lookahead() {
                        let num: String = self.take(num_len);
                        let unit: String = self.take(unit_len);
                        debug_assert_eq!(total, num.chars().count() + unit.chars().count());
                        let value: f64 = num.parse().map_err(|_| LexError {
                            span: Span::new(start, self.loc()),
                            msg: format!("malformed length constant '{num}{unit}'"),
                        })?;
                        self.emit(start, Token::LengthConst(value, unit));
                        return Ok(());
                    }
                    let run = self.scan_while(is_opsymbol);
                    let tok = match run.as_str() {
                        "-" => Token::ExactMinus,
                        "--" => Token::PathLine,
                        "->" => Token::Arrow,
                        _ => Token::BinopMinus(run),
                    };
                    self.emit(start, tok);
                    return Ok(());
                }
                '*' => {
                    let run = self.scan_while(is_opsymbol);
                    let tok = if run == "*" {
                        Token::ExactTimes
                    } else {
                        Token::BinopTimes(run)
                    };
                    self.emit(start, tok);
                    return Ok(());
                }
                '/' => {
                    let run = self.scan_while(is_opsymbol);
                    self.emit(start, Token::BinopDivides(run));
                    return Ok(());
                }
                '=' => {
                    let run = self.scan_while(is_opsymbol);
                    let tok = if run == "=" {
                        Token::DefEq
                    } else {
                        Token::BinopEq(run)
                    };
                    self.emit(start, tok);
                    return Ok(());
                }
                '<' => {
                    if self.peek_at(1) == Some('[') {
                        self.bump_n(2);
                        self.emit(start, Token::BPath);
                        return Ok(());
                    }
                    let run = self.scan_while(is_opsymbol);
                    let tok = if run == "<-" {
                        Token::OverwriteEq
                    } else {
                        Token::BinopLt(run)
                    };
                    self.emit(start, tok);
                    return Ok(());
                }
                '>' => {
                    let run = self.scan_while(is_opsymbol);
                    self.emit(start, Token::BinopGt(run));
                    return Ok(());
                }
                '&' => {
                    let run = self.scan_while(is_opsymbol);
                    let tok = if run == "&" {
                        Token::ExactAmp
                    } else {
                        Token::BinopAmp(run)
                    };
                    self.emit(start, tok);
                    return Ok(());
                }
                '|' => {
                    if self.peek_at(1) == Some(')') {
                        self.bump_n(2);
                        self.pop_mode(start, "too many closing")?;
                        self.emit(start, Token::ERecord);
                        return Ok(());
                    }
                    let run = self.scan_while(is_opsymbol);
                    let tok = if run == "|" {
                        Token::Bar
                    } else {
                        Token::BinopBar(run)
                    };
                    self.emit(start, tok);
                    return Ok(());
                }
                '^' => {
                    let run = self.scan_while(is_opsymbol);
                    self.emit(start, Token::BinopHat(run));
                    return Ok(());
                }
                '!' => {
                    let run = self.scan_while(is_opsymbol);
                    self.emit(start, Token::UnopExclam(run));
                    return Ok(());
                }
                '~' => {
                    self.bump();
                    self.emit(start, Token::ExactTilde);
                    return Ok(());
                }
                ':' => {
                    self.bump();
                    if self.peek() == Some(':') {
                        self.bump();
                        self.emit(start, Token::Cons);
                    } else if self.version == RustyfiVersion::V0_1 && self.peek() == Some('>') {
                        // 0.1's COERCE `:>` (lexer_v1.mll:280). Tried AFTER
                        // `::` so `::>` still lexes `Cons` + `BinopGt`, the
                        // same longest/first-match ocamllex resolves
                        // (lexer_v1.mll:280 vs :288 — `::` wins on `::>`).
                        // V0_1-gated: under V0_0_6, `:>` stays `Colon` +
                        // `BinopGt(">")` — the differential test pins it.
                        self.bump();
                        self.emit(start, Token::Coerce);
                    } else {
                        self.emit(start, Token::Colon);
                    }
                    return Ok(());
                }
                '.' => {
                    if self.peek_at(1) == Some('.') {
                        self.bump_n(2);
                        self.emit(start, Token::PathCurve);
                        return Ok(());
                    }
                    if self.peek_at(1).is_some_and(is_digit) {
                        return self.lex_number(start);
                    }
                    return self.error(start, "illegal token '.' in a program area");
                }
                '?' => {
                    self.bump();
                    // SATySFi 0.1 removed the fused `?:`/`?*`/`?->` optional
                    // sigils: a bare `?` is the only `?`-headed token (it
                    // heads a `?(l = e, …)` labeled-optional bundle, lexed as
                    // `OptionalType` + a paren group). So under V0_1 emit only
                    // `OptionalType`; `?:`/`?*`/`?->` then lex as `?` + `:`/`*`
                    // /`->`, a downstream parse error, matching upstream.
                    // Under V0_0_6 this stays byte-identical (pinned by
                    // `lex_with_version_differential.rs`).
                    //
                    // optional-arg-rows increment 2: under V0_1, `?` directly
                    // followed by `'` + a lowercase name (no space — one
                    // lexeme, matching upstream `ROWVAR`, `lexer_v1.mll:310
                    // -311`) is a row variable (`Token::RowVar`), e.g. `?'r`
                    // in a row-var record tail `(| … | ?'r |)`. A SPACE
                    // between `?` and `'r` (`? 'r`) must NOT fuse — and
                    // naturally doesn't, since this whole match arm only
                    // runs once per token (space-skipping happens between
                    // token scans, not inside it), so `? 'r` lexes as two
                    // separate tokens (`OptionalType` then `TypeVar`), same
                    // as it always has.
                    let tok = if self.version == RustyfiVersion::V0_1 {
                        if self.peek() == Some('\'') {
                            if let Some(len) = self.ident_len_at(1) {
                                self.bump(); // consume the `'`
                                let name = self.take(len);
                                Token::RowVar(name)
                            } else {
                                Token::OptionalType
                            }
                        } else {
                            Token::OptionalType
                        }
                    } else {
                        match self.peek() {
                            Some(':') => {
                                self.bump();
                                Token::Optional
                            }
                            Some('*') => {
                                self.bump();
                                Token::Omission
                            }
                            Some('-') if self.peek_at(1) == Some('>') => {
                                self.bump_n(2);
                                Token::OptionalArrow
                            }
                            _ => Token::OptionalType,
                        }
                    };
                    self.emit(start, tok);
                    return Ok(());
                }
                _ if is_digit(c) => {
                    return self.lex_number(start);
                }
                _ if is_small(c) => {
                    let len = self.name_len_at(0).unwrap();
                    let name = self.take(len);
                    let tok = self.keyword(&name).unwrap_or(Token::Var(name));
                    self.emit(start, tok);
                    return Ok(());
                }
                _ if is_capital(c) => {
                    let (mods, last, last_is_ctor) = self.scan_dotted().unwrap();
                    if mods.is_empty() && last_is_ctor {
                        // `Mod.(` opens the module scope inline.
                        if self.peek() == Some('.') && self.peek_at(1) == Some('(') {
                            self.bump_n(2);
                            self.push_mode(Mode::Program);
                            self.emit(start, Token::OpenModule(last));
                        } else {
                            self.emit(start, Token::Constructor(last));
                        }
                    } else if last_is_ctor {
                        if self.version == RustyfiVersion::V0_1 {
                            // 0.1's LONG_UPPER (lexer_v1.mll:357-363):
                            // `A.B.C` is a module/signature path token.
                            self.emit(start, Token::LongUpper(mods, last));
                        } else {
                            // 0.0.6: unchanged — the exact error string is
                            // pinned by the differential test (Err/Err arm).
                            return self.error(start, "module path must end with a variable name");
                        }
                    } else {
                        self.emit(start, Token::VarWithMod(mods, last));
                    }
                    return Ok(());
                }
                _ => {
                    self.bump();
                    return self.error(start, format!("illegal token '{c}' in a program area"));
                }
            }
        }
    }

    /// `@require:`/`@import:`/`@stage:` headers (the `@` is already consumed).
    fn lex_header(&mut self, start: Loc) -> Result<(), LexError> {
        if self.peek() == Some('`') {
            return self.error(start, "positioned string literals '@`' are not supported yet");
        }
        let Some(len) = self.ident_len_at(0) else {
            return self.error(start, "illegal token '@' in a program area");
        };
        let headertype = self.take(len);
        if self.peek() != Some(':') {
            return self.error(start, format!("undefined header type '{headertype}'"));
        }
        self.bump();
        while self.peek() == Some(' ') {
            self.bump();
        }
        let mut content = String::new();
        while let Some(c) = self.peek() {
            if is_break(c) {
                self.bump();
                break;
            }
            content.push(self.bump());
        }
        let tok = match headertype.as_str() {
            "require" => Token::HeaderRequire(content),
            "import" => Token::HeaderImport(content),
            "stage" => {
                // SATySFi 0.1's lexer dropped the `"stage" -> HEADER_STAGE*`
                // arm entirely: staging is per-binding (`val ~x`, `val
                // persistent ~x`) rather than a whole-file `@stage:` header.
                // Seeing `@stage:` under `V0_1` is therefore a real, direct
                // lex-level signal that the source is not valid 0.1 (mirrors
                // `version::sniff_version`'s own use of this fact).
                if self.version == RustyfiVersion::V0_1 {
                    return self.error(
                        start,
                        "the '@stage:' header does not exist in SATySFi 0.1 \
                         (staging is per-binding there: 'val ~x' / 'val persistent ~x')",
                    );
                }
                match content.as_str() {
                    "persistent" => Token::HeaderPersistent0,
                    "0" => Token::HeaderStage0,
                    "1" => Token::HeaderStage1,
                    _ => {
                        return self.error(
                            start,
                            format!(
                                "undefined stage type '{content}'; should be 'persistent', '0', or '1'."
                            ),
                        )
                    }
                }
            }
            _ => return self.error(start, format!("undefined header type '{headertype}'")),
        };
        self.emit(start, tok);
        Ok(())
    }

    /// Int / float / length constants starting with a digit or `.`.
    fn lex_number(&mut self, start: Loc) -> Result<(), LexError> {
        if self.peek() == Some('0')
            && matches!(self.peek_at(1), Some('x') | Some('X'))
            && self.peek_at(2).is_some_and(is_hex)
        {
            self.bump_n(2);
            let hex = self.scan_while(is_hex);
            let value = i64::from_str_radix(&hex, 16).map_err(|_| LexError {
                span: Span::new(start, self.loc()),
                msg: format!("malformed hexadecimal constant '0x{hex}'"),
            })?;
            self.emit(start, Token::IntConst(value));
            return Ok(());
        }
        if let Some((_, num_len, unit_len)) = self.length_lookahead() {
            let num = self.take(num_len);
            let unit = self.take(unit_len);
            let value: f64 = num.parse().unwrap_or_default();
            self.emit(start, Token::LengthConst(value, unit));
            return Ok(());
        }
        let int_part = self.scan_while(is_digit);
        if self.peek() == Some('.')
            && (self.peek_at(1).is_some_and(is_digit) || !int_part.is_empty())
        {
            self.bump();
            let frac = self.scan_while(is_digit);
            let text = format!("{int_part}.{frac}");
            let value: f64 = text.parse().unwrap_or_default();
            self.emit(start, Token::FloatConst(value));
        } else {
            let value: i64 = int_part.parse().map_err(|_| LexError {
                span: Span::new(start, self.loc()),
                msg: format!("malformed integer constant '{int_part}'"),
            })?;
            self.emit(start, Token::IntConst(value));
        }
        Ok(())
    }

    // ---- vertical mode (vertexpr) ---------------------------------------------

    fn lex_vertical(&mut self) -> Result<(), LexError> {
        loop {
            let start = self.loc();
            let Some(c) = self.peek() else {
                if self.stack.len() == 1 {
                    self.emit(start, Token::Eoi);
                    return Ok(());
                }
                return self.error(start, "unexpected end of input while reading a vertical area");
            };
            match c {
                '%' => {
                    self.bump();
                    self.comment();
                }
                _ if is_space(c) || is_break(c) => {
                    self.bump();
                }
                '#' => {
                    self.bump();
                    let Some((mods, name, _)) = self.scan_dotted() else {
                        return self.error(start, "unexpected character '#' in a vertical area");
                    };
                    self.push_mode(Mode::Active);
                    self.emit(start, Token::VarInVert(mods, name));
                    return Ok(());
                }
                '+' => {
                    self.bump();
                    let Some((mods, name, _)) = self.scan_dotted() else {
                        return self.error(start, "unexpected character '+' in a vertical area");
                    };
                    self.push_mode(Mode::Active);
                    if mods.is_empty() {
                        if self.peek() == Some('@') {
                            self.bump();
                            self.emit(start, Token::VertMacro(format!("+{name}@")));
                        } else {
                            self.emit(start, Token::VertCmd(format!("+{name}")));
                        }
                    } else {
                        self.emit(start, Token::VertCmdWithMod(mods, format!("+{name}")));
                    }
                    return Ok(());
                }
                '<' => {
                    self.bump();
                    self.push_mode(Mode::Vertical);
                    self.emit(start, Token::BVertGrp);
                    return Ok(());
                }
                '>' => {
                    self.bump();
                    self.pop_mode(start, "too many closing")?;
                    self.emit(start, Token::EVertGrp);
                    return Ok(());
                }
                '{' => {
                    self.bump();
                    self.push_mode(Mode::Horizontal);
                    self.skip_spaces();
                    self.emit(start, Token::BHorzGrp);
                    return Ok(());
                }
                _ => {
                    self.bump();
                    return self.error(
                        start,
                        format!("unexpected character '{c}' in a vertical area"),
                    );
                }
            }
        }
    }

    // ---- horizontal mode (horzexpr) --------------------------------------------

    fn lex_horizontal(&mut self) -> Result<(), LexError> {
        loop {
            let start = self.loc();
            let Some(c) = self.peek() else {
                if self.stack.len() == 1 {
                    self.emit(start, Token::Eoi);
                    return Ok(());
                }
                return self.error(
                    start,
                    "unexpected end of input while reading an inline text area",
                );
            };

            if c == '%' {
                self.bump();
                self.comment();
                self.skip_spaces();
                continue;
            }

            // The `(break | space)* <terminator>` family: `{`, `}`, `<`, `|`, item.
            let ws = self.count_at(0, |c| is_space(c) || is_break(c));
            match self.peek_at(ws) {
                Some('{') => {
                    self.bump_n(ws + 1);
                    self.push_mode(Mode::Horizontal);
                    self.skip_spaces();
                    self.emit(start, Token::BHorzGrp);
                    return Ok(());
                }
                Some('}') => {
                    self.bump_n(ws + 1);
                    self.pop_mode(start, "too many closing")?;
                    self.emit(start, Token::EHorzGrp);
                    return Ok(());
                }
                Some('<') => {
                    self.bump_n(ws + 1);
                    self.push_mode(Mode::Vertical);
                    self.emit(start, Token::BVertGrp);
                    return Ok(());
                }
                Some('|') => {
                    self.bump_n(ws + 1);
                    self.skip_spaces();
                    self.emit(start, Token::Sep);
                    return Ok(());
                }
                Some('*') => {
                    self.bump_n(ws);
                    let stars = self.scan_while(|c| c == '*');
                    self.skip_spaces();
                    self.emit(start, Token::Item(stars.len()));
                    return Ok(());
                }
                _ => {}
            }
            if ws > 0 {
                let first = self.peek().unwrap();
                self.bump_n(ws);
                self.skip_spaces();
                self.emit(start, if is_break(first) { Token::Break } else { Token::Space });
                return Ok(());
            }

            match c {
                '#' => {
                    self.bump();
                    if self.peek() == Some('`') {
                        let quotes = self.scan_while(|c| c == '`');
                        self.literal_horz(start, quotes.len(), false)?;
                        return Ok(());
                    }
                    let Some((mods, name, _)) = self.scan_dotted() else {
                        return self.error(start, "illegal token '#' in an inline text area");
                    };
                    self.push_mode(Mode::Active);
                    self.emit(start, Token::VarInHorz(mods, name));
                    return Ok(());
                }
                '\\' => {
                    self.bump();
                    if let Some((mods, name, _)) = self.scan_dotted() {
                        if mods.is_empty() {
                            if self.peek() == Some('@') {
                                self.bump();
                                self.push_mode(Mode::Active);
                                self.emit(start, Token::HorzMacro(format!("\\{name}@")));
                            } else {
                                self.push_mode(Mode::Active);
                                self.emit(start, Token::HorzCmd(format!("\\{name}")));
                            }
                        } else {
                            self.push_mode(Mode::Active);
                            self.emit(start, Token::HorzCmdWithMod(mods, format!("\\{name}")));
                        }
                        return Ok(());
                    }
                    if self.peek().is_some_and(is_symbol_char) {
                        let sym = self.bump();
                        self.emit(start, Token::Char(sym.to_string()));
                        return Ok(());
                    }
                    return self.error(start, "illegal token '\\' in an inline text area");
                }
                '$' => {
                    self.bump();
                    if self.peek() == Some('{') {
                        self.bump();
                        self.push_mode(Mode::Math);
                        self.emit(start, Token::BMathGrp);
                        return Ok(());
                    }
                    return self.error(start, "illegal token '$' in an inline text area");
                }
                '`' => {
                    let quotes = self.scan_while(|c| c == '`');
                    self.literal_horz(start, quotes.len(), true)?;
                    return Ok(());
                }
                _ if is_str_char(c) => {
                    let text = self.scan_while(is_str_char);
                    self.emit(start, Token::Char(text));
                    return Ok(());
                }
                _ => {
                    self.bump();
                    return self.error(
                        start,
                        format!("illegal token '{c}' in an inline text area"),
                    );
                }
            }
        }
    }

    // ---- active mode ------------------------------------------------------------

    fn lex_active(&mut self) -> Result<(), LexError> {
        loop {
            let start = self.loc();
            let Some(c) = self.peek() else {
                return self.error(start, "unexpected end of input while reading an active area");
            };
            match c {
                '%' => {
                    self.bump();
                    self.comment();
                }
                _ if is_space(c) || is_break(c) => {
                    self.bump();
                }
                '?' => {
                    self.bump();
                    // SATySFi 0.1 (optional-arg-rows increment 3b-β): a command
                    // APPLIED in an active area (`\cmd ?(l = e){…}` /
                    // `+cmd ?(l = e)<…>`) carries a `?(l = e, …)` labeled-
                    // optional bundle — the `?` is `OptionalType` and the
                    // `(…)` group lexes via the `(` arm on the next scan,
                    // exactly like program-mode application (`lex_program`'s
                    // `?` arm). The fused `?:`/`?*` sigils no longer exist
                    // under V0_1. Under V0_0_6 this stays byte-identical
                    // (`?:`/`?*` handled, a bare `?` is still an error) —
                    // pinned by `lex_with_version_differential.rs`.
                    if self.version == RustyfiVersion::V0_1 {
                        self.emit(start, Token::OptionalType);
                        return Ok(());
                    }
                    match self.peek() {
                        Some(':') => {
                            self.bump();
                            self.emit(start, Token::Optional);
                        }
                        Some('*') => {
                            self.bump();
                            self.emit(start, Token::Omission);
                        }
                        _ => return self.error(start, "unexpected token '?' in an active area"),
                    }
                    return Ok(());
                }
                '~' => {
                    self.bump();
                    self.emit(start, Token::ExactTilde);
                    return Ok(());
                }
                '(' => {
                    self.bump();
                    if self.peek() == Some('|') {
                        self.bump();
                        self.push_mode(Mode::Program);
                        self.emit(start, Token::BRecord);
                    } else {
                        self.push_mode(Mode::Program);
                        self.emit(start, Token::LParen);
                    }
                    return Ok(());
                }
                '[' => {
                    self.bump();
                    self.push_mode(Mode::Program);
                    self.emit(start, Token::BList);
                    return Ok(());
                }
                '{' => {
                    self.bump();
                    self.pop_mode(start, "BUG; this cannot happen")?;
                    self.push_mode(Mode::Horizontal);
                    self.skip_spaces();
                    self.emit(start, Token::BHorzGrp);
                    return Ok(());
                }
                '<' => {
                    self.bump();
                    self.pop_mode(start, "BUG; this cannot happen")?;
                    self.push_mode(Mode::Vertical);
                    self.emit(start, Token::BVertGrp);
                    return Ok(());
                }
                ';' => {
                    self.bump();
                    self.pop_mode(start, "BUG; this cannot happen")?;
                    self.emit(start, Token::EndActive);
                    return Ok(());
                }
                _ => {
                    self.bump();
                    return self.error(start, format!("unexpected token '{c}' in an active area"));
                }
            }
        }
    }

    // ---- math mode (mathexpr) -----------------------------------------------------

    fn lex_math(&mut self) -> Result<(), LexError> {
        loop {
            let start = self.loc();
            let Some(c) = self.peek() else {
                return self.error(start, "unexpected end of file in a math area");
            };
            match c {
                '%' => {
                    self.bump();
                    self.comment();
                }
                _ if is_space(c) || is_break(c) => {
                    self.bump();
                }
                '?' => {
                    self.bump();
                    match self.peek() {
                        Some(':') => {
                            self.bump();
                            self.emit(start, Token::Optional);
                        }
                        Some('*') => {
                            self.bump();
                            self.emit(start, Token::Omission);
                        }
                        _ => return self.error(start, "illegal token '?' in a math area"),
                    }
                    return Ok(());
                }
                '!' => {
                    self.bump();
                    match self.peek() {
                        Some('{') => {
                            self.bump();
                            self.push_mode(Mode::Horizontal);
                            self.skip_spaces();
                            self.emit(start, Token::BHorzGrp);
                        }
                        Some('<') => {
                            self.bump();
                            self.push_mode(Mode::Vertical);
                            self.emit(start, Token::BVertGrp);
                        }
                        Some('(') => {
                            self.bump();
                            if self.peek() == Some('|') {
                                self.bump();
                                self.push_mode(Mode::Program);
                                self.emit(start, Token::BRecord);
                            } else {
                                self.push_mode(Mode::Program);
                                self.emit(start, Token::LParen);
                            }
                        }
                        Some('[') => {
                            self.bump();
                            self.push_mode(Mode::Program);
                            self.emit(start, Token::BList);
                        }
                        _ => return self.error(start, "illegal token '!' in a math area"),
                    }
                    return Ok(());
                }
                '{' => {
                    self.bump();
                    self.push_mode(Mode::Math);
                    self.emit(start, Token::BMathGrp);
                    return Ok(());
                }
                '}' => {
                    self.bump();
                    self.pop_mode(start, "too many closing")?;
                    self.emit(start, Token::EMathGrp);
                    return Ok(());
                }
                '|' => {
                    self.bump();
                    self.emit(start, Token::Sep);
                    return Ok(());
                }
                '^' => {
                    self.bump();
                    self.emit(start, Token::Superscript);
                    return Ok(());
                }
                '_' => {
                    self.bump();
                    self.emit(start, Token::Subscript);
                    return Ok(());
                }
                '\'' => {
                    let primes = self.scan_while(|c| c == '\'');
                    self.emit(start, Token::Primes(primes.len()));
                    return Ok(());
                }
                '#' => {
                    self.bump();
                    let Some((mods, name, _)) = self.scan_dotted() else {
                        return self.error(start, "illegal token '#' in a math area");
                    };
                    self.emit(start, Token::VarInMath(mods, name));
                    return Ok(());
                }
                '\\' => {
                    self.bump();
                    if let Some((mods, name, _)) = self.scan_dotted() {
                        if mods.is_empty() {
                            self.emit(start, Token::MathCmd(format!("\\{name}")));
                        } else {
                            self.emit(start, Token::MathCmdWithMod(mods, format!("\\{name}")));
                        }
                        return Ok(());
                    }
                    if self.peek().is_some_and(is_symbol_char) {
                        let sym = self.bump();
                        self.emit(start, Token::MathChar(sym.to_string()));
                        return Ok(());
                    }
                    return self.error(start, "illegal token '\\' in a math area");
                }
                _ if is_mathsymbol_top(c) => {
                    self.bump();
                    let mut run = c.to_string();
                    run.push_str(&self.scan_while(is_mathsymbol));
                    self.emit(start, Token::MathChar(run));
                    return Ok(());
                }
                _ if c.is_ascii_alphanumeric() => {
                    self.bump();
                    self.emit(start, Token::MathChar(c.to_string()));
                    return Ok(());
                }
                _ => {
                    self.bump();
                    return self.error(start, format!("illegal token '{c}' in a math area"));
                }
            }
        }
    }
}
