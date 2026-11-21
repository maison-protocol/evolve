//! Lexer (RFC 0001 "Lexical structure").
//!
//! Two passes:
//! 1. [`raw_lex`] splits the source into tokens that cover every byte (lossless).
//! 2. [`layout`] turns physical line breaks into significant `NEWLINE`s or
//!    trivia `LINE_BREAK`s, and inserts zero-width `INDENT` / `DEDENT` tokens.

use crate::{SyntaxError, SyntaxKind, SyntaxKind::*};
use rowan::{TextRange, TextSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Token {
    pub kind: SyntaxKind,
    pub start: usize,
    pub end: usize,
}

impl Token {
    fn new(kind: SyntaxKind, start: usize, end: usize) -> Token {
        Token { kind, start, end }
    }

    pub fn is_virtual(&self) -> bool {
        self.start == self.end
    }
}

pub(crate) fn range(start: usize, end: usize) -> TextRange {
    TextRange::new(to_size(start), to_size(end))
}

fn to_size(offset: usize) -> TextSize {
    TextSize::try_from(offset).expect("source files are limited to 4 GiB")
}

pub(crate) fn lex(src: &str, errors: &mut Vec<SyntaxError>) -> Vec<Token> {
    let raw = raw_lex(src, errors);
    layout(src, &raw, errors)
}

// ---------------------------------------------------------------- pass 1 ----

const PUNCT: &[(&str, SyntaxKind)] = &[
    ("+/-", PLUS_MINUS),
    ("..=", DOT2EQ),
    ("...", DOT3),
    ("//=", SLASH2_EQ),
    ("..", DOT2),
    (".+", DOT_PLUS),
    (".-", DOT_MINUS),
    (".*", DOT_STAR),
    ("./", DOT_SLASH),
    (".\\", DOT_BACKSLASH),
    (".^", DOT_CARET),
    ("|>", PIPE_GT),
    ("->", ARROW),
    ("==", EQ2),
    ("!=", NEQ),
    ("<=", LTEQ),
    (">=", GTEQ),
    ("**", STAR2),
    ("//", SLASH2),
    ("+=", PLUS_EQ),
    ("-=", MINUS_EQ),
    ("*=", STAR_EQ),
    ("/=", SLASH_EQ),
    ("%=", PERCENT_EQ),
    ("^=", CARET_EQ),
    ("&&", AMP2),
    ("||", PIPE2),
    ("±", PLUS_MINUS),
    ("(", L_PAREN),
    (")", R_PAREN),
    ("[", L_BRACK),
    ("]", R_BRACK),
    ("{", L_BRACE),
    ("}", R_BRACE),
    (",", COMMA),
    (":", COLON),
    (";", SEMICOLON),
    (".", DOT),
    ("@", AT),
    ("~", TILDE),
    ("=", EQ),
    ("<", LT),
    (">", GT),
    ("+", PLUS),
    ("-", MINUS),
    ("*", STAR),
    ("/", SLASH),
    ("\\", BACKSLASH),
    ("%", PERCENT),
    ("^", CARET),
    ("|", PIPE),
    ("!", BANG),
];

const BOM: char = '\u{feff}';

fn is_superscript(c: char) -> bool {
    matches!(
        c,
        '⁰' | '¹' | '²' | '³' | '⁴' | '⁵' | '⁶' | '⁷' | '⁸' | '⁹' | '⁻'
    )
}

fn is_ident_start(c: char) -> bool {
    c == '_' || unicode_ident::is_xid_start(c)
}

fn is_ident_continue(c: char) -> bool {
    !is_superscript(c) && unicode_ident::is_xid_continue(c)
}

/// Characters that can make source display differently from how it runs ("Trojan Source").
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn take_while(s: &str, pred: impl Fn(char) -> bool) -> usize {
    s.char_indices()
        .find(|&(_, c)| !pred(c))
        .map_or(s.len(), |(i, _)| i)
}

fn char_len(s: &str) -> usize {
    s.chars().next().map_or(0, char::len_utf8)
}

/// The byte length of the escape body after a backslash (`s` starts just after it).
/// A CRLF line ending is one newline, so `\` before it escapes both bytes.
fn escape_len(s: &str) -> usize {
    if s.starts_with("\r\n") {
        2
    } else {
        char_len(s)
    }
}

/// How a string literal's body is checked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StrMode {
    /// Plain string: escapes are checked (RFC 0001 "Strings").
    Plain,
    /// `b"…"`: escapes are checked, and the body must be ASCII.
    Bytes,
    /// `r"…"` and user string macros: the macro receives the raw text.
    Raw,
}

