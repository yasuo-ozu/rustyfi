//! A local stand-in for syan's `#[derive(TokenLeaves)]`.
//!
//! `TokenLeaves` generates, per annotated `Token` variant, a leaf struct with
//! the `Parse`/`Unparse`/`Spanned` trio. It lives on syan's `api-ergonomics`
//! branch only (`ergo-g4-tokenleaves`); syan's `main` has no such derive, which
//! is most of why this crate cannot build against it. This macro reproduces the
//! generated code from this side, so the token layer stops depending on that
//! branch.
//!
//! Ported against `syan2/macro/attribute/token_leaves.rs:209-262` — the same
//! peek/match/pushback parse, the same two error arms, the same `write_one`
//! unparse. Differences are only what `macro_rules!` cannot do: it cannot
//! synthesize identifiers, so each leaf name is passed explicitly (the derive
//! read it from `#[leaf(name = "..")]`), and it cannot inspect the enum, so the
//! variant list is maintained by hand here rather than derived from `Token`.
//!
//! NOT YET WIRED IN. `token.rs` still uses the derive; the `tests` module below
//! type-checks this macro against the real `Atom`/`Token` under throwaway leaf
//! names so it is known-good before the ~99 declarations are migrated.

/// Generate leaf structs + `Parse`/`Unparse`/`Spanned`, one per rule.
///
/// ```ignore
/// token_leaves! {
///     atom = Atom, span = Span, read_span = |a| a.span;
///     (Let => KwLet, "'let'");
///     (Var(String) => VarTok, "a variable", field = name);
/// }
/// ```
macro_rules! token_leaves {
    (
        atom = $atom:ty, span = $span:ty, read_span = $read:expr;
        $( $rule:tt );* $(;)?
    ) => {
        $( token_leaves!(@one $atom, $span, $read, $rule); )*
    };

    // ---- unit variant: `Let` -> `struct KwLet(pub Span)` --------------------
    (@one $atom:ty, $span:ty, $read:expr,
     ($variant:ident => $leaf:ident, $expect:literal)) => {
        #[derive(Clone, Debug)]
        pub struct $leaf(pub $span);

        impl ::syan::parse::parse::Parse<$atom> for $leaf {
            type Error = ::syan::error::ParseError<$span>;
            fn parse_stream<__SyanS: ::syan::parse::parse_stream::ParseStream<Atom = $atom>>(
                __stream: &mut __SyanS,
            ) -> ::core::result::Result<Self, Self::Error> {
                use ::syan::parse::parse_stream::ParseStream;
                // Bound to a `fn` pointer so the `&atom` parameter type is
                // fixed — a bare closure cannot infer it from an immediate
                // call (the derive does the same, and says so).
                let __read_span: fn(&$atom) -> $span = $read;
                match ParseStream::next(__stream) {
                    ::core::option::Option::Some(__atom) => match &__atom.slot {
                        $crate::token::Token::$variant => {
                            let __span = __read_span(&__atom);
                            ::core::result::Result::Ok($leaf(__span))
                        }
                        _ => {
                            let __span = __read_span(&__atom);
                            // Pushback is the whole design: a failed leaf must
                            // leave the stream untouched so enum-variant
                            // backtracking can try the next alternative.
                            ParseStream::push(__stream, __atom);
                            ::core::result::Result::Err(::syan::error::ParseError::expected(__span, $expect))
                        }
                    },
                    ::core::option::Option::None => {
                        ::core::result::Result::Err(::syan::error::ParseError::eof(<$span as ::core::default::Default>::default()))
                    }
                }
            }
        }

        impl ::syan::parse::unparse::Unparse<$atom> for $leaf {
            fn unparse<__E: ::syan::parse::unparse::Emitter<$atom>>(
                &self,
                __sink: &mut __E,
            ) -> ::core::result::Result<(), __E::Error> {
                type __A = $atom;
                ::syan::parse::unparse::Emitter::write_one(
                    __sink,
                    __A {
                        slot: $crate::token::Token::$variant,
                        span: self.0.clone(),
                    },
                )
            }
        }

        impl ::syan::span::Spanned for $leaf {
            type Span = $span;
            fn span(&self) -> Self::Span {
                self.0.clone()
            }
        }
    };

    // ---- single-field variant: `Var(String)` -> `struct VarTok { name, span }`
    (@one $atom:ty, $span:ty, $read:expr,
     ($variant:ident($fty:ty) => $leaf:ident, $expect:literal, field = $field:ident)) => {
        #[derive(Clone, Debug)]
        pub struct $leaf {
            pub $field: $fty,
            pub span: $span,
        }

        impl ::syan::parse::parse::Parse<$atom> for $leaf {
            type Error = ::syan::error::ParseError<$span>;
            fn parse_stream<__SyanS: ::syan::parse::parse_stream::ParseStream<Atom = $atom>>(
                __stream: &mut __SyanS,
            ) -> ::core::result::Result<Self, Self::Error> {
                use ::syan::parse::parse_stream::ParseStream;
                let __read_span: fn(&$atom) -> $span = $read;
                match ParseStream::next(__stream) {
                    ::core::option::Option::Some(__atom) => match &__atom.slot {
                        $crate::token::Token::$variant(__v) => {
                            let __v = __v.clone();
                            let __span = __read_span(&__atom);
                            ::core::result::Result::Ok($leaf {
                                $field: __v,
                                span: __span,
                            })
                        }
                        _ => {
                            let __span = __read_span(&__atom);
                            ParseStream::push(__stream, __atom);
                            ::core::result::Result::Err(::syan::error::ParseError::expected(__span, $expect))
                        }
                    },
                    ::core::option::Option::None => {
                        ::core::result::Result::Err(::syan::error::ParseError::eof(<$span as ::core::default::Default>::default()))
                    }
                }
            }
        }

        impl ::syan::parse::unparse::Unparse<$atom> for $leaf {
            fn unparse<__E: ::syan::parse::unparse::Emitter<$atom>>(
                &self,
                __sink: &mut __E,
            ) -> ::core::result::Result<(), __E::Error> {
                type __A = $atom;
                ::syan::parse::unparse::Emitter::write_one(
                    __sink,
                    __A {
                        slot: $crate::token::Token::$variant(self.$field.clone()),
                        span: self.span.clone(),
                    },
                )
            }
        }

        impl ::syan::span::Spanned for $leaf {
            type Span = $span;
            fn span(&self) -> Self::Span {
                self.span.clone()
            }
        }
    };
}

#[cfg(test)]
mod tests {
    //! Type-check the macro against the REAL `Atom`/`Token`, under throwaway
    //! leaf names so nothing in `leaf.rs` is shadowed. If this compiles, the
    //! generated impls line up with the same traits the derive satisfies.
    use crate::span::Span;
    use crate::token::Atom;

    token_leaves! {
        atom = Atom, span = Span, read_span = |a| a.span;
        (Let => ProbeKwLet, "'let'")
    }

    #[test]
    fn probe_leaf_parses_and_reports_its_span() {
        use syan::span::Spanned;
        let leaf = ProbeKwLet(Span::default());
        let _: Span = leaf.span();
    }
}
