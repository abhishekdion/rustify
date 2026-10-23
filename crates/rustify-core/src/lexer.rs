//! Python tokenizer: converts source text into a token stream with
//! NEWLINE / INDENT / DEDENT markers (CPython's logical-line model).

use crate::ast::StrKind;

#[derive(Debug, Clone, PartialEq)]
pub enum FSegRaw {
    Text(String),
    /// Raw source of the embedded expression, plus conversion and format spec.
    Expr {
        src: String,
        conversion: Option<char>,
        format_spec: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Newline,
    Indent,
    Dedent,
    Name(String),
    Int(i64),
    Float(f64),
    Str {
        value: String,
        kind: StrKind,
    },
    FStr {
        segments: Vec<FSegRaw>,
        kind: StrKind,
    },
    Op(&'static str),
    Eof,
}

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Newline => "NEWLINE".into(),
            Tok::Indent => "INDENT".into(),
            Tok::Dedent => "DEDENT".into(),
            Tok::Name(n) => format!("NAME({})", n),
            Tok::Int(v) => format!("INT({})", v),
            Tok::Float(v) => format!("FLOAT({})", v),
            Tok::Str { .. } => "STRING".into(),
            Tok::FStr { .. } => "FSTRING".into(),
            Tok::Op(o) => format!("OP({})", o),
            Tok::Eof => "EOF".into(),
        }
    }
}

#[derive(Debug)]
pub struct LexError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}:{}: {}", self.line, self.col, self.msg)
    }
}

/// Operators, longest-first so that greedy matching works.
const OPERATORS: &[&str] = &[
    "**=", "//=", ">>=", "<<=", "...", "!=", "==", "<=", ">=", "->", ":=", "**", "//", "<<", ">>", "+=",
    "-=", "*=", "/=", "%=", "&=", "|=", "^=", "@=", "+", "-", "*", "/", "%", "@", "&", "|", "^", "~",
    "<", ">", "(", ")", "[", "]", "{", "}", ",", ":", ".", ";", "=", "!", "^",
];

pub struct Lexer {
    src: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    indents: Vec<usize>,
    paren_depth: usize,
    at_line_start: bool,
    /// Reserved for multi-level dedents at a single NEWLINE token.
    #[allow(dead_code)]
    pending_dedents: usize,
    /// True once the synthetic NEWLINE at end of file has been emitted.
    emitted_eof_newline: bool,
    /// Source line for each emitted token (parallel to the token stream).
    pub lines: Vec<usize>,
}