pub(crate) fn raw_lex(src: &str, errors: &mut Vec<SyntaxError>) -> Vec<Token> {
    for (i, c) in src.char_indices().filter(|&(_, c)| is_bidi_control(c)) {
        errors.push(SyntaxError::new(
            format!(
                "bidirectional control character U+{:04X} is not allowed in source, even in strings and comments; inside a string write it as `\\u{:04x}`",
                u32::from(c),
                u32::from(c)
            ),
            range(i, i + c.len_utf8()),
        ));
    }

    for (i, _) in src.match_indices('\0') {
        errors.push(SyntaxError::new(
            "the NUL character is not allowed in source, even in strings and comments; inside a string write it as `\\0`",
            range(i, i + 1),
        ));
    }

    let mut tokens: Vec<Token> = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        let c = rest.chars().next().expect("non-empty");
        let (kind, len) = match c {
            '\n' => (NEWLINE, 1),
            '\r' if rest.starts_with("\r\n") => (NEWLINE, 2),
            '\r' => {
                errors.push(SyntaxError::new(
                    "a lone carriage return (CR) is not a line ending; use LF or CRLF",
                    range(i, i + 1),
                ));
                (NEWLINE, 1)
            }
            BOM if i == 0 => (WHITESPACE, BOM.len_utf8()),
            BOM => {
                errors.push(SyntaxError::new(
                    "a byte-order mark is only allowed at the start of the file",
                    range(i, i + BOM.len_utf8()),
                ));
                (ERROR_TOKEN, BOM.len_utf8())
            }
            ' ' | '\t' | '\x0c' => (
                WHITESPACE,
                take_while(rest, |c| matches!(c, ' ' | '\t' | '\x0c')),
            ),
            '#' => (COMMENT, take_while(rest, |c| c != '\n' && c != '\r')),
            '"' | '\'' => {
                let mode = match tokens.last() {
                    Some(t) if t.kind == IDENT && t.end == i => {
                        if matches!(&src[t.start..t.end], "b" | "B") {
                            StrMode::Bytes
                        } else {
                            StrMode::Raw
                        }
                    }
                    _ => StrMode::Plain,
                };
                (STRING, lex_string(rest, i, mode, errors))
            }
            '.' if rest[1..].starts_with(|c: char| c.is_ascii_digit()) => {
                let len = 1 + lex_number(&rest[1..], i + 1, errors);
                errors.push(SyntaxError::new(
                    format!("write `0{}`, not `{}`", &rest[..len], &rest[..len]),
                    range(i, i + len),
                ));
                (NUMBER, len)
            }
            c if c.is_ascii_digit() => {
                let len = lex_number(rest, i, errors);
                let word = take_while(&rest[len..], is_ident_continue);
                if word > 0 {
                    let (num, name) = (&rest[..len], &rest[len..len + word]);
                    errors.push(SyntaxError::new(
                        format!(
                            "a number can't be directly followed by a letter: write `{num} {name}` for a unit, or `{num} * {name}` to multiply"
                        ),
                        range(i, i + len + word),
                    ));
                }
                (NUMBER, len)
            }
            c if is_superscript(c) => (SUPERSCRIPT, take_while(rest, is_superscript)),
            c if is_ident_start(c) => {
                let len = c.len_utf8() + take_while(&rest[c.len_utf8()..], is_ident_continue);
                let word = &rest[..len];
                let after = rest[len..].chars().next();
                if matches!(word, "f" | "F" | "t" | "T") && matches!(after, Some('"' | '\'')) {
                    (FSTRING, len + lex_fstring(&rest[len..], i + len, errors))
                } else {
                    (SyntaxKind::from_keyword(word).unwrap_or(IDENT), len)
                }
            }
            _ => match PUNCT.iter().find(|(p, _)| rest.starts_with(p)) {
                Some(&(p, kind)) => (kind, p.len()),
                // already reported above; lexed as whitespace so parsing goes on undisturbed
                None if is_bidi_control(c) || c == '\0' => (WHITESPACE, c.len_utf8()),
                None => {
                    errors.push(SyntaxError::new(
                        format!("unexpected character {c:?}"),
                        range(i, i + c.len_utf8()),
                    ));
                    (ERROR_TOKEN, c.len_utf8())
                }
            },
        };
        tokens.push(Token::new(kind, i, i + len));
        i += len;
    }
    tokens
}

/// The opening delimiter of the string at the start of `s` and whether it is triple-quoted.
fn delimiter(s: &str) -> (&str, bool) {
    // Compare bytes, not a 3-byte slice: the body may start with a multi-byte character.
    let quote = s.as_bytes()[0];
    if s.as_bytes().get(..3) == Some(&[quote; 3]) {
        (&s[..3], true)
    } else {
        (&s[..1], false)
    }
}

fn unterminated(s: &str, offset: usize, end: usize, errors: &mut Vec<SyntaxError>) -> usize {
    let end = end.min(s.len());
    errors.push(SyntaxError::new(
        "unterminated string literal",
        range(offset, offset + end),
    ));
    end
}

/// Returns the byte length of the string literal at the start of `s`.
fn lex_string(s: &str, offset: usize, mode: StrMode, errors: &mut Vec<SyntaxError>) -> usize {
    let (delim, multiline) = delimiter(s);
    let mut i = delim.len();
    while i < s.len() {
        let rest = &s[i..];
        if let Some(escaped) = rest.strip_prefix('\\') {
            if mode != StrMode::Raw {
                check_escape(escaped, offset + i, mode, errors);
            }
            i += 1 + escape_len(escaped);
        } else if rest.starts_with(delim) {
            return i + delim.len();
        } else if !multiline && (rest.starts_with('\n') || rest.starts_with('\r')) {
            break;
        } else {
            let c = rest.chars().next().expect("non-empty");
            if mode == StrMode::Bytes && !c.is_ascii() {
                errors.push(SyntaxError::new(
                    "bytes literals are ASCII only; write other bytes as `\\xhh`",
                    range(offset + i, offset + i + c.len_utf8()),
                ));
            }
            i += c.len_utf8();
        }
    }
    unterminated(s, offset, i, errors)
}

