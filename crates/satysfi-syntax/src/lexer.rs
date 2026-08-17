//! Stateful mode-stack lexer, a direct port of the v0.0.6 `lexer.mll`.
//!
//! The transitions between the five states are driven entirely by the lexer's
//! own stack (see the transition table in the OCaml header comment); the whole
//! file is lexed eagerly so that parser backtracking can never desynchronize
//! the mode stack.

use crate::span::{Loc, Span};
use crate::token::{Atom, Token};

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

/// Lex a whole source file (a `.saty` document, i.e. program mode at the top).
pub fn lex(src: &str) -> Result<Vec<Atom>, LexError> {
    Lexer::new(src, Mode::Program).run()
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    byte: usize,
    stack: Vec<Mode>,
    out: Vec<Atom>,
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

fn keyword(s: &str) -> Option<Token> {
    use Token::*;
    Some(match s {
        // `not` is deliberately NOT reserved here — see `token.rs`'s
        // `Token::Mod` doc comment: it lexes as an ordinary `Var`/`VarTok`,
        // the same as any other primitive name, so `not expr` parses
        // through the existing `AppExpr` application-chain machinery.
        "mod" => Mod,
        "if" => If,
        "then" => Then,
        "else" => Else,
        "let" => Let,
        "let-rec" => LetRec,
        "and" => LetAnd,
        "in" => In,
        "fun" => Fun,
        "true" => True,
        "false" => False,
        "before" => Before,
        "while" => While,
        "do" => Do,
        "let-mutable" => LetMutable,
        "match" => Match,
        "with" => With,
        "when" => When,
        "as" => As,
        "type" => Type,
        "of" => Of,
        "module" => Module,
        "struct" => Struct,
        "sig" => Sig,
        "val" => Val,
        "end" => End,
        "direct" => Direct,
        "constraint" => Constraint,
        "let-inline" => LetHorz,
        "let-block" => LetVert,
        "let-math" => LetMath,
        "controls" => Controls,
        "cycle" => Cycle,
        "inline-cmd" => HorzCmdType,
        "block-cmd" => VertCmdType,
        "math-cmd" => MathCmdType,
        "command" => Command,
        "open" => Open,
        _ => return None,
    })
}

impl Lexer {
    fn new(src: &str, initial: Mode) -> Self {
        Lexer {
            chars: src.chars().collect(),
            pos: 0,
            line: 1,
            col: 0,
            byte: 0,
            stack: vec![initial],
            out: Vec::new(),
        }
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

    /// Backtick literal body, after the opening backticks have been consumed.
    fn literal(&mut self, start: Loc, quote_len: usize, omit_pre: bool) -> Result<(), LexError> {
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
                        self.emit(
                            start,
                            Token::Literal {
                                body,
                                omit_pre,
                                omit_post,
                            },
                        );
                        return Ok(());
                    }
                }
                Some(c) => {
                    body.push(c);
                    self.bump();
                }
            }
        }
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
                    self.bump();
                    let Some(len) = self.name_len_at(0) else {
                        return self.error(start, "illegal token '\\' in a program area");
                    };
                    let name = format!("\\{}", self.take(len));
                    if self.peek() == Some('@') {
                        self.bump();
                        self.emit(start, Token::HorzMacro(format!("{name}@")));
                    } else {
                        self.emit(start, Token::HorzCmd(name));
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
                    let tok = match self.peek() {
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
                    let tok = keyword(&name).unwrap_or(Token::Var(name));
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
                        return self.error(start, "module path must end with a variable name");
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
            "stage" => match content.as_str() {
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
            },
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
                        self.literal(start, quotes.len(), false)?;
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
                    self.literal(start, quotes.len(), true)?;
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