impl Lexer {
    pub fn new(src: &str) -> Self {
        Lexer {
            src: src.chars().collect(),
            pos: 0,
            line: 1,
            col: 0,
            indents: vec![0],
            paren_depth: 0,
            at_line_start: true,
            pending_dedents: 0,
            emitted_eof_newline: false,
            lines: Vec::new(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.src.get(self.pos + n).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.src.get(self.pos).copied();
        if let Some(ch) = c {
            self.pos += 1;
            if ch == '\n' {
                self.line += 1;
                self.col = 0;
            } else {
                self.col += 1;
            }
        }
        c
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T, LexError> {
        Err(LexError {
            line: self.line,
            col: self.col,
            msg: msg.into(),
        })
    }

    pub fn tokenize(mut self) -> Result<(Vec<Tok>, Vec<usize>), LexError> {
        let mut out: Vec<Tok> = Vec::new();
        loop {
            if self.at_line_start && self.paren_depth == 0 {
                self.handle_line_start(&mut out)?;
                if self.at_line_start {
                    // EOF reached while scanning indentation.
                    break;
                }
            }
            match self.next_token()? {
                Some(t) => {
                    let is_newline = t == Tok::Newline;
                    let line = self.line;
                    out.push(t);
                    self.lines.push(line);
                    if is_newline {
                        continue;
                    }
                }
                None => break,
            }
        }
        // Close out remaining indentation levels.
        while self.indents.len() > 1 {
            self.indents.pop();
            out.push(Tok::Dedent);
            self.lines.push(self.line);
        }
        out.push(Tok::Eof);
        self.lines.push(self.line);
        Ok((out, self.lines))
    }

    /// Measure indentation of the upcoming logical line. Blank and
    /// comment-only lines are skipped entirely.
    fn handle_line_start(&mut self, out: &mut Vec<Tok>) -> Result<(), LexError> {
        loop {
            let mut width = 0usize;
            let start = self.pos;
            while let Some(c) = self.peek() {
                match c {
                    ' ' => {
                        width += 1;
                        self.bump();
                    }
                    '\t' => {
                        // Python 3 tab stop of 8
                        width = (width / 8 + 1) * 8;
                        self.bump();
                    }
                    '\x0c' => {
                        width = 0;
                        self.bump();
                    }
                    _ => break,
                }
            }
            match self.peek() {
                None => {
                    self.at_line_start = false;
                    return Ok(());
                }
                Some('\n') => {
                    // Blank line: no NEWLINE token, no indent processing.
                    self.bump();
                    continue;
                }
                Some('#') => {
                    // Comment-only line.
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                    if self.peek() == Some('\n') {
                        self.bump();
                    }
                    continue;
                }
                _ => {}
            }
            let _ = start;
            let cur = *self.indents.last().unwrap();
            if width > cur {
                self.indents.push(width);
                out.push(Tok::Indent);
                self.lines.push(self.line);
            } else if width < cur {
                while *self.indents.last().unwrap() > width {
                    self.indents.pop();
                    out.push(Tok::Dedent);
                    self.lines.push(self.line);
                }
                if *self.indents.last().unwrap() != width {
                    return self.err("inconsistent dedent");
                }
            }
            self.at_line_start = false;
            return Ok(());
        }
    }

    fn next_token(&mut self) -> Result<Option<Tok>, LexError> {
        loop {
            // Skip horizontal whitespace and intra-line comments.
            loop {
                match self.peek() {
                    Some(' ') | Some('\t') | Some('\x0c') => {
                        self.bump();
                    }
                    Some('#') => {
                        while let Some(c) = self.peek() {
                            if c == '\n' {
                                break;
                            }
                            self.bump();
                        }
                    }
                    _ => break,
                }
            }
            let c = match self.peek() {
                None => {
                    // End of file: if the last line had content, emit NEWLINE
                    // exactly once.
                    if !self.emitted_eof_newline {
                        self.emitted_eof_newline = true;
                        if self.src.last().map_or(false, |c| *c != '\n') {
                            return Ok(Some(Tok::Newline));
                        }
                    }
                    return Ok(None);
                }
                Some(c) => c,
            };

            // Explicit line continuation.
            if c == '\\' {
                if self.peek_at(1) == Some('\n') {
                    self.bump();
                    self.bump();
                    continue;
                }
                if self.peek_at(1) == Some('\r') && self.peek_at(2) == Some('\n') {
                    self.bump();
                    self.bump();
                    self.bump();
                    continue;
                }
                return self.err("unexpected '\\'");
            }

            if c == '\n' {
                self.bump();
                if self.paren_depth > 0 {
                    continue;
                }
                self.at_line_start = true;
                return Ok(Some(Tok::Newline));
            }
            if c == '\r' {
                self.bump();
                continue;
            }

            // String literals (possibly with a prefix).
            if c == '"' || c == '\'' {
                if let Some(t) = self.try_string()? {
                    return Ok(Some(t));
                }
            }
            if c.is_alphabetic() || c == '_' {
                if let Some(t) = self.try_string()? {
                    return Ok(Some(t));
                }
            }

            // Numbers.
            if c.is_ascii_digit()
                || (c == '.' && self.peek_at(1).map_or(false, |d| d.is_ascii_digit()))
            {
                return Ok(Some(self.lex_number()?));
            }

            // Identifiers / keywords.
            if c.is_alphabetic() || c == '_' {
                let mut name = String::new();
                while let Some(ch) = self.peek() {
                    if ch.is_alphanumeric() || ch == '_' {
                        name.push(ch);
                        self.bump();
                    } else {
                        break;
                    }
                }
                return Ok(Some(Tok::Name(name)));
            }

            // Operators.
            for op in OPERATORS {
                let chars: Vec<char> = op.chars().collect();
                if chars.iter().enumerate().all(|(i, &cc)| self.peek_at(i) == Some(cc)) {
                    for _ in 0..chars.len() {
                        self.bump();
                    }
                    match *op {
                        "(" | "[" | "{" => self.paren_depth += 1,
                        ")" | "]" | "}" => self.paren_depth = self.paren_depth.saturating_sub(1),
                        _ => {}
                    }
                    return Ok(Some(Tok::Op(op)));
                }
            }

            return self.err(format!("unexpected character {:?}", c));
        }
    }

    /// Detect a string prefix (r/b/f/u combinations) followed by a quote.
    fn try_string(&mut self) -> Result<Option<Tok>, LexError> {
        let save = (self.pos, self.line, self.col);
        let mut prefix = String::new();
        while let Some(c) = self.peek() {
            if matches!(c, 'r' | 'R' | 'b' | 'B' | 'f' | 'F' | 'u' | 'U') {
                prefix.push(c.to_ascii_lowercase());
                self.bump();
            } else {
                break;
            }
        }
        if prefix.len() > 2 || !matches!(self.peek(), Some('"') | Some('\'')) {
            // Not a string: rewind and treat as a normal identifier.
            self.pos = save.0;
            self.line = save.1;
            self.col = save.2;
            return Ok(None);
        }
        if prefix.contains('u') && (prefix.contains('r') || prefix.contains('b') || prefix.contains('f')) {
            self.pos = save.0;
            self.line = save.1;
            self.col = save.2;
            return Ok(None);
        }
        let raw = prefix.contains('r');
        let bytes = prefix.contains('b');
        let fstr = prefix.contains('f');

        let (body, _triple) = self.read_string_body()?;
        if fstr {
            let kind = if bytes { StrKind::Bytes } else { StrKind::Str };
            let segments = split_fstring(&body)?;
            return Ok(Some(Tok::FStr { segments, kind }));
        }
        let value = if raw {
            body
        } else {
            decode_escapes(&body, bytes)?
        };
        let kind = if bytes { StrKind::Bytes } else { StrKind::Str };
        Ok(Some(Tok::Str { value, kind }))
    }

    /// Read a string literal body (without quotes), handling escapes
    /// conservatively (the raw text is kept for raw strings).
    fn read_string_body(&mut self) -> Result<(String, bool), LexError> {
        let quote = self.bump().unwrap();
        let triple = self.peek() == Some(quote) && self.peek_at(1) == Some(quote);
        if triple {
            self.bump();
            self.bump();
        }
        let mut body = String::new();
        loop {
            let c = match self.peek() {
                None => return self.err("unterminated string literal"),
                Some(c) => c,
            };
            if c == '\\' {
                // Keep the escape sequence verbatim; decoding happens later.
                body.push(c);
                self.bump();
                if let Some(n) = self.bump() {
                    body.push(n);
                }
                continue;
            }
            if c == quote {
                if triple {
                    if self.peek_at(1) == Some(quote) && self.peek_at(2) == Some(quote) {
                        self.bump();
                        self.bump();
                        self.bump();
                        return Ok((body, true));
                    }
                } else {
                    self.bump();
                    return Ok((body, false));
                }
            }
            if c == '\n' && !triple {
                return self.err("unterminated string literal");
            }
            body.push(c);
            self.bump();
        }
    }

    fn lex_number(&mut self) -> Result<Tok, LexError> {
        let start = self.pos;
        // Hex / oct / bin
        if self.peek() == Some('0') {
            if let Some(p) = self.peek_at(1) {
                if matches!(p, 'x' | 'X' | 'o' | 'O' | 'b' | 'B') {
                    self.bump();
                    self.bump();
                    let mut digits = String::new();
                    while let Some(c) = self.peek() {
                        if c.is_ascii_alphanumeric() || c == '_' {
                            if c != '_' {
                                digits.push(c);
                            }
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    let radix = match p.to_ascii_lowercase() {
                        'x' => 16,
                        'o' => 8,
                        _ => 2,
                    };
                    return match i64::from_str_radix(&digits, radix) {
                        Ok(v) => Ok(Tok::Int(v)),
                        Err(_) => self.err("integer literal out of range for i64"),
                    };
                }
            }
        }
        let mut text = String::new();
        let mut is_float = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                text.push(c);
                self.bump();
            } else if c == '_' {
                self.bump();
            } else {
                break;
            }
        }
        if self.peek() == Some('.') {
            is_float = true;
            text.push('.');
            self.bump();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    text.push(c);
                    self.bump();
                } else if c == '_' {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            let save = self.pos;
            let mut exp = String::from("e");
            self.bump();
            if matches!(self.peek(), Some('+') | Some('-')) {
                exp.push(self.peek().unwrap());
                self.bump();
            }
            let mut any = false;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    exp.push(c);
                    any = true;
                    self.bump();
                } else {
                    break;
                }
            }
            if any {
                is_float = true;
                text.push_str(&exp);
            } else {
                self.pos = save;
            }
        }
        if matches!(self.peek(), Some('j') | Some('J')) {
            return self.err("complex literals are not supported");
        }
        let _ = start;
        if is_float {
            match text.parse::<f64>() {
                Ok(v) => Ok(Tok::Float(v)),
                Err(_) => self.err("invalid float literal"),
            }
        } else {
            match text.parse::<i64>() {
                Ok(v) => Ok(Tok::Int(v)),
                Err(_) => self.err("integer literal out of range for i64"),
            }
        }
    }
}

/// Decode Python escape sequences in a string body.
pub fn decode_escapes(body: &str, bytes: bool) -> Result<String, LexError> {
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let e = match chars.next() {
            Some(e) => e,
            None => {
                out.push('\\');
                break;
            }
        };
        match e {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            'a' => out.push('\x07'),
            'b' => out.push('\x08'),
            'f' => out.push('\x0c'),
            'v' => out.push('\x0b'),
            '0'..='7' => {
                let mut val = e.to_digit(8).unwrap();
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            val = val * 8 + d;
                            chars.next();
                        }
                        None => break,
                    }
                }
                if bytes {
                    out.push((val & 0xFF) as u8 as char);
                } else {
                    out.push(char::from_u32(val).unwrap_or('\u{fffd}'));
                }
            }
            'x' => {
                let mut val = 0u32;
                for _ in 0..2 {
                    match chars.next().and_then(|c| c.to_digit(16)) {
                        Some(d) => val = val * 16 + d,
                        None => {
                            return Err(LexError {
                                line: 0,
                                col: 0,
                                msg: "invalid \\x escape".into(),
                            })
                        }
                    }
                }
                if bytes {
                    out.push((val & 0xFF) as u8 as char);
                } else {
                    out.push(char::from_u32(val).unwrap_or('\u{fffd}'));
                }
            }
            'u' | 'U' if !bytes => {
                let n = if e == 'u' { 4 } else { 8 };
                let mut val = 0u32;
                for _ in 0..n {
                    match chars.next().and_then(|c| c.to_digit(16)) {
                        Some(d) => val = val * 16 + d,
                        None => {
                            return Err(LexError {
                                line: 0,
                                col: 0,
                                msg: "invalid unicode escape".into(),
                            })
                        }
                    }
                }
                out.push(char::from_u32(val).unwrap_or('\u{fffd}'));
            }
            '\n' => {}
            other => {
                // Python keeps unknown escapes verbatim.
                out.push('\\');
                out.push(other);
            }
        }
    }
    Ok(out)
}