/// Returns the byte length of the f- or t-string body at the start of `s` (after the
/// prefix). Replacement fields may nest strings that reuse the outer quote (PEP 701).
/// In a single-quoted f-string a replacement field stays on one line.
fn lex_fstring(s: &str, offset: usize, errors: &mut Vec<SyntaxError>) -> usize {
    let (delim, multiline) = delimiter(s);
    let mut i = delim.len();
    let mut depth = 0usize;
    while i < s.len() {
        let rest = &s[i..];
        let c = rest.chars().next().expect("non-empty");
        if !multiline && (c == '\n' || c == '\r') {
            break;
        }
        if depth == 0 {
            if let Some(escaped) = rest.strip_prefix('\\') {
                check_escape(escaped, offset + i, StrMode::Plain, errors);
                i += 1 + escape_len(escaped);
            } else if rest.starts_with(delim) {
                return i + delim.len();
            } else if rest.starts_with("{{") || rest.starts_with("}}") {
                i += 2;
            } else {
                depth += usize::from(c == '{');
                i += c.len_utf8();
            }
            continue;
        }
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            '\'' | '"' => {
                let prefixed = s[..i].ends_with(['f', 'F', 't', 'T']);
                i += if prefixed {
                    lex_fstring(rest, offset + i, errors)
                } else {
                    lex_string(rest, offset + i, StrMode::Plain, errors)
                };
                continue;
            }
            '#' => {
                i += take_while(rest, |c| c != '\n' && c != '\r');
                continue;
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    unterminated(s, offset, i, errors)
}

/// True when `s` starts with `{name}`. Unicode character names are made of ASCII
/// letters, digits, spaces, and hyphens; whether the name exists is checked later.
fn has_char_name(s: &str) -> bool {
    s.strip_prefix('{').is_some_and(|rest| {
        let name = take_while(rest, |c| c.is_ascii_alphanumeric() || c == ' ' || c == '-');
        name > 0 && rest[name..].starts_with('}')
    })
}

/// Checks the escape after a backslash (`s` starts just after it).
fn check_escape(s: &str, offset: usize, mode: StrMode, errors: &mut Vec<SyntaxError>) {
    let hex = |n: usize| {
        s[1..]
            .chars()
            .take(n)
            .filter(char::is_ascii_hexdigit)
            .count()
            == n
    };
    let Some(c) = s.chars().next() else { return };
    let problem = match c {
        '\n' | '\r' | '\\' | '\'' | '"' | 'n' | 'r' | 't' | '0' => None,
        'x' if hex(2) => None,
        'x' => Some("`\\x` needs exactly 2 hex digits".to_string()),
        'u' | 'U' | 'N' if mode == StrMode::Bytes => {
            Some(format!("`\\{c}` is not allowed in a bytes literal"))
        }
        'u' if hex(4) => None,
        'u' => Some("`\\u` needs exactly 4 hex digits".to_string()),
        'U' if hex(8) => None,
        'U' => Some("`\\U` needs exactly 8 hex digits".to_string()),
        'N' if has_char_name(&s[1..]) => None,
        'N' => Some("`\\N` needs a character name: `\\N{name}`".to_string()),
        c => Some(format!(
            "unknown escape `\\{c}`: write `\\\\{c}` for a backslash, or use a raw string `r\"…\"`"
        )),
    };
    if let Some(message) = problem {
        errors.push(SyntaxError::new(
            message,
            range(offset, offset + 1 + c.len_utf8()),
        ));
    }
}

/// Returns the byte length of the number literal at the start of `s`, and reports
/// misplaced `_`, leading zeros, and `5.` (RFC 0001 "Numbers").
fn lex_number(s: &str, offset: usize, errors: &mut Vec<SyntaxError>) -> usize {
    let digits = |s: &str, radix: u32| take_while(s, |c| c == '_' || c.is_digit(radix));
    let mut segments: Vec<(usize, usize)> = Vec::new();
    let mut end = None;
    for (prefix, radix, digit, digit_set, example) in [
        ("0x", 16, "a hex", "", "0xFF"),
        ("0X", 16, "a hex", "", "0xFF"),
        ("0b", 2, "a binary", "binary uses `0` and `1`", "0b1010"),
        ("0B", 2, "a binary", "binary uses `0` and `1`", "0b1010"),
        ("0o", 8, "an octal", "octal uses `0` to `7`", "0o17"),
        ("0O", 8, "an octal", "octal uses `0` to `7`", "0o17"),
    ] {
        if s.starts_with(prefix) {
            let n = digits(&s[2..], radix);
            // Decimal digits the radix doesn't have (`0b102`, `0o78`) stay in the token,
            // so they get one precise error instead of the number-then-letter hint.
            let stray = take_while(&s[2 + n..], |c| c == '_' || c.is_ascii_digit());
            let len = 2 + n + stray;
            if stray > 0 {
                let bad = &s[2 + n..2 + n + 1];
                errors.push(SyntaxError::new(
                    format!("`{bad}` is not {digit} digit: {digit_set}"),
                    range(offset + 2 + n, offset + 2 + n + 1),
                ));
            } else if !s[2..len].contains(|c: char| c.is_digit(radix)) {
                errors.push(SyntaxError::new(
                    format!("`{prefix}` must be followed by {digit} digit, as in `{example}`"),
                    range(offset, offset + len),
                ));
            }
            // one `_` may follow the prefix, as in Python: 0x_FF
            let first = 2 + usize::from(s[2..].starts_with('_'));
            segments.push((first, len));
            end = Some(len);
            break;
        }
    }
    let i = match end {
        Some(i) => i,
        None => {
            let int_end = digits(s, 10);
            segments.push((0, int_end));
            let mut i = int_end;
            let mut float = false;
            let after_dot = &s[i..].get(1..).unwrap_or("");
            if s[i..].starts_with('.') && after_dot.starts_with(|c: char| c.is_ascii_digit()) {
                // fraction: "1.5" but not "1..5" (range) or "1.method"
                let n = digits(after_dot, 10);
                segments.push((i + 1, i + 1 + n));
                i += 1 + n;
                float = true;
            } else if s[i..].starts_with('.')
                && (after_dot.is_empty()
                    || after_dot.starts_with(|c: char| {
                        c.is_whitespace() || matches!(c, ')' | ']' | '}' | ',' | ':' | '#')
                    }))
            {
                errors.push(SyntaxError::new(
                    format!("write `{}.0`, not `{}.`", &s[..i], &s[..i]),
                    range(offset, offset + i + 1),
                ));
                i += 1;
                float = true;
            }
            // exponent: "1e-5", "2.5E3"
            let rest = &s[i..];
            if rest.starts_with(['e', 'E']) {
                let sign = usize::from(rest[1..].starts_with(['+', '-']));
                if rest[1 + sign..].starts_with(|c: char| c.is_ascii_digit()) {
                    let n = digits(&rest[1 + sign..], 10);
                    segments.push((i + 1 + sign, i + 1 + sign + n));
                    i += 1 + sign + n;
                    float = true;
                }
            }
            let int = &s[..int_end];
            if !float
                && int.len() > 1
                && int.starts_with('0')
                && int.contains(['1', '2', '3', '4', '5', '6', '7', '8', '9'])
            {
                let bare = int.trim_start_matches(['0', '_']);
                errors.push(SyntaxError::new(
                    format!("leading zeros are not allowed: write `0o{bare}` for octal or `{bare}` for decimal"),
                    range(offset, offset + int_end),
                ));
            }
            i
        }
    };
    for (a, b) in segments {
        let seg = &s[a..b];
        if seg.starts_with('_') || seg.ends_with('_') || seg.contains("__") {
            errors.push(SyntaxError::new(
                "`_` in a number must sit between two digits",
                range(offset, offset + i),
            ));
            break;
        }
    }
    i
}

// ---------------------------------------------------------------- pass 2 ----

/// A line ending in one of these continues onto the next line (RFC 0001 "Line joining").
fn continues_line(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        PLUS | MINUS
            | STAR
            | SLASH
            | SLASH2
            | BACKSLASH
            | PERCENT
            | CARET
            | PIPE
            | PIPE_GT
            | PLUS_MINUS
            | DOT_PLUS
            | DOT_MINUS
            | DOT_STAR
            | DOT_SLASH
            | DOT_BACKSLASH
            | DOT_CARET
            | AND_KW
            | OR_KW
            | EQ2
            | NEQ
            | LT
            | LTEQ
            | GT
            | GTEQ
    )
}

/// True when the next line holding a real token starts with `|>`.
fn next_line_starts_with_pipe(raw: &[Token], from: usize) -> bool {
    raw[from..]
        .iter()
        .find(|t| !matches!(t.kind, WHITESPACE | COMMENT | NEWLINE))
        .is_some_and(|t| t.kind == PIPE_GT)
}

pub(crate) fn layout(src: &str, raw: &[Token], errors: &mut Vec<SyntaxError>) -> Vec<Token> {
    let mut out = Vec::with_capacity(raw.len() + 16);
    let mut indents = vec![0usize];
    let mut depth = 0usize;
    let mut at_logical_line_start = true;
    let mut line_has_tokens = false;
    let mut at_physical_line_start = true;
    let mut line_indent = 0usize;
    let mut prev_significant: Option<SyntaxKind> = None;

    for (idx, &tok) in raw.iter().enumerate() {
        match tok.kind {
            WHITESPACE => {
                if at_physical_line_start {
                    let text = &src[tok.start..tok.end];
                    if text.contains('\t') {
                        errors.push(SyntaxError::new(
                            "tabs are not allowed in indentation",
                            range(tok.start, tok.end),
                        ));
                    }
                    line_indent += text.chars().filter(|&c| c != BOM).count();
                }
                out.push(tok);
            }
            COMMENT => out.push(tok),
            NEWLINE => {
                let significant = depth == 0
                    && line_has_tokens
                    && !prev_significant.is_some_and(continues_line)
                    && !next_line_starts_with_pipe(raw, idx + 1);
                if significant {
                    out.push(tok);
                    line_has_tokens = false;
                    at_logical_line_start = true;
                } else {
                    out.push(Token {
                        kind: LINE_BREAK,
                        ..tok
                    });
                }
                at_physical_line_start = true;
                line_indent = 0;
            }
            kind => {
                if at_logical_line_start {
                    indent_to(line_indent, tok.start, &mut indents, &mut out, errors);
                    at_logical_line_start = false;
                }
                match kind {
                    L_PAREN | L_BRACK | L_BRACE => depth += 1,
                    R_PAREN | R_BRACK | R_BRACE => depth = depth.saturating_sub(1),
                    _ => {}
                }
                line_has_tokens = true;
                at_physical_line_start = false;
                prev_significant = Some(kind);
                out.push(tok);
            }
        }
    }

    let end = src.len();
    if line_has_tokens {
        out.push(Token::new(NEWLINE, end, end));
    }
    while indents.len() > 1 {
        indents.pop();
        out.push(Token::new(DEDENT, end, end));
    }
    out
}

fn indent_to(
    width: usize,
    at: usize,
    indents: &mut Vec<usize>,
    out: &mut Vec<Token>,
    errors: &mut Vec<SyntaxError>,
) {
    let top = *indents.last().expect("indent stack never empty");
    if width > top {
        indents.push(width);
        out.push(Token::new(INDENT, at, at));
        return;
    }
    while width < *indents.last().expect("indent stack never empty") {
        indents.pop();
        out.push(Token::new(DEDENT, at, at));
    }
    if width != *indents.last().expect("indent stack never empty") {
        errors.push(SyntaxError::new(
            "unindent does not match any outer indentation level",
            range(at, at),
        ));
    }
}
