use nom::{
    bytes::complete::take_while1,
    IResult,
};

// Type alias for parser result
pub(crate) type ParseResult<'a, O> = IResult<&'a [u8], O>;

fn is_token_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || 
    c == b'-' || c == b'.' || c == b'!' || c == b'%' || c == b'*' || 
    c == b'_' || c == b'+' || c == b'`' || c == b'\'' || c == b'~'
}

pub(crate) fn token(input: &[u8]) -> ParseResult<&[u8]> {
    // token = 1*(alphanum / "-" / "." / "!" / "%" / "*" / "_" / "+" / "`" / "'" / "~")
    take_while1(is_token_char)(input)
}

fn is_word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || 
    c == b'-' || c == b'.' || c == b'!' || c == b'%' || c == b'*' || 
    c == b'_' || c == b'+' || c == b'`' || c == b'\'' || c == b'~' || 
    c == b'(' || c == b')' || c == b'<' || c == b'>' || c == b':' || 
    c == b'\\' || c == b'"' || c == b'/' || c == b'[' || c == b']' || 
    c == b'?' || c == b'{' || c == b'}'
}

pub(crate) fn word(input: &[u8]) -> ParseResult<&[u8]> {
    take_while1(is_word_char)(input)
} 