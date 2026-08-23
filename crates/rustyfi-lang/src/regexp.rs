//! A backtracking regular-expression engine for OCaml's `Str` dialect.
//!
//! Upstream SATySFi's `regexp` values are `Str.regexp`s and its regexp
//! primitives (`string-scan`, `string-match`, `split-on-regexp`) are thin
//! wrappers over `Str.string_match` / `Str.search_forward`. This port models a
//! `regexp` as its own pattern *string* (`regexp-of-string` is the identity),
//! so the dialect has to be interpreted here.
//!
//! # Why `Str` and not a stock regex crate
//!
//! `Str`'s surface syntax is deliberately unlike PCRE, and the difference is
//! not cosmetic — it inverts which characters are special:
//!
//! * grouping is `\(` … `\)`, and a bare `(` / `)` is a LITERAL parenthesis;
//! * alternation is `\|`, and a bare `|` is a LITERAL bar;
//! * `{` and `}` are always literal — `Str` has no counted repetition at all;
//! * inside a `[…]` set, backslash is NOT an escape: `[\t]` is the two-element
//!   set `{'\\', 't'}`.
//!
//! Feeding a `Str` pattern to a PCRE-syntax engine therefore does not merely
//! fail, it silently means something else: `satysfi-code-printer`'s SATySFi
//! identifier rule `\(\\\|\+\)?[a-zA-Z][a-zA-Z0-9-]*\|[0-9]+` would parse as a
//! literal-parenthesis soup. Translating `Str` to PCRE is the same work as
//! parsing `Str`, minus the control over match semantics, so the pattern is
//! parsed directly here.
//!
//! # Semantics
//!
//! `Str` is a backtracking matcher (`strstubs.c`'s `re_match`), so quantifiers
//! are Perl-style — greedy by default, leftmost-biased alternation, first
//! match in backtracking order rather than leftmost-longest. This engine
//! reproduces that, which is what makes `Str.matched_string` predictable:
//! `a\|ab` matched against `"ab"` yields `"a"` in both.
//!
//! Supported: `.` (any but newline), `*`, `+`, `?` and their lazy `*?`, `+?`,
//! `??` forms, `[…]` sets with ranges and a leading `^` complement, `^` / `$`
//! (line anchors), `\|`, `\(`…`\)`, `\b`, `\1`–`\9` backreferences, and `\`
//! quoting of any special character.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// One element of a `[…]` character set.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ClassItem {
    Char(char),
    Range(char, char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    /// Matches nothing and consumes nothing — the empty branch of `a\|`.
    Empty,
    Char(char),
    /// `.` — any character except a newline, per `Str`.
    Any,
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    /// `^` — start of the text or just after a `\n`.
    Bol,
    /// `$` — end of the text or just before a `\n`.
    Eol,
    /// `\b` — a word-constituent/non-constituent boundary.
    WordBoundary,
    /// `\(` … `\)`; the index is 1-based, matching `Str.matched_group`.
    Group(usize, Box<Node>),
    /// `\1` … `\9`.
    Backref(usize),
    Concat(Vec<Node>),
    /// `\|`, in source order — the matcher tries branches left to right.
    Alt(Vec<Node>),
    Repeat {
        node: Box<Node>,
        min: u32,
        /// `None` for an unbounded `*` / `+`.
        max: Option<u32>,
        greedy: bool,
    },
}

/// A parsed pattern, plus the group count needed to size the capture vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Regexp {
    root: Node,
    groups: usize,
    /// The parser hit [`Budget::MAX_NESTING`] and read a `\(` as a literal
    /// `(`. The tree below is therefore not this pattern, so matching it
    /// reports [`GaveUp`] rather than answering from the degraded reading —
    /// the parser's other degradations turn a malformed pattern into a
    /// literal one, but this one would turn a WELL-FORMED pattern into a
    /// different well-formed pattern, and answer confidently.
    truncated: bool,
}

struct Parser<'p> {
    src: &'p [char],
    pos: usize,
    groups: usize,
    /// `\(` nesting still available. The grammar below is recursive descent —
    /// `parse_atom` → `parse_alt` → `parse_concat` → `parse_repeat` →
    /// `parse_atom` — so a pattern is free to drive the PARSER off the stack
    /// even though nothing has matched yet. See [`Budget::MAX_NESTING`].
    nesting: usize,
    /// Set once the cap above has fired; see [`Regexp::truncated`].
    truncated: bool,
}

impl<'p> Parser<'p> {
    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<char> {
        self.src.get(self.pos + off).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    /// alternation := concat (`\|` concat)*
    fn parse_alt(&mut self) -> Node {
        let mut branches = vec![self.parse_concat()];
        while self.peek() == Some('\\') && self.peek_at(1) == Some('|') {
            self.pos += 2;
            branches.push(self.parse_concat());
        }
        if branches.len() == 1 {
            branches.pop().unwrap()
        } else {
            Node::Alt(branches)
        }
    }

    /// concat := repeat*, stopping at `\|` or `\)` (or end of input).
    fn parse_concat(&mut self) -> Node {
        let mut items: Vec<Node> = Vec::new();
        loop {
            match self.peek() {
                None => break,
                Some('\\') => match self.peek_at(1) {
                    // Both terminate a concatenation; leave them for the caller.
                    Some('|') | Some(')') => break,
                    _ => {}
                },
                _ => {}
            }
            match self.parse_repeat() {
                Some(node) => items.push(node),
                None => break,
            }
        }
        match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Concat(items),
        }
    }

