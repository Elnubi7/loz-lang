use std::fmt;

use loz_ast::{Diagnostic, Span};

pub type LexResult<T> = Result<T, LexError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub diagnostic: Diagnostic,
}

impl LexError {
    fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            diagnostic: Diagnostic::error(message).with_span(span),
        }
    }

    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "lexer error: {}", self.diagnostic.message)
    }
}

impl std::error::Error for LexError {}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub span: Span,
}

impl Token {
    fn new(kind: TokenKind, lexeme: String, span: Span) -> Self {
        Self { kind, lexeme, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Module,
    Import,
    Const,
    Mut,
    Func,
    Async,
    Await,
    Tool,
    Agent,
    Task,
    Workflow,
    Step,
    Struct,
    Impl,
    If,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    Return,
    Ref,
    True,
    False,
    I8Type,
    I16Type,
    I32Type,
    I64Type,
    U8Type,
    U16Type,
    U32Type,
    U64Type,
    F32Type,
    F64Type,
    IntType,
    FloatType,
    BoolType,
    TextType,
    CharType,
    VoidType,
    Colon,
    Semicolon,
    Comma,
    Dot,
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Equal,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqualEqual,
    NotEqual,
    Greater,
    Less,
    GreaterEqual,
    LessEqual,
    Arrow,
    And,
    Or,
    Not,
    IntegerLiteral(i64),
    FloatLiteral(String),
    StringLiteral(String),
    Identifier(String),
    Eof,
}

pub struct Lexer<'a> {
    source: &'a str,
    file_path: Option<String>,
    chars: Vec<char>,
    byte_offsets: Vec<usize>,
    position: usize,
    byte_position: usize,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self::with_file_path(source, None)
    }

    pub fn with_file_path(source: &'a str, file_path: Option<String>) -> Self {
        let chars = source.chars().collect::<Vec<_>>();
        let mut byte_offsets = source
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        byte_offsets.push(source.len());

        Self {
            source,
            file_path,
            chars,
            byte_offsets,
            position: 0,
            byte_position: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn tokenize(&mut self) -> LexResult<Vec<Token>> {
        let mut tokens = Vec::new();

        while let Some(ch) = self.current() {
            match ch {
                ' ' | '\t' | '\r' | '\n' => {
                    self.advance();
                }
                '/' if self.peek() == Some('/') => {
                    self.skip_comment();
                }
                '0'..='9' => {
                    tokens.push(self.read_number()?);
                }
                '"' => {
                    tokens.push(self.read_string()?);
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    tokens.push(self.read_identifier_or_keyword());
                }
                ':' => tokens.push(self.single_char(TokenKind::Colon)),
                ';' => tokens.push(self.single_char(TokenKind::Semicolon)),
                ',' => tokens.push(self.single_char(TokenKind::Comma)),
                '.' => tokens.push(self.single_char(TokenKind::Dot)),
                '(' => tokens.push(self.single_char(TokenKind::LeftParen)),
                ')' => tokens.push(self.single_char(TokenKind::RightParen)),
                '{' => tokens.push(self.single_char(TokenKind::LeftBrace)),
                '}' => tokens.push(self.single_char(TokenKind::RightBrace)),
                '[' => tokens.push(self.single_char(TokenKind::LeftBracket)),
                ']' => tokens.push(self.single_char(TokenKind::RightBracket)),
                '=' => {
                    if self.peek() == Some('=') {
                        tokens.push(self.double_char(TokenKind::EqualEqual));
                    } else {
                        tokens.push(self.single_char(TokenKind::Equal));
                    }
                }
                '!' => {
                    if self.peek() == Some('=') {
                        tokens.push(self.double_char(TokenKind::NotEqual));
                    } else {
                        return Err(LexError::new(
                            "unexpected character '!'",
                            self.current_span(),
                        ));
                    }
                }
                '>' => {
                    if self.peek() == Some('=') {
                        tokens.push(self.double_char(TokenKind::GreaterEqual));
                    } else {
                        tokens.push(self.single_char(TokenKind::Greater));
                    }
                }
                '<' => {
                    if self.peek() == Some('=') {
                        tokens.push(self.double_char(TokenKind::LessEqual));
                    } else {
                        tokens.push(self.single_char(TokenKind::Less));
                    }
                }
                '-' => {
                    if self.peek() == Some('>') {
                        tokens.push(self.double_char(TokenKind::Arrow));
                    } else {
                        tokens.push(self.single_char(TokenKind::Minus));
                    }
                }
                '+' => tokens.push(self.single_char(TokenKind::Plus)),
                '*' => tokens.push(self.single_char(TokenKind::Star)),
                '/' => tokens.push(self.single_char(TokenKind::Slash)),
                '%' => tokens.push(self.single_char(TokenKind::Percent)),
                _ => {
                    return Err(LexError::new(
                        format!("unexpected character '{}'", ch),
                        self.current_span(),
                    ));
                }
            }
        }

        let eof_span = Span::new(
            self.file_path.clone(),
            self.byte_position,
            self.byte_position,
            self.line,
            self.column,
        );
        tokens.push(Token::new(TokenKind::Eof, String::new(), eof_span));
        Ok(tokens)
    }

    fn current(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.position + 1).copied()
    }

    fn current_span(&self) -> Span {
        Span::new(
            self.file_path.clone(),
            self.byte_position,
            self.byte_offsets
                .get(self.position + 1)
                .copied()
                .unwrap_or(self.byte_position),
            self.line,
            self.column,
        )
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.current()?;
        self.position += 1;
        self.byte_position = self
            .byte_offsets
            .get(self.position)
            .copied()
            .unwrap_or(self.source.len());

        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }

        Some(ch)
    }

