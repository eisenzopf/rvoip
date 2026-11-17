---
section: Breaking changes
breaking: true
---
**nom 7 → 8 in rvoip-sip-core.** The public parser functions (about 220
returning `nom::IResult` / `ParseResult`) keep their signatures, but the
`nom::IResult`, `nom::Err` and `nom::error::Error` types in them are now
nom 8's. So are the `rvoip_sip_core::parser::{NomError, ErrorKind,
ParseError}` re-exports, the `ParseResult` aliases, and the `From` impls on
`rvoip_sip_core::Error` for `nom::Err<…>` and `NomError<&[u8]>`.
`comma_separated_list0`, `comma_separated_list1` and
`semicolon_separated_params0` still return `impl FnMut`. Migration: callers
that compose sip-core parsers with their own nom combinators must use nom 8
and call `.parse(input)` (with `nom::Parser` in scope) instead of
`combinator(…)(input)`; callers that only call sip-core parsers need no
change.