    /// repeat := atom postfix*, where postfix is `*`, `+` or `?` with an
    /// optional trailing `?` making it lazy.
    fn parse_repeat(&mut self) -> Option<Node> {
        let mut node = self.parse_atom()?;
        loop {
            let (min, max) = match self.peek() {
                Some('*') => (0, None),
                Some('+') => (1, None),
                Some('?') => (0, Some(1)),
                _ => break,
            };
            self.pos += 1;
            // `Str` supports the lazy forms `*?`, `+?`, `??`. A `?` here is
            // therefore a modifier, not another quantifier.
            let greedy = if self.peek() == Some('?') {
                self.pos += 1;
                false
            } else {
                true
            };
            node = Node::Repeat {
                node: Box::new(node),
                min,
                max,
                greedy,
            };
        }
        Some(node)
    }

    fn parse_atom(&mut self) -> Option<Node> {
        let c = self.peek()?;
        match c {
            '.' => {
                self.pos += 1;
                Some(Node::Any)
            }
            '^' => {
                self.pos += 1;
                Some(Node::Bol)
            }
            '$' => {
                self.pos += 1;
                Some(Node::Eol)
            }
            '[' => {
                self.pos += 1;
                Some(self.parse_class())
            }
            // A quantifier with nothing to quantify is a literal in `Str`.
            '*' | '+' | '?' => {
                self.pos += 1;
                Some(Node::Char(c))
            }
            '\\' => {
                self.pos += 1;
                let e = match self.bump() {
                    Some(e) => e,
                    // A trailing lone backslash is itself.
                    None => return Some(Node::Char('\\')),
                };
                match e {
                    // Past the nesting cap a `\(` reads as the literal `(` it
                    // would be without the backslash — recursing further is
                    // how the PARSER goes off the stack, before anything has
                    // matched. `truncated` then makes every match report
                    // `GaveUp`, so the degraded tree is never answered from.
                    '(' if self.nesting == 0 => {
                        self.truncated = true;
                        Some(Node::Char('('))
                    }
                    '(' => {
                        self.groups += 1;
                        let idx = self.groups;
                        self.nesting -= 1;
                        let inner = self.parse_alt();
                        self.nesting += 1;
                        // Tolerate an unclosed `\(` rather than failing the
                        // whole pattern: `Str` would raise, but a raised
                        // exception here would abort a whole document over
                        // one malformed rule.
                        if self.peek() == Some('\\') && self.peek_at(1) == Some(')') {
                            self.pos += 2;
                        }
                        Some(Node::Group(idx, Box::new(inner)))
                    }
                    'b' => Some(Node::WordBoundary),
                    d if d.is_ascii_digit() && d != '0' => {
                        Some(Node::Backref(d as usize - '0' as usize))
                    }
                    other => Some(Node::Char(other)),
                }
            }
            other => {
                self.pos += 1;
                Some(Node::Char(other))
            }
        }
    }

    /// `[…]`. Per `Str`, a leading `^` complements, a `]` or `-` in first
    /// position is literal, and backslash is NOT an escape inside the set.
    fn parse_class(&mut self) -> Node {
        let negated = if self.peek() == Some('^') {
            self.pos += 1;
            true
        } else {
            false
        };
        let mut items: Vec<ClassItem> = Vec::new();
        let mut first = true;
        loop {
            let c = match self.peek() {
                None => break,
                Some(c) => c,
            };
            if c == ']' && !first {
                self.pos += 1;
                break;
            }
            self.pos += 1;
            first = false;
            // `a-z`, but a `-` in last position is a literal `-`.
            if self.peek() == Some('-')
                && self.peek_at(1).is_some()
                && self.peek_at(1) != Some(']')
            {
                self.pos += 1;
                let hi = self.bump().unwrap();
                items.push(ClassItem::Range(c, hi));
            } else {
                items.push(ClassItem::Char(c));
            }
        }
        Node::Class { negated, items }
    }
}

impl Regexp {
    /// Parse a `Str`-dialect pattern. Never fails: malformed input degrades to
    /// the most literal reading, because a raised error here would take down a
    /// whole document over a single bad highlighting rule.
    pub fn parse(pattern: &str) -> Regexp {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser {
            src: &chars,
            pos: 0,
            groups: 0,
            nesting: Budget::MAX_NESTING,
            truncated: false,
        };
        let root = p.parse_alt();
        Regexp {
            root,
            groups: p.groups,
            truncated: p.truncated,
        }
    }

