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
}

struct Parser<'p> {
    src: &'p [char],
    pos: usize,
    groups: usize,
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
                    '(' => {
                        self.groups += 1;
                        let idx = self.groups;
                        let inner = self.parse_alt();
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
        };
        let root = p.parse_alt();
        Regexp {
            root,
            groups: p.groups,
        }
    }

    /// Anchored match at `start`, returning the end offset (in `char`s) of the
    /// match — `Str.string_match` plus `Str.match_end`.
    ///
    /// A pattern that exhausts its step budget reports `Err(GaveUp)` rather
    /// than "no match": see [`Budget`]. Silently answering `None` would turn a
    /// hang into wrong output, which is worse.
    pub fn match_at(&self, input: &[char], start: usize) -> Result<Option<usize>, GaveUp> {
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
        // loop built — makes the total `input.len() × budget`, i.e. the budget
        // stops bounding anything as soon as the input is long, which is
        // precisely when it needs to.
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
    /// Steps remaining. Decremented once per [`Matcher::node`] entry, which
    /// is every alternative the backtracker tries.
    fuel: Cell<u64>,
    /// Remaining nesting depth for [`Matcher::repeat`]'s general branch, the
    /// one that recurses once per repetition. Separate from `fuel` because
    /// the failure it prevents is different: not slowness but a stack
    /// overflow, which aborts the process natively and traps unrecoverably
    /// in wasm, where the shadow stack is fixed at link time.
    depth: Cell<u32>,
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
    /// Steps per input character. A linear scan costs a handful per
    /// character, so this is orders of magnitude of headroom.
    const PER_CHAR: u64 = 4_096;

    /// Floor, so a pattern against a short input is not capped below what a
    /// long one gets. At roughly 10M steps a second this is about a tenth of
    /// a second of trying.
    const FLOOR: u64 = 1_000_000;

    fn for_input(len: usize) -> u64 {
        ((len as u64).saturating_mul(Self::PER_CHAR)).max(Self::FLOOR)
    }

    /// Nesting cap for the recursive repeat branch. Each level costs roughly
    /// 650 bytes of stack, so this is about 3 MB at the limit — comfortably
    /// inside the wasm build's 32 MB shadow stack and the CLI's 256 MB
    /// worker, while still allowing far more nesting than any real pattern.
    const MAX_DEPTH: u32 = 5_000;
}

/// Returns a [`Matcher::depth`] unit on scope exit, so a backtracked branch
/// does not permanently consume nesting the way a bare decrement would.
struct DepthGuard<'a, 'i>(&'a Matcher<'i>);

impl Drop for DepthGuard<'_, '_> {
    fn drop(&mut self) {
        self.0.depth.set(self.0.depth.get() + 1);
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
            depth: Cell::new(Budget::MAX_DEPTH),
            gave_up: Cell::new(false),
        }
    }

    /// Spend one step. `false` means the budget is gone and the caller must
    /// unwind — every `node` alternative checks this, so exhaustion stops the
    /// whole match rather than being mistaken for a failed alternative.
    fn spend(&self) -> bool {
        if self.gave_up.get() {
            return false;
        }
        match self.fuel.get() {
            0 => {
                self.gave_up.set(true);
                false
            }
            f => {
                self.fuel.set(f - 1);
                true
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
        if !self.spend() {
            return None;
        }
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
                if pos + len <= self.input.len() && self.input[pos..pos + len] == self.input[s..e] {
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
            let mut ends: Vec<usize> = vec![pos];
            let mut cur = pos;
            let cap = max.unwrap_or(u32::MAX);
            let mut count = 0u32;
            while count < cap {
                match self.single(node, cur) {
                    Some(next) => {
                        cur = next;
                        ends.push(cur);
                        count += 1;
                    }
                    None => break,
                }
            }
            // `ends` holds one offset per admissible repetition count, from 0
            // up to however many matched, so `min` repetitions are available
            // exactly when `ends.len() > min`.
            let lo = min as usize;
            if ends.len() <= lo {
                return None;
            }
            if greedy {
                for i in (lo..ends.len()).rev() {
                    if let Some(v) = k(ends[i], caps) {
                        return Some(v);
                    }
                }
            } else {
                for i in lo..ends.len() {
                    if let Some(v) = k(ends[i], caps) {
                        return Some(v);
                    }
                }
            }
            return None;
        }

        // General case: `node` can consume a variable amount, so recurse —
        // one frame per repetition, which is what the depth cap bounds.
        let more = |m: &Matcher<'i>, p: usize, caps: &mut Vec<Option<(usize, usize)>>| {
            if max == Some(0) {
                return None;
            }
            match m.depth.get() {
                0 => {
                    m.gave_up.set(true);
                    return None;
                }
                d => m.depth.set(d - 1),
            }
            let _restore = DepthGuard(m);
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

    /// Total pattern TEXT cached, in chars. The cap is on bytes rather than on
    /// entry count because the entries are not uniform: a parsed pattern is
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
    /// The patterns below the fold are the ones that actually REACH the
    /// anchor: `"a$"` does not, because the `a` fails on `input.get(pos)`
    /// first and the `$` is never evaluated — so `$`, indexing `input[pos]`
    /// forward, was still panicking after `^` and `\b` had been fixed. A
    /// guard test has to be written against the node under test, not against
    /// a pattern that merely contains it.
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
}