    fn skip_comment(&mut self) {
        while let Some(ch) = self.current() {
            self.advance();

            if ch == '\n' {
                break;
            }
        }
    }

    fn single_char(&mut self, kind: TokenKind) -> Token {
        let start = self.position;
        let start_byte = self.byte_position;
        let line = self.line;
        let column = self.column;
        self.advance();
        self.token_from_range(kind, start, self.position, start_byte, line, column)
    }

    fn double_char(&mut self, kind: TokenKind) -> Token {
        let start = self.position;
        let start_byte = self.byte_position;
        let line = self.line;
        let column = self.column;
        self.advance();
        self.advance();
        self.token_from_range(kind, start, self.position, start_byte, line, column)
    }

    fn read_number(&mut self) -> LexResult<Token> {
        let start = self.position;
        let start_byte = self.byte_position;
        let line = self.line;
        let column = self.column;

        while matches!(self.current(), Some('0'..='9')) {
            self.advance();
        }

        let is_float = self.current() == Some('.') && matches!(self.peek(), Some('0'..='9'));

        let kind = if is_float {
            self.advance();

            while matches!(self.current(), Some('0'..='9')) {
                self.advance();
            }

            TokenKind::FloatLiteral(self.lexeme(start, self.position))
        } else {
            let value = self
                .lexeme(start, self.position)
                .parse::<i64>()
                .map_err(|_| {
                    LexError::new(
                        "invalid integer literal",
                        Span::new(
                            self.file_path.clone(),
                            start_byte,
                            self.byte_position,
                            line,
                            column,
                        ),
                    )
                })?;
            TokenKind::IntegerLiteral(value)
        };

        Ok(self.token_from_range(kind, start, self.position, start_byte, line, column))
    }

    fn read_string(&mut self) -> LexResult<Token> {
        let start = self.position;
        let start_byte = self.byte_position;
        let line = self.line;
        let column = self.column;
        self.advance();

        let mut value = String::new();

        while let Some(ch) = self.current() {
            match ch {
                '"' => {
                    self.advance();
                    return Ok(self.token_from_range(
                        TokenKind::StringLiteral(value),
                        start,
                        self.position,
                        start_byte,
                        line,
                        column,
                    ));
                }
                '\\' => {
                    self.advance();

                    let escaped = match self.current() {
                        Some('"') => '"',
                        Some('\\') => '\\',
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some(other) => other,
                        None => {
                            return Err(LexError::new(
                                "unterminated string literal",
                                Span::new(
                                    self.file_path.clone(),
                                    start_byte,
                                    self.byte_position,
                                    line,
                                    column,
                                ),
                            ));
                        }
                    };

                    value.push(escaped);
                    self.advance();
                }
                _ => {
                    value.push(ch);
                    self.advance();
                }
            }
        }

        Err(LexError::new(
            "unterminated string literal",
            Span::new(
                self.file_path.clone(),
                start_byte,
                self.byte_position,
                line,
                column,
            ),
        ))
    }