    /// Anchored match at `start`, returning the end offset (in `char`s) of the
    /// match — `Str.string_match` plus `Str.match_end`.
    ///
    /// A pattern that exhausts its step budget reports `Err(GaveUp)` rather
    /// than "no match": see [`Budget`]. Silently answering `None` would turn a
    /// hang into wrong output, which is worse.
    pub fn match_at(&self, input: &[char], start: usize) -> Result<Option<usize>, GaveUp> {
        if self.truncated {
            return Err(GaveUp);
        }
        let m = Matcher::with_budget(input);
        let mut caps = self.fresh_caps();
        let out = self.attempt(&m, start, &mut caps);
        if m.gave_up.get() {
            Err(GaveUp)
        } else {
            Ok(out)
        }
    }

    /// Leftmost match at or after `start` — `Str.search_forward`.
    pub fn search_from(
        &self,
        input: &[char],
        start: usize,
    ) -> Result<Option<(usize, usize)>, GaveUp> {
        // ONE budget for the whole search, not one per start position. A fresh
        // `Matcher` per position — which is what calling `match_at` in the
        // loop would build — makes the total `input.len() × budget`, i.e. the
        // budget stops bounding anything as soon as the input is long, which
        // is precisely when it needs to.
        if self.truncated {
            return Err(GaveUp);
        }
        let m = Matcher::with_budget(input);
        let mut caps = self.fresh_caps();
        for i in start..=input.len() {
            caps.iter_mut().for_each(|c| *c = None);
            let hit = self.attempt(&m, i, &mut caps);
            if m.gave_up.get() {
                return Err(GaveUp);
            }
            if let Some(end) = hit {
                return Ok(Some((i, end)));
            }
        }
        Ok(None)
    }

    fn fresh_caps(&self) -> Vec<Option<(usize, usize)>> {
        vec![None; self.groups + 1]
    }

    /// One anchored attempt against an already-seeded budget.
    fn attempt(
        &self,
        m: &Matcher<'_>,
        start: usize,
        caps: &mut Vec<Option<(usize, usize)>>,
    ) -> Option<usize> {
        m.node(&self.root, start, caps, &|end, _| Some(end))
    }
}

/// The matcher ran out of steps or nesting depth before deciding.
///
/// This is the regexp twin of `rustyfi-syntax`'s `ParseFailureKind::GaveUp`,
/// and it exists for the same reason: the matcher is an ordered-choice
/// backtracker, so a pattern with a quantified group inside a quantifier
/// (`\(a*\)*b`) costs a factor per nesting level. Unbounded, thirty
/// characters of input take two minutes and thirty-two never finish — in a
/// browser tab that is a freeze with no cancel, because the playground
/// compiles on the main thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GaveUp;

struct Matcher<'i> {
    input: &'i [char],
    /// Work remaining. One unit per [`Matcher::node`] entry — every
    /// alternative the backtracker tries — plus one per CHARACTER for the two
    /// places a single entry buys a whole scan: `repeat`'s single-character
    /// fast path and a backreference's slice compare. Charging those keeps
    /// the budget a bound on work rather than merely on step count; without
    /// it a step is O(input) and the wall clock is quadratic while the
    /// counter looks linear.
    fuel: Cell<u64>,
    /// [`Matcher::node`] frames still available. Every recursion in the
    /// matcher — `repeat`'s general branch, a nested `Group`, the `seq` chain
    /// down a `Concat` — bottoms out in a `node` call whose frame stays live
    /// while its continuation runs, so counting live `node` frames counts the
    /// stack. Separate from `fuel` because the failure it prevents is
    /// different: not slowness but a stack overflow, which aborts the process
    /// natively and traps unrecoverably in wasm, where the shadow stack is
    /// fixed at link time.
    frames: Cell<u32>,
    gave_up: Cell<bool>,
}

/// How much work a single [`Regexp::match_at`] may do.
///
/// Modelled on `rustyfi-syntax`'s `Budget`, and seeded the same way — a floor
/// so a short input still gets a real answer, plus an allowance per input
/// character so a long one is not cut off for being long. Only SUPERLINEAR
/// backtracking can outrun it.
struct Budget;

impl Budget {
    /// Work units per input character.
    ///
    /// Measured, not guessed. Across all 80 patterns `satysfi-code-printer`'s
    /// `code-syntax.satyg` can produce — every `syntax-rule-line`, both halves
    /// of every `syntax-rule-block`, and every keyword list joined the way
    /// `strlst-to-syntax-rule` joins it — the worst honest cost against a
    /// deliberately hostile 200,000-character input (an unterminated string
    /// body, `"[^"]*"`, which scans to the end and then retries the closing
    /// quote at every offset) is **2 units per character**. Everything else in
    /// the corpus is O(1) in the input, because the single-character
    /// quantifier takes the iterative fast path. 64 is a 32× margin on that.
    ///
    /// The old value here was 4,096, and the cost of that slack is paid on
    /// the pathological side, because the give-up point is `PER_CHAR × len`
    /// units away: `\(a*\)*b` against 200,000 characters took **21.7 s** to
    /// give up. It now takes 0.17 s. Erring high is not free — a bound this
    /// loose is a bound in name only, since nothing in a document is
    /// interactive at twenty seconds.
    const PER_CHAR: u64 = 64;

