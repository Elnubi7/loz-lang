#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Const,
    Mut,
    Func,
    Struct,
    If,
    Else,
    While,
    For,
    In,
    Return,
    True,
    False,
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
    FloatLiteral(f64),
    StringLiteral(String),
    Identifier(String),
    Eof,
}

pub struct Lexer<'a> {
    source: &'a str,
    chars: Vec<char>,
    position: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.chars().collect(),
            position: 0,
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
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
                    tokens.push(self.read_number());
                }
                '"' => {
                    tokens.push(self.read_string());
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    tokens.push(self.read_identifier_or_keyword());
                }
                ':' => {
                    tokens.push(Token::Colon);
                    self.advance();
                }
                ';' => {
                    tokens.push(Token::Semicolon);
                    self.advance();
                }
                ',' => {
                    tokens.push(Token::Comma);
                    self.advance();
                }
                '.' => {
                    tokens.push(Token::Dot);
                    self.advance();
                }
                '(' => {
                    tokens.push(Token::LeftParen);
                    self.advance();
                }
                ')' => {
                    tokens.push(Token::RightParen);
                    self.advance();
                }
                '{' => {
                    tokens.push(Token::LeftBrace);
                    self.advance();
                }
                '}' => {
                    tokens.push(Token::RightBrace);
                    self.advance();
                }
                '[' => {
                    tokens.push(Token::LeftBracket);
                    self.advance();
                }
                ']' => {
                    tokens.push(Token::RightBracket);
                    self.advance();
                }
                '=' => {
                    if self.peek() == Some('=') {
                        tokens.push(Token::EqualEqual);
                        self.advance();
                        self.advance();
                    } else {
                        tokens.push(Token::Equal);
                        self.advance();
                    }
                }
                '!' => {
                    if self.peek() == Some('=') {
                        tokens.push(Token::NotEqual);
                        self.advance();
                        self.advance();
                    } else {
                        panic!("unexpected character '!' in source: {}", self.source);
                    }
                }
                '>' => {
                    if self.peek() == Some('=') {
                        tokens.push(Token::GreaterEqual);
                        self.advance();
                        self.advance();
                    } else {
                        tokens.push(Token::Greater);
                        self.advance();
                    }
                }
                '<' => {
                    if self.peek() == Some('=') {
                        tokens.push(Token::LessEqual);
                        self.advance();
                        self.advance();
                    } else {
                        tokens.push(Token::Less);
                        self.advance();
                    }
                }
                '-' => {
                    if self.peek() == Some('>') {
                        tokens.push(Token::Arrow);
                        self.advance();
                        self.advance();
                    } else {
                        tokens.push(Token::Minus);
                        self.advance();
                    }
                }
                '+' => {
                    tokens.push(Token::Plus);
                    self.advance();
                }
                '*' => {
                    tokens.push(Token::Star);
                    self.advance();
                }
                '/' => {
                    tokens.push(Token::Slash);
                    self.advance();
                }
                '%' => {
                    tokens.push(Token::Percent);
                    self.advance();
                }
                _ => {
                    panic!("unexpected character '{}' in source: {}", ch, self.source);
                }
            }
        }

        tokens.push(Token::Eof);
        tokens
    }

    fn current(&self) -> Option<char> {
        self.chars.get(self.position).copied()
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.position + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.current();
        self.position += 1;
        ch
    }

    fn skip_comment(&mut self) {
        while let Some(ch) = self.current() {
            self.advance();

            if ch == '\n' {
                break;
            }
        }
    }

    fn read_number(&mut self) -> Token {
        let start = self.position;

        while matches!(self.current(), Some('0'..='9')) {
            self.advance();
        }

        let is_float = self.current() == Some('.') && matches!(self.peek(), Some('0'..='9'));

        if is_float {
            self.advance();

            while matches!(self.current(), Some('0'..='9')) {
                self.advance();
            }

            let value = self.lexeme(start, self.position).parse::<f64>().unwrap();
            Token::FloatLiteral(value)
        } else {
            let value = self.lexeme(start, self.position).parse::<i64>().unwrap();
            Token::IntegerLiteral(value)
        }
    }

    fn read_string(&mut self) -> Token {
        self.advance();
        let mut value = String::new();

        while let Some(ch) = self.current() {
            match ch {
                '"' => {
                    self.advance();
                    return Token::StringLiteral(value);
                }
                '\\' => {
                    self.advance();

                    let escaped = match self.current() {
                        Some('"') => '"',
                        Some('\\') => '\\',
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some(other) => other,
                        None => panic!("unterminated string literal"),
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

        panic!("unterminated string literal");
    }

    fn read_identifier_or_keyword(&mut self) -> Token {
        let start = self.position;

        while matches!(
            self.current(),
            Some('a'..='z' | 'A'..='Z' | '0'..='9' | '_')
        ) {
            self.advance();
        }

        let lexeme = self.lexeme(start, self.position);

        match lexeme.as_str() {
            "const" => Token::Const,
            "mut" => Token::Mut,
            "func" => Token::Func,
            "struct" => Token::Struct,
            "if" => Token::If,
            "else" => Token::Else,
            "while" => Token::While,
            "for" => Token::For,
            "in" => Token::In,
            "return" => Token::Return,
            "true" => Token::True,
            "false" => Token::False,
            "Int" => Token::IntType,
            "Float" => Token::FloatType,
            "Bool" => Token::BoolType,
            "Text" => Token::TextType,
            "Char" => Token::CharType,
            "Void" => Token::VoidType,
            "and" => Token::And,
            "or" => Token::Or,
            "not" => Token::Not,
            _ => Token::Identifier(lexeme),
        }
    }

    fn lexeme(&self, start: usize, end: usize) -> String {
        self.chars[start..end].iter().collect()
    }
}

pub fn tokenize(source: &str) -> Vec<Token> {
    Lexer::new(source).tokenize()
}

#[cfg(test)]
mod tests {
    use super::{Token, tokenize};

    #[test]
    fn tokenizes_sample_source() {
        let source = r#"const x: Int = 10;
func add(a: Int, b: Int) -> Int {
    return a + b;
}
"#;

        let tokens = tokenize(source);

        assert_eq!(
            tokens,
            vec![
                Token::Const,
                Token::Identifier("x".to_string()),
                Token::Colon,
                Token::IntType,
                Token::Equal,
                Token::IntegerLiteral(10),
                Token::Semicolon,
                Token::Func,
                Token::Identifier("add".to_string()),
                Token::LeftParen,
                Token::Identifier("a".to_string()),
                Token::Colon,
                Token::IntType,
                Token::Comma,
                Token::Identifier("b".to_string()),
                Token::Colon,
                Token::IntType,
                Token::RightParen,
                Token::Arrow,
                Token::IntType,
                Token::LeftBrace,
                Token::Return,
                Token::Identifier("a".to_string()),
                Token::Plus,
                Token::Identifier("b".to_string()),
                Token::Semicolon,
                Token::RightBrace,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn skips_comments_and_tokenizes_float_and_string_literals() {
        let source = r#"// comment
const pi: Float = 3.14;
const text: Text = "hello";
"#;

        let tokens = tokenize(source);

        assert_eq!(
            tokens,
            vec![
                Token::Const,
                Token::Identifier("pi".to_string()),
                Token::Colon,
                Token::FloatType,
                Token::Equal,
                Token::FloatLiteral(3.14),
                Token::Semicolon,
                Token::Const,
                Token::Identifier("text".to_string()),
                Token::Colon,
                Token::TextType,
                Token::Equal,
                Token::StringLiteral("hello".to_string()),
                Token::Semicolon,
                Token::Eof,
            ]
        );
    }
}