/// Read an f-string format spec starting at `start`.
///
/// Returns the spec text and the index just past the closing `}` of the
/// replacement field.
fn read_format_spec(chars: &[char], start: usize) -> (String, usize) {
    let mut spec = String::new();
    let mut depth = 0i32;
    let mut j = start;
    while j < chars.len() {
        let ch = chars[j];
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            if depth == 0 {
                break;
            }
            depth -= 1;
        }
        spec.push(ch);
        j += 1;
    }
    (spec, j + 1)
}

/// Split an f-string body into literal text and embedded expressions.
pub fn split_fstring(body: &str) -> Result<Vec<FSegRaw>, LexError> {
    let chars: Vec<char> = body.chars().collect();
    let mut segs = Vec::new();
    let mut text = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' {
            if chars.get(i + 1) == Some(&'{') {
                text.push('{');
                i += 2;
                continue;
            }
            if !text.is_empty() {
                segs.push(FSegRaw::Text(std::mem::take(&mut text)));
            }
            // Scan the expression, tracking nesting of (), [], {} and quotes.
            i += 1;
            let start = i;
            let mut depth = 0i32;
            let mut quote: Option<char> = None;
            let mut expr_end = None;
            while i < chars.len() {
                let ch = chars[i];
                if let Some(q) = quote {
                    if ch == '\\' {
                        i += 2;
                        continue;
                    }
                    if ch == q {
                        quote = None;
                    }
                    i += 1;
                    continue;
                }
                match ch {
                    '\'' | '"' => {
                        quote = Some(ch);
                        i += 1;
                    }
                    '(' | '[' | '{' => {
                        depth += 1;
                        i += 1;
                    }
                    ')' | ']' => {
                        depth -= 1;
                        i += 1;
                    }
                    ':' if depth == 0 => {
                        // Format-spec separator: the expression ends here.
                        expr_end = Some(i);
                        break;
                    }
                    '}' => {
                        if depth == 0 {
                            expr_end = Some(i);
                            break;
                        }
                        depth -= 1;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
            let end = match expr_end {
                Some(e) => e,
                None => {
                    return Err(LexError {
                        line: 0,
                        col: 0,
                        msg: "unterminated f-string expression".into(),
                    })
                }
            };
            let expr_src: String = chars[start..end].iter().collect();

            // `end` points at the terminator of the replacement field: either
            // the closing '}' or the ':' that introduces a format spec.
            let mut conversion = None;
            let mut format_spec = None;
            let mut pos = end;

            if chars.get(pos) == Some(&':') {
                // Format spec directly after the expression.
                pos += 1;
                let (spec, next) = read_format_spec(&chars, pos);
                format_spec = Some(spec);
                pos = next;
            } else {
                // Closing brace: an optional `!r` conversion and an optional
                // format spec may follow.
                pos += 1;
                if chars.get(pos) == Some(&'!') {
                    conversion = chars.get(pos + 1).copied();
                    pos += 2;
                }
                if chars.get(pos) == Some(&':') {
                    pos += 1;
                    let (spec, next) = read_format_spec(&chars, pos);
                    format_spec = Some(spec);
                    pos = next;
                }
            }
            i = pos;
            segs.push(FSegRaw::Expr {
                src: expr_src,
                conversion,
                format_spec,
            });
        } else if c == '}' {
            if chars.get(i + 1) == Some(&'}') {
                text.push('}');
                i += 2;
                continue;
            }
            return Err(LexError {
                line: 0,
                col: 0,
                msg: "single '}' in f-string".into(),
            });
        } else {
            text.push(c);
            i += 1;
        }
    }
    if !text.is_empty() {
        segs.push(FSegRaw::Text(text));
    }
    Ok(segs)
}