    /// Floor, so a pattern against a short input is not capped below what a
    /// long one gets. Also measured: the most work any corpus pattern does in
    /// one anchored match is 835 units (the ~470-branch COBOL keyword
    /// alternation), so this is a 1,197× margin, and about 25 ms of trying.
    const FLOOR: u64 = 1_000_000;

    fn for_input(len: usize) -> u64 {
        ((len as u64).saturating_mul(Self::PER_CHAR)).max(Self::FLOOR)
    }

    /// Live [`Matcher::node`] frames allowed.
    ///
    /// Also measured, by bisecting the thread stack a match actually needs.
    /// At 4,096 the worst shape found needs 1.3 MB in a release build and
    /// 5.9 MB in a debug one — inside the wasm shadow stack 24× over, and
    /// inside a default 8 MB thread stack even unoptimised.
    ///
    /// This replaced a cap on `repeat`'s recursion depth alone, which did NOT
    /// bound the stack: the bytes a repeat LEVEL costs are a function of the
    /// pattern nested inside it, so while `\(a\)*` needed 2.75 MB at the old
    /// 5,000-level cap, `\(` × 60 around the same body needed **59 MB**
    /// (227 MB in a debug build) — past the wasm shadow stack, and heading
    /// for the CLI worker's 256 MB. Nor did it cover the other two recursions:
    /// a 200,000-atom `Concat` took 37.6 MB and 100,000 nested `\(` took the
    /// same, both without touching `repeat` at all. The four shapes now come
    /// in at 1.3 MB / 0.9 MB / 0.2 MB / 0.4 MB.
    const MAX_FRAMES: u32 = 4_096;

    /// `\(` nesting the PARSER will descend into; see [`Parser::nesting`].
    /// Four frames a level (`parse_atom` → `parse_alt` → `parse_concat` →
    /// `parse_repeat`) at about 850 bytes, so under a megabyte at the cap.
    /// Unbounded, `\(` × 300,000 overflowed a 256 MB stack before a single
    /// character had been matched. The deepest pattern in the corpus nests
    /// twice.
    const MAX_NESTING: usize = 1_024;
}

/// Returns a [`Matcher::frames`] slot on scope exit, so a backtracked branch
/// does not permanently consume nesting the way a bare decrement would.
struct Frame<'a, 'i>(&'a Matcher<'i>);

impl Drop for Frame<'_, '_> {
    fn drop(&mut self) {
        self.0.frames.set(self.0.frames.get() + 1);
    }
}