    fn read_identifier_or_keyword(&mut self) -> Token {
        let start = self.position;
        let start_byte = self.byte_position;
        let line = self.line;
        let column = self.column;

        while matches!(
            self.current(),
            Some('a'..='z' | 'A'..='Z' | '0'..='9' | '_')
        ) {
            self.advance();
        }

        let lexeme = self.lexeme(start, self.position);

        let kind = match lexeme.as_str() {
            "module" => TokenKind::Module,
            "import" => TokenKind::Import,
            "const" => TokenKind::Const,
            "mut" => TokenKind::Mut,
            "func" => TokenKind::Func,
            "async" => TokenKind::Async,
            "await" => TokenKind::Await,
            "tool" => TokenKind::Tool,
            "agent" => TokenKind::Agent,
            "task" => TokenKind::Task,
            "workflow" => TokenKind::Workflow,
            "step" => TokenKind::Step,
            "struct" => TokenKind::Struct,
            "impl" => TokenKind::Impl,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "return" => TokenKind::Return,
            "ref" => TokenKind::Ref,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "i8" => TokenKind::I8Type,
            "i16" => TokenKind::I16Type,
            "i32" => TokenKind::I32Type,
            "i64" => TokenKind::I64Type,
            "u8" => TokenKind::U8Type,
            "u16" => TokenKind::U16Type,
            "u32" => TokenKind::U32Type,
            "u64" => TokenKind::U64Type,
            "f32" => TokenKind::F32Type,
            "f64" => TokenKind::F64Type,
            "Int" => TokenKind::IntType,
            "Float" => TokenKind::FloatType,
            "Bool" => TokenKind::BoolType,
            "Text" => TokenKind::TextType,
            "Char" => TokenKind::CharType,
            "Void" => TokenKind::VoidType,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "not" => TokenKind::Not,
            _ => TokenKind::Identifier(lexeme.clone()),
        };

        self.token_from_range(kind, start, self.position, start_byte, line, column)
    }

    fn token_from_range(
        &self,
        kind: TokenKind,
        start: usize,
        end: usize,
        start_byte: usize,
        line: usize,
        column: usize,
    ) -> Token {
        let span = Span::new(
            self.file_path.clone(),
            start_byte,
            self.byte_offsets
                .get(end)
                .copied()
                .unwrap_or(self.source.len()),
            line,
            column,
        );

        Token::new(kind, self.lexeme(start, end), span)
    }

    fn lexeme(&self, start: usize, end: usize) -> String {
        self.chars[start..end].iter().collect()
    }
}

pub fn tokenize(source: &str) -> LexResult<Vec<Token>> {
    Lexer::new(source).tokenize()
}

pub fn tokenize_with_file_path(
    source: &str,
    file_path: impl Into<String>,
) -> LexResult<Vec<Token>> {
    Lexer::with_file_path(source, Some(file_path.into())).tokenize()
}

#[cfg(test)]
mod tests {
    use super::{TokenKind, tokenize};

    #[test]
    fn tokenizes_sample_source() {
        let source = r#"const x: Int = 10;
func add(a: Int, b: Int) -> Int {
    return a + b;
}
"#;

        let tokens = tokenize(source).unwrap();

        assert_eq!(
            tokens.iter().map(|token| &token.kind).collect::<Vec<_>>(),
            vec![
                &TokenKind::Const,
                &TokenKind::Identifier("x".to_string()),
                &TokenKind::Colon,
                &TokenKind::IntType,
                &TokenKind::Equal,
                &TokenKind::IntegerLiteral(10),
                &TokenKind::Semicolon,
                &TokenKind::Func,
                &TokenKind::Identifier("add".to_string()),
                &TokenKind::LeftParen,
                &TokenKind::Identifier("a".to_string()),
                &TokenKind::Colon,
                &TokenKind::IntType,
                &TokenKind::Comma,
                &TokenKind::Identifier("b".to_string()),
                &TokenKind::Colon,
                &TokenKind::IntType,
                &TokenKind::RightParen,
                &TokenKind::Arrow,
                &TokenKind::IntType,
                &TokenKind::LeftBrace,
                &TokenKind::Return,
                &TokenKind::Identifier("a".to_string()),
                &TokenKind::Plus,
                &TokenKind::Identifier("b".to_string()),
                &TokenKind::Semicolon,
                &TokenKind::RightBrace,
                &TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn tracks_token_spans() {
        let source = "const x: i32 = 42;\n";
        let tokens = tokenize(source).unwrap();

        assert_eq!(tokens[0].span.line, 1);
        assert_eq!(tokens[0].span.column, 1);
        assert_eq!(tokens[1].span.line, 1);
        assert_eq!(tokens[1].span.column, 7);
        assert_eq!(tokens[5].span.line, 1);
        assert_eq!(tokens[5].span.column, 16);
    }

    #[test]
    fn reports_unterminated_string_with_diagnostic() {
        let source = "const name: Text = \"loz";
        let error = tokenize(source).unwrap_err();
        let rendered = error.diagnostic.render_with_source(Some(source));

        assert!(rendered.contains("error: unterminated string literal"));
        assert!(rendered.contains("^"));
    }
}
