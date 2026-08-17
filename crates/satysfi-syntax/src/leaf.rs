//! Leaf token types: one small struct per keyword/punctuation/payload token,
//! each with hand-written `Parse<Atom>`/`Unparse<Atom>`/`Spanned` (the
//! peek-next-pushback pattern), plus the delimiter `Group` aliases.

use crate::span::Span;
use crate::token::{Atom, Token};
use syan::error::ParseError;
use syan::nested::group::EmptyGroup;
use syan::parse::unparse::Emitter;
use syan::parse::{IntoParseStream, Parse, ParseStream, Unparse};
use syan::span::Spanned;

/// Unit-payload leaves: `$name` holds only the matched token's span.
macro_rules! unit_tokens {
    ($($(#[$doc:meta])* $name:ident => $variant:ident, $desc:literal;)*) => {
        $(
            $(#[$doc])*
            #[derive(Clone, Debug, Default, PartialEq)]
            pub struct $name(pub Span);

            impl Parse<Atom> for $name {
                type Error = ParseError;

                fn parse(stream: impl IntoParseStream<Atom = Atom>) -> Result<Self, Self::Error> {
                    let mut stream = stream.into_parse_stream();
                    match stream.next() {
                        Some(atom) if atom.slot == Token::$variant => Ok($name(atom.span)),
                        Some(atom) => {
                            let span = atom.span;
                            stream.push(atom);
                            Err(ParseError::new(span, concat!("expected ", $desc)))
                        }
                        None => Err(ParseError::new(
                            Span::default(),
                            concat!("expected ", $desc, ", found end of input"),
                        )),
                    }
                }
            }

            impl Unparse<Atom> for $name {
                fn unparse<S: Emitter<Atom>>(&self, sink: &mut S) -> Result<(), S::Error> {
                    sink.write_one(Atom { slot: Token::$variant, span: self.0 })
                }
            }

            impl Spanned for $name {
                type Span = Span;
                fn span(&self) -> Span {
                    self.0
                }
            }
        )*
    };
}

// TODO: use puncts under syan::symbol::chars if possible
unit_tokens! {
    /// `let`
    KwLet => Let, "'let'";
    /// `let-rec`
    KwLetRec => LetRec, "'let-rec'";
    /// `let-inline`
    KwLetHorz => LetHorz, "'let-inline'";
    /// `let-block`
    KwLetVert => LetVert, "'let-block'";
    /// `let-math`
    KwLetMath => LetMath, "'let-math'";
    /// `and`
    KwAnd => LetAnd, "'and'";
    /// `in`
    KwIn => In, "'in'";
    /// `fun`
    KwFun => Fun, "'fun'";
    /// `if`
    KwIf => If, "'if'";
    /// `then`
    KwThen => Then, "'then'";
    /// `else`
    KwElse => Else, "'else'";
    /// `true`
    KwTrue => True, "'true'";
    /// `false`
    KwFalse => False, "'false'";
    /// `->`
    ArrowTok => Arrow, "'->'";
    /// `=`
    DefEqTok => DefEq, "'='";
    /// `;` in program mode (list/record separator)
    ListPunctTok => ListPunct, "';'";
    /// `,`
    CommaTok => Comma, "','";
    /// `(`
    LParenTok => LParen, "'('";
    /// `)`
    RParenTok => RParen, "')'";
    /// `(|`
    BRecordTok => BRecord, "'(|'";
    /// `|)`
    ERecordTok => ERecord, "'|)'";
    /// `[`
    BListTok => BList, "'['";
    /// `]`
    EListTok => EList, "']'";
    /// `{` opening inline text
    BHorzGrpTok => BHorzGrp, "'{'";
    /// `}` closing inline text
    EHorzGrpTok => EHorzGrp, "'}'";
    /// `'<` / `<` opening block text
    BVertGrpTok => BVertGrp, "'<'";
    /// `>` closing block text
    EVertGrpTok => EVertGrp, "'>'";
    /// a space in inline text
    SpaceTok => Space, "a space";
    /// a line break in inline text
    BreakTok => Break, "a line break";
    /// `;` terminating an active command area
    EndActiveTok => EndActive, "';'";
    /// end of input
    EoiTok => Eoi, "end of input";
}

/// Payload-carrying leaves: match one token variant and keep its data.
macro_rules! payload_tokens {
    ($(
        $(#[$doc:meta])* $name:ident($($field:ident: $fty:ty),*) => $variant:ident, $desc:literal;
    )*) => {
        $(
            $(#[$doc])*
            #[derive(Clone, Debug, PartialEq)]
            pub struct $name {
                $(pub $field: $fty,)*
                pub span: Span,
            }

            impl Parse<Atom> for $name {
                type Error = ParseError;

                fn parse(stream: impl IntoParseStream<Atom = Atom>) -> Result<Self, Self::Error> {
                    let mut stream = stream.into_parse_stream();
                    match stream.next() {
                        Some(Atom { slot: Token::$variant($($field),*), span }) => {
                            Ok($name { $($field,)* span })
                        }
                        Some(atom) => {
                            let span = atom.span;
                            stream.push(atom);
                            Err(ParseError::new(span, concat!("expected ", $desc)))
                        }
                        None => Err(ParseError::new(
                            Span::default(),
                            concat!("expected ", $desc, ", found end of input"),
                        )),
                    }
                }
            }

            impl Unparse<Atom> for $name {
                fn unparse<S: Emitter<Atom>>(&self, sink: &mut S) -> Result<(), S::Error> {
                    sink.write_one(Atom {
                        slot: Token::$variant($(self.$field.clone()),*),
                        span: self.span,
                    })
                }
            }

            impl Spanned for $name {
                type Span = Span;
                fn span(&self) -> Span {
                    self.span
                }
            }
        )*
    };
}

payload_tokens! {
    /// A lowercase identifier.
    VarTok(name: String) => Var, "a variable name";
    /// An uppercase constructor name.
    CtorTok(name: String) => Constructor, "a constructor name";
    /// An integer constant.
    IntTok(value: i64) => IntConst, "an integer constant";
    /// A float constant.
    FloatTok(value: f64) => FloatConst, "a float constant";
    /// An inline command name (`\cmd`), sigil included.
    HorzCmdTok(name: String) => HorzCmd, "an inline command";
    /// A block command name (`+cmd`), sigil included.
    VertCmdTok(name: String) => VertCmd, "a block command";
    /// A run of plain inline-text characters.
    CharTok(text: String) => Char, "inline text";
    /// An itemize bullet (`*`+).
    ItemTok(depth: usize) => Item, "an itemize bullet";
    /// An `@require:` header.
    HeaderRequireTok(content: String) => HeaderRequire, "'@require:'";
    /// An `@import:` header.
    HeaderImportTok(content: String) => HeaderImport, "'@import:'";
}

/// A length constant such as `12pt` (two payload fields, hand-written).
#[derive(Clone, Debug, PartialEq)]
pub struct LengthTok {
    pub value: f64,
    pub unit: String,
    pub span: Span,
}

impl Parse<Atom> for LengthTok {
    type Error = ParseError;

    fn parse(stream: impl IntoParseStream<Atom = Atom>) -> Result<Self, Self::Error> {
        let mut stream = stream.into_parse_stream();
        match stream.next() {
            Some(Atom {
                slot: Token::LengthConst(value, unit),
                span,
            }) => Ok(LengthTok { value, unit, span }),
            Some(atom) => {
                let span = atom.span;
                stream.push(atom);
                Err(ParseError::new(span, "expected a length constant"))
            }
            None => Err(ParseError::new(
                Span::default(),
                "expected a length constant, found end of input",
            )),
        }
    }
}

impl Unparse<Atom> for LengthTok {
    fn unparse<S: Emitter<Atom>>(&self, sink: &mut S) -> Result<(), S::Error> {
        sink.write_one(Atom {
            slot: Token::LengthConst(self.value, self.unit.clone()),
            span: self.span,
        })
    }
}

impl Spanned for LengthTok {
    type Span = Span;
    fn span(&self) -> Span {
        self.span
    }
}

/// A backtick string literal with its space-trimming flags.
#[derive(Clone, Debug, PartialEq)]
pub struct LiteralTok {
    pub body: String,
    pub omit_pre: bool,
    pub omit_post: bool,
    pub span: Span,
}

impl Parse<Atom> for LiteralTok {
    type Error = ParseError;

    fn parse(stream: impl IntoParseStream<Atom = Atom>) -> Result<Self, Self::Error> {
        let mut stream = stream.into_parse_stream();
        match stream.next() {
            Some(Atom {
                slot:
                    Token::Literal {
                        body,
                        omit_pre,
                        omit_post,
                    },
                span,
            }) => Ok(LiteralTok {
                body,
                omit_pre,
                omit_post,
                span,
            }),
            Some(atom) => {
                let span = atom.span;
                stream.push(atom);
                Err(ParseError::new(span, "expected a string literal"))
            }
            None => Err(ParseError::new(
                Span::default(),
                "expected a string literal, found end of input",
            )),
        }
    }
}

impl Unparse<Atom> for LiteralTok {
    fn unparse<S: Emitter<Atom>>(&self, sink: &mut S) -> Result<(), S::Error> {
        sink.write_one(Atom {
            slot: Token::Literal {
                body: self.body.clone(),
                omit_pre: self.omit_pre,
                omit_post: self.omit_post,
            },
            span: self.span,
        })
    }
}

impl Spanned for LiteralTok {
    type Span = Span;
    fn span(&self) -> Span {
        self.span
    }
}

// ---- delimiter groups ---------------------------------------------------------

/// Local delimiter-group structs (the foreign `Group<T, O, C>` can't receive a
/// generic `Unparse<Atom>` impl under the orphan rules). Each behaves exactly
/// like `syan::nested::group::Group`: sequential parse, delimiter-only span,
/// `EmptyGroup` so `#[group(self.field)]` content association works.
macro_rules! define_groups {
    ($($(#[$doc:meta])* $name:ident: $open:ident, $close:ident;)*) => {
        $(
            $(#[$doc])*
            #[derive(Clone, Debug, PartialEq)]
            pub struct $name<T> {
                pub open: $open,
                pub slot: T,
                pub close: $close,
            }

            impl<T> Parse<Atom> for $name<T>
            where
                T: Parse<Atom>,
            {
                type Error = ParseError;

                fn parse(stream: impl IntoParseStream<Atom = Atom>) -> Result<Self, Self::Error> {
                    let mut stream = stream.into_parse_stream();
                    let open = <$open>::parse(&mut stream)?;
                    let slot =
                        T::parse(&mut stream).map_err(syan::error::Error::into_parse_error)?;
                    let close = <$close>::parse(&mut stream)?;
                    Ok($name { open, slot, close })
                }
            }

            impl<T> Unparse<Atom> for $name<T>
            where
                T: Unparse<Atom>,
            {
                fn unparse<S: Emitter<Atom>>(&self, sink: &mut S) -> Result<(), S::Error> {
                    self.open.unparse(sink)?;
                    self.slot.unparse(sink)?;
                    self.close.unparse(sink)
                }
            }

            // The delimiters alone carry the span, so an empty group still has one.
            impl<T> Spanned for $name<T> {
                type Span = Span;
                fn span(&self) -> Span {
                    self.open.span().unite(self.close.span())
                }
            }

            impl EmptyGroup for $name<()> {
                type Fill<Slot> = $name<Slot>;

                fn fill<Slot>(self, slot: Slot) -> Self::Fill<Slot> {
                    $name { open: self.open, slot, close: self.close }
                }

                fn unfill<Slot>(group: Self::Fill<Slot>) -> (Slot, Self) {
                    (
                        group.slot,
                        $name { open: group.open, slot: (), close: group.close },
                    )
                }
            }

            impl<T> std::ops::Deref for $name<T> {
                type Target = T;
                fn deref(&self) -> &T {
                    &self.slot
                }
            }
        )*
    };
}

// TODO: implement Unparse in Group in ../syan2
// TODO: use type reference to Parenthesis / Group in syan core to support TokenStream2 as much as
// possible
define_groups! {
    /// `( … )` in program mode.
    ParenGroup: LParenTok, RParenTok;
    /// `(| … |)` record.
    RecordGroup: BRecordTok, ERecordTok;
    /// `[ … ]` list.
    ListGroup: BListTok, EListTok;
    /// `{ … }` inline text.
    InlineGroup: BHorzGrpTok, EHorzGrpTok;
    /// `'< … >` / `< … >` block text.
    BlockGroup: BVertGrpTok, EVertGrpTok;
}