/// The continuation a node hands its remainder to. Returning `Some(end)`
/// means the whole match succeeded and `end` is its final offset; `None`
/// asks the node to backtrack and try its next alternative.
type Cont<'k> = &'k dyn Fn(usize, &mut Vec<Option<(usize, usize)>>) -> Option<usize>;

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl<'i> Matcher<'i> {
    fn with_budget(input: &'i [char]) -> Self {
        Matcher {
            input,
            fuel: Cell::new(Budget::for_input(input.len())),
            frames: Cell::new(Budget::MAX_FRAMES),
            gave_up: Cell::new(false),
        }
    }

    /// Spend `n` units of work. `false` means the budget is gone and the
    /// caller must unwind — every `node` alternative checks this, so
    /// exhaustion stops the whole match rather than being mistaken for a
    /// failed alternative.
    fn spend(&self, n: u64) -> bool {
        if self.gave_up.get() {
            return false;
        }
        match self.fuel.get().checked_sub(n) {
            Some(left) => {
                self.fuel.set(left);
                true
            }
            None => {
                self.fuel.set(0);
                self.gave_up.set(true);
                false
            }
        }
    }

    /// Take one step and one stack frame, returning the frame on scope exit.
    /// `None` means one of the two budgets is gone.
    fn enter(&self) -> Option<Frame<'_, 'i>> {
        if !self.spend(1) {
            return None;
        }
        match self.frames.get() {
            0 => {
                self.gave_up.set(true);
                None
            }
            n => {
                self.frames.set(n - 1);
                Some(Frame(self))
            }
        }
    }

    fn single(&self, node: &Node, pos: usize) -> Option<usize> {
        let c = *self.input.get(pos)?;
        let ok = match node {
            Node::Char(want) => c == *want,
            Node::Any => c != '\n',
            Node::Class { negated, items } => {
                let hit = items.iter().any(|it| match it {
                    ClassItem::Char(x) => c == *x,
                    ClassItem::Range(lo, hi) => *lo <= c && c <= *hi,
                });
                hit != *negated
            }
            _ => return None,
        };
        if ok {
            Some(pos + 1)
        } else {
            None
        }
    }

    fn node(
        &self,
        node: &Node,
        pos: usize,
        caps: &mut Vec<Option<(usize, usize)>>,
        k: Cont<'_>,
    ) -> Option<usize> {
        let _frame = self.enter()?;
        match node {
            Node::Empty => k(pos, caps),
            Node::Char(_) | Node::Any | Node::Class { .. } => match self.single(node, pos) {
                Some(next) => k(next, caps),
                None => None,
            },
            Node::Bol => {
                if pos == 0 || self.input.get(pos - 1) == Some(&'\n') {
                    k(pos, caps)
                } else {
                    None
                }
            }
            Node::Eol => {
                if pos == self.input.len() || self.input.get(pos) == Some(&'\n') {
                    k(pos, caps)
                } else {
                    None
                }
            }
            Node::WordBoundary => {
                let before = pos > 0 && self.input.get(pos - 1).copied().is_some_and(is_word_char);
                let after = pos < self.input.len() && is_word_char(self.input[pos]);
                if before != after {
                    k(pos, caps)
                } else {
                    None
                }
            }
            Node::Backref(n) => {
                let (s, e) = match caps.get(*n).copied().flatten() {
                    Some(span) => span,
                    // An unset group matches the empty string, as in `Str`.
                    None => return k(pos, caps),
                };
                let len = e - s;
                if pos + len > self.input.len() {
                    return None;
                }
                // The compare below is O(len) for what the entry step already
                // paid one unit for, and a group can capture the whole input:
                // `\(a*\)\1\1x` did 5e9 character compares against 200,000
                // characters while spending 200,001 units. Charge the compare.
                if !self.spend(len as u64) {
                    return None;
                }
                if self.input[pos..pos + len] == self.input[s..e] {
                    k(pos + len, caps)
                } else {
                    None
                }
            }
            Node::Group(idx, inner) => {
                let idx = *idx;
                let saved = caps.get(idx).copied().flatten();
                let start = pos;
                let out = self.node(inner, pos, caps, &|end, caps| {
                    let prev = caps[idx];
                    caps[idx] = Some((start, end));
                    match k(end, caps) {
                        Some(v) => Some(v),
                        None => {
                            caps[idx] = prev;
                            None
                        }
                    }
                });
                if out.is_none() {
                    caps[idx] = saved;
                }
                out
            }
            Node::Concat(items) => self.seq(items, pos, caps, k),
            Node::Alt(branches) => {
                for b in branches {
                    if let Some(v) = self.node(b, pos, caps, k) {
                        return Some(v);
                    }
                }
                None
            }
            Node::Repeat {
                node,
                min,
                max,
                greedy,
            } => self.repeat(node, *min, *max, *greedy, pos, caps, k),
        }
    }

    fn seq(
        &self,
        items: &[Node],
        pos: usize,
        caps: &mut Vec<Option<(usize, usize)>>,
        k: Cont<'_>,
    ) -> Option<usize> {
        // A run of single-character nodes is DETERMINISTIC — each matches one
        // character or fails, with nothing to backtrack into — so walk it
        // iteratively. Going through `node` per item would cost one live
        // frame per PATTERN character, and a literal run is the commonest
        // long thing a pattern has: `strlst-to-syntax-rule` builds a keyword
        // alternation whose every branch is one. The step accounting is
        // unchanged, one unit per item either way.
        let mut pos = pos;
        let mut items = items;
        while let Some((head, rest)) = items.split_first() {
            if !matches!(head, Node::Char(_) | Node::Any | Node::Class { .. }) {
                break;
            }
            if !self.spend(1) {
                return None;
            }
            pos = self.single(head, pos)?;
            items = rest;
        }
        match items.split_first() {
            None => k(pos, caps),
            Some((head, rest)) => self.node(head, pos, caps, &|p, caps| self.seq(rest, p, caps, k)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(
        &self,
        node: &Node,
        min: u32,
        max: Option<u32>,
        greedy: bool,
        pos: usize,
        caps: &mut Vec<Option<(usize, usize)>>,
        k: Cont<'_>,
    ) -> Option<usize> {
        // Fast path for the overwhelmingly common `X*` / `X+` where `X` is a
        // single-character matcher (`[^"]*`, `[a-z]+`, `.*`). Recursing once
        // per repetition would make stack depth proportional to input length;
        // a code block a few thousand characters long would then risk
        // overflowing on a pattern as ordinary as a string literal's body.
        if matches!(node, Node::Char(_) | Node::Any | Node::Class { .. }) {
            // One `single` call per repetition is real work that the single
            // step this `node` entry already paid for would otherwise buy
            // without limit, so clamp the scan to what is left of the budget
            // and charge what it consumes.
            let hard = max.map_or(u64::MAX, u64::from);
            let afford = self.fuel.get();
            let cap = hard.min(afford);
            let mut cur = pos;
            let mut count = 0u64;
            while count < cap {
                match self.single(node, cur) {
                    Some(next) => {
                        cur = next;
                        count += 1;
                    }
                    None => break,
                }
            }
            self.fuel.set(afford - count);
            if count == cap && cap < hard {
                // Stopped because the budget ran out, not because the input
                // did: a truncated repetition would be a WRONG answer.
                self.gave_up.set(true);
                return None;
            }
            // A single-character node consumes exactly one character, so the
            // end offset for `i` repetitions is `pos + i` — the admissible
            // ones run from `min` to `count`. (This used to be materialised as
            // a `Vec` of offsets, which is an O(input) allocation per scan for
            // a sequence that is just addition.)
            let lo = u64::from(min);
            if count < lo {
                return None;
            }
            let mut try_at = |i: u64| k(pos + i as usize, caps);
            if greedy {
                for i in (lo..=count).rev() {
                    if let Some(v) = try_at(i) {
                        return Some(v);
                    }
                }
            } else {
                for i in lo..=count {
                    if let Some(v) = try_at(i) {
                        return Some(v);
                    }
                }
            }
            return None;
        }

        // General case: `node` can consume a variable amount, so recurse —
        // one `node` frame per repetition, which is what `Budget::MAX_FRAMES`
        // bounds, along with every other recursion in the matcher.
        let more = |m: &Matcher<'i>, p: usize, caps: &mut Vec<Option<(usize, usize)>>| {
            if max == Some(0) {
                return None;
            }
            let next_min = min.saturating_sub(1);
            let next_max = max.map(|m| m - 1);
            m.node(node, p, caps, &|q, caps| {
                // A zero-width body would otherwise loop forever.
                if q == p && next_min == 0 {
                    return None;
                }
                m.repeat(node, next_min, next_max, greedy, q, caps, k)
            })
        };

        if min > 0 {
            return more(self, pos, caps);
        }
        if greedy {
            match more(self, pos, caps) {
                Some(v) => Some(v),
                None => k(pos, caps),
            }
        } else {
            match k(pos, caps) {
                Some(v) => Some(v),
                None => more(self, pos, caps),
            }
        }
    }
}

thread_local! {
    /// `code-printer` calls `regexp-of-string` inside its lexer loop, and this
    /// port's `regexp` is the pattern string itself, so the same handful of
    /// patterns are re-parsed once per scanned character. Memoizing keeps that
    /// linear in the input rather than in input × pattern length.
    static CACHE: RefCell<HashMap<String, Rc<Regexp>>> = RefCell::new(HashMap::new());

    /// Total pattern TEXT cached, in chars. The cap is on text size rather
    /// than on entry count because the entries are not uniform: a parsed
    /// pattern is
    /// roughly thirty times its own text, so 4,096 twenty-thousand-character
    /// patterns — cheap to generate from a document, `arabic i ^ body` will do
    /// — reach 2.6 GB while never approaching an entry-count limit.
    static CACHE_CHARS: Cell<usize> = const { Cell::new(0) };
}

/// Cached pattern text allowed before the table is dropped, in chars. The
/// real corpus caches a few hundred characters in total.
const CACHE_CHAR_LIMIT: usize = 4 * 1024 * 1024;

/// Parse `pattern`, reusing a previously parsed copy when possible.
pub fn compile(pattern: &str) -> Rc<Regexp> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(re) = c.get(pattern) {
            return Rc::clone(re);
        }
        // The pattern set a document uses is small and fixed (one per
        // language rule), but cap the table anyway so a program generating
        // patterns cannot grow it without bound.
        CACHE_CHARS.with(|n| {
            if n.get() + pattern.chars().count() > CACHE_CHAR_LIMIT {
                c.clear();
                n.set(0);
            }
            n.set(n.get() + pattern.chars().count());
        });
        let re = Rc::new(Regexp::parse(pattern));
        c.insert(pattern.to_string(), Rc::clone(&re));
        re
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(pat: &str, input: &str) -> Option<String> {
        let re = Regexp::parse(pat);
        let chars: Vec<char> = input.chars().collect();
        re.match_at(&chars, 0)
            .expect("this pattern must not exhaust the budget")
            .map(|end| chars[..end].iter().collect())
    }

    /// `scan`, for a pattern that is EXPECTED to run out of budget.
    fn scan_gives_up(pat: &str, input: &str) -> bool {
        let re = Regexp::parse(pat);
        let chars: Vec<char> = input.chars().collect();
        re.match_at(&chars, 0).is_err()
    }

    #[test]
    fn literals_and_escapes() {
        assert_eq!(scan("abc", "abcdef").as_deref(), Some("abc"));
        assert_eq!(scan("abc", "abdef"), None);
        // `(` and `|` are LITERAL in `Str`, unlike PCRE.
        assert_eq!(scan("(a)", "(a)b").as_deref(), Some("(a)"));
        assert_eq!(scan("a|b", "a|b").as_deref(), Some("a|b"));
        assert_eq!(scan("{2}", "{2}x").as_deref(), Some("{2}"));
        assert_eq!(scan(r"\*", "*x").as_deref(), Some("*"));
    }

    #[test]
    fn alternation_is_leftmost_biased_not_longest() {
        // `Str`, like Perl and unlike POSIX, returns the FIRST branch that
        // matches rather than the longest.
        assert_eq!(scan(r"a\|ab", "ab").as_deref(), Some("a"));
        assert_eq!(scan(r"ab\|a", "ab").as_deref(), Some("ab"));
    }

    #[test]
    fn quantifiers_greedy_and_lazy() {
        assert_eq!(scan("a*", "aaab").as_deref(), Some("aaa"));
        assert_eq!(scan("a*?", "aaab").as_deref(), Some(""));
        assert_eq!(scan("a+", "aaab").as_deref(), Some("aaa"));
        assert_eq!(scan("a+?", "aaab").as_deref(), Some("a"));
        assert_eq!(scan("ab?", "ab").as_deref(), Some("ab"));
        assert_eq!(scan("ab?", "ac").as_deref(), Some("a"));
        assert_eq!(scan("a+", "b"), None);
    }

    #[test]
    fn character_classes() {
        assert_eq!(scan("[a-z]+", "abcD").as_deref(), Some("abc"));
        assert_eq!(scan("[^\"]*", "ab\"c").as_deref(), Some("ab"));
        assert_eq!(scan("[]a]+", "]a]b").as_deref(), Some("]a]"));
        assert_eq!(scan("[a-]+", "a-a!").as_deref(), Some("a-a"));
        // Backslash is not an escape inside a set: `[\t]` is `{'\\','t'}`.
        assert_eq!(scan(r"[\t]+", r"t\t;").as_deref(), Some(r"t\t"));
    }

    #[test]
    fn anchors() {
        assert_eq!(scan("$", "").as_deref(), Some(""));
        assert_eq!(scan("$", "\nx").as_deref(), Some(""));
        assert_eq!(scan("$", "x\n"), None);
        assert_eq!(scan("^a", "a"). as_deref(), Some("a"));
    }

    #[test]
    fn groups_and_backrefs() {
        assert_eq!(scan(r"\(ab\)+", "ababc").as_deref(), Some("abab"));
        assert_eq!(scan(r"\(a\|b\)c", "bc").as_deref(), Some("bc"));
        assert_eq!(scan(r"\(a\)\1", "aa").as_deref(), Some("aa"));
        assert_eq!(scan(r"\(a\)\1", "ab"), None);
    }

    #[test]
    fn code_printer_real_patterns() {
        // The SATySFi identifier rule from `code-syntax.satyg`.
        let ident = r"\(\\\|\+\)?[a-zA-Z][a-zA-Z0-9-]*\|[0-9]+\|0x[0-9a-fA-F]+";
        assert_eq!(scan(ident, "let-rec x").as_deref(), Some("let-rec"));
        assert_eq!(scan(ident, r"\emph{a}").as_deref(), Some(r"\emph"));
        assert_eq!(scan(ident, "+section{}").as_deref(), Some("+section"));
        assert_eq!(scan(ident, "123abc").as_deref(), Some("123"));
        // The rule's own third branch, `0x[0-9a-fA-F]+`, is unreachable: the
        // second branch `[0-9]+` is tried first and always matches the
        // leading `0`. That is what `Str` does too — the port must reproduce
        // the quirk, not "fix" it, or highlighting would drift from upstream.
        assert_eq!(scan(ident, "0x1F;").as_deref(), Some("0"));

        // Rust identifiers, and the block-comment delimiters.
        assert_eq!(scan("[a-zA-Z][a-zA-Z0-9_]*!?", "println!(").as_deref(), Some("println!"));
        assert_eq!(scan(r"/\*", "/* c */").as_deref(), Some("/*"));
        assert_eq!(scan(r"\*/", "*/ rest").as_deref(), Some("*/"));
        // A double-quoted string body.
        assert_eq!(scan("\"[^\"]*\"", "\"hi\" x").as_deref(), Some("\"hi\""));
    }

    #[test]
    fn long_repeat_does_not_overflow_the_stack() {
        // The single-character fast path must keep this iterative.
        let long: String = std::iter::repeat('a').take(200_000).collect();
        assert_eq!(scan("[a-z]*", &long).map(|s| s.len()), Some(200_000));
    }

    /// The case the test above does NOT cover, and which used to abort the
    /// process: a repeated GROUP takes `repeat`'s recursive branch, one frame
    /// per repetition. `[a-z]*` above is the single-character fast path — it
    /// was never at risk, so it could not have caught this.
    #[test]
    fn a_repeated_group_over_a_long_input_gives_up_instead_of_overflowing() {
        let long: String = std::iter::repeat('a').take(200_000).collect();
        // Either it matches or it gives up; what it must not do is abort.
        let re = Regexp::parse("\\(a\\)*");
        let chars: Vec<char> = long.chars().collect();
        let _ = re.match_at(&chars, 0);
    }

    /// Catastrophic backtracking is bounded. Unbounded, this took two minutes
    /// at thirty characters and did not finish at thirty-two.
    #[test]
    fn a_quantified_group_inside_a_quantifier_gives_up_rather_than_hanging() {
        let input: String = std::iter::repeat('a').take(64).collect::<String>() + "!";
        assert!(
            scan_gives_up("\\(a*\\)*b", &input),
            "the exponential pattern should have exhausted its budget"
        );
        // And the budget must not fire on ordinary work: the same shape that
        // CAN match still does, promptly.
        assert_eq!(scan("\\(a*\\)*b", "aaab").as_deref(), Some("aaab"));
    }

    /// `match_at` is `pub` and takes an arbitrary `start`; `^` and `\b` used
    /// to index `input[pos - 1]` unguarded and panic past the end.
    ///
    /// The three patterns below the fold are the ones that actually REACH the
    /// anchor: `"a$"` does not, because the `a` fails on `input.get(pos)`
    /// first and the `$` is never evaluated — so it was `$`, indexing
    /// `input[pos]` forward, that was still panicking after `^` and `\b` were
    /// fixed. A guard test has to be written against the node under test, not
    /// against a pattern that merely contains it.
    #[test]
    fn match_at_past_the_end_does_not_panic() {
        let chars: Vec<char> = "abc".chars().collect();
        for pat in ["^a", "\\ba", "a$", "$", "a*$", "^", "\\b", "\\(a\\)*$"] {
            let _ = Regexp::parse(pat).match_at(&chars, 7);
        }
    }

    #[test]
    fn search_forward_finds_leftmost() {
        let re = Regexp::parse("b+");
        let chars: Vec<char> = "aabbbc".chars().collect();
        assert_eq!(re.search_from(&chars, 0), Ok(Some((2, 5))));
        assert_eq!(re.search_from(&chars, 5), Ok(None));
    }

    /// `search_from` tries every start position, so a per-position budget
    /// would let the total reach `len × budget` — the bound would stop
    /// bounding exactly when the input got long. One shared budget means a
    /// pattern that is pathological at ONE position is refused for the whole
    /// search rather than being paid for at every one of them.
    #[test]
    fn a_leftmost_search_shares_one_budget_across_start_positions() {
        let input: String = "a".repeat(4_000);
        let chars: Vec<char> = input.chars().collect();
        let re = Regexp::parse("\\(a*\\)*b");
        let t0 = std::time::Instant::now();
        assert_eq!(re.search_from(&chars, 0), Err(GaveUp));
        // Per-position budgets would multiply this by 4,000.
        assert!(
            t0.elapsed().as_secs() < 5,
            "the search re-seeded its budget per start position"
        );
    }

    /// The work a step buys must be bounded, or the counter is linear while
    /// the clock is quadratic: the group here captures a prefix of the input
    /// and the backreference compares it character by character, so one step
    /// used to buy an O(input) `memcmp`. 200,000 characters cost 5e9 compares
    /// for 200,001 steps.
    #[test]
    fn a_backreference_is_charged_for_the_characters_it_compares() {
        let input: String = "a".repeat(20_000);
        let chars: Vec<char> = input.chars().collect();
        let t0 = std::time::Instant::now();
        assert!(Regexp::parse("\\(a*\\)\\1\\1x").match_at(&chars, 0).is_err());
        assert!(t0.elapsed().as_secs() < 5, "the compare was not charged");
        // A backreference to a SHORT group — the shape real patterns use, a
        // matching quote or bracket — is unaffected.
        assert_eq!(scan(r"\(['`]\)[a-z]*\1", "'abc'!").as_deref(), Some("'abc'"));
    }

    /// Same point for `repeat`'s single-character fast path, which scans the
    /// whole input for the one step its `node` entry paid for. Charging it is
    /// what keeps a give-up fast: `\(a*\)*b` against 200,000 characters took
    /// 21.7 s to give up before, and 0.17 s after.
    #[test]
    fn a_quantifier_scan_is_charged_for_the_characters_it_consumes() {
        let input: String = "a".repeat(200_000);
        let chars: Vec<char> = input.chars().collect();
        let t0 = std::time::Instant::now();
        assert!(Regexp::parse("\\(a*\\)*b").match_at(&chars, 0).is_err());
        assert!(
            t0.elapsed().as_secs() < 10,
            "giving up took longer than doing the work would have"
        );
        // The charge must not make an honest scan of the same input fail.
        assert_eq!(scan("[a-z]*", &input).map(|s| s.len()), Some(200_000));
    }

    /// A pattern nested deeper than [`Budget::MAX_NESTING`] drives the PARSER
    /// off the stack, before a single character has been matched — `\(` ×
    /// 300,000 overflowed a 256 MB stack. The parser stops descending, and
    /// because the tree it then has is a DIFFERENT well-formed pattern rather
    /// than a malformed one read literally, matching it reports the give-up
    /// instead of answering.
    #[test]
    fn a_pattern_nested_past_the_parser_cap_is_refused_not_reinterpreted() {
        let deep = format!("{}a{}", r"\(".repeat(100_000), r"\)".repeat(100_000));
        let chars: Vec<char> = "a".chars().collect();
        assert_eq!(Regexp::parse(&deep).match_at(&chars, 0), Err(GaveUp));
        // Just inside the cap still works.
        let ok = format!("{}a{}", r"\(".repeat(1_000), r"\)".repeat(1_000));
        assert_eq!(Regexp::parse(&ok).match_at(&chars, 0), Ok(Some(1)));
    }

    /// A long run of single-character atoms is deterministic, so `seq` walks
    /// it iteratively. Recursing per atom made the STACK proportional to the
    /// pattern's length — 200,000 literal characters needed 37.6 MB — which
    /// no cap on `repeat`'s depth could see.
    #[test]
    fn a_long_literal_run_does_not_recurse_per_character() {
        let long: String = "a".repeat(200_000);
        assert_eq!(scan(&long, &long).map(|s| s.len()), Some(200_000));
    }
}
