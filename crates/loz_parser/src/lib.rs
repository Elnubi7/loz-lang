use std::fmt;
use std::mem;

use loz_ast::{
    AssignmentStatement, BinaryExpression, BinaryOperator, CallExpression, Expression,
    FunctionDeclaration, FunctionParameter, IfStatement, Program, ReturnStatement, Statement,
    TypeName, VariableDeclaration, WhileStatement,
};
use loz_lexer::Token;

pub type ParseResult<T> = Result<T, ParseError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

impl ParseError {
    fn new(message: impl Into<String>, position: usize) -> Self {
        Self {
            message: message.into(),
            position,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "parse error at token {}: {}",
            self.position, self.message
        )
    }
}

impl std::error::Error for ParseError {}

pub struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        let tokens = if tokens.is_empty() {
            vec![Token::Eof]
        } else {
            tokens
        };

        Self {
            tokens,
            position: 0,
        }
    }

    pub fn parse_program(&mut self) -> ParseResult<Program> {
        let mut statements = Vec::new();

        while !self.is_at_end() {
            statements.push(self.parse_statement()?);
        }

        Ok(Program { statements })
    }

    pub fn parse_statement(&mut self) -> ParseResult<Statement> {
        match self.current() {
            Token::Const | Token::Mut => self
                .parse_variable_declaration()
                .map(Statement::VariableDeclaration),
            Token::Func => self
                .parse_function_declaration()
                .map(Statement::FunctionDeclaration),
            Token::If => self.parse_if_statement().map(Statement::If),
            Token::While => self.parse_while_statement().map(Statement::While),
            Token::Return => self.parse_return_statement().map(Statement::Return),
            Token::For | Token::Struct => Err(ParseError::new(
                "unsupported statement for this parser phase",
                self.position,
            )),
            Token::Identifier(_) if self.peek_is(&Token::Equal) => {
                self.parse_assignment_statement().map(Statement::Assignment)
            }
            _ => {
                let expression = self.parse_expression()?;
                self.expect_token(Token::Semicolon, "expected ';' after expression")?;
                Ok(Statement::Expression(expression))
            }
        }
    }

    pub fn parse_expression(&mut self) -> ParseResult<Expression> {
        self.parse_comparison_expression()
    }

    fn parse_variable_declaration(&mut self) -> ParseResult<VariableDeclaration> {
        let is_mutable = match self.current() {
            Token::Const => {
                self.advance();
                false
            }
            Token::Mut => {
                self.advance();
                true
            }
            _ => {
                return Err(ParseError::new(
                    "expected 'const' or 'mut' for variable declaration",
                    self.position,
                ));
            }
        };

        let name = self.expect_identifier()?;
        self.expect_token(Token::Colon, "expected ':' after variable name")?;
        let type_name = self.parse_type_name()?;
        self.expect_token(Token::Equal, "expected '=' in variable declaration")?;
        let value = self.parse_expression()?;
        self.expect_token(Token::Semicolon, "expected ';' after variable declaration")?;

        Ok(VariableDeclaration {
            is_mutable,
            name,
            type_name,
            value,
        })
    }

    fn parse_function_declaration(&mut self) -> ParseResult<FunctionDeclaration> {
        self.expect_token(Token::Func, "expected 'func'")?;
        let name = self.expect_identifier()?;
        self.expect_token(Token::LeftParen, "expected '(' after function name")?;

        let mut parameters = Vec::new();
        if !self.current_is(&Token::RightParen) {
            loop {
                let parameter_name = self.expect_identifier()?;
                self.expect_token(Token::Colon, "expected ':' after parameter name")?;
                let parameter_type = self.parse_type_name()?;
                parameters.push(FunctionParameter {
                    name: parameter_name,
                    type_name: parameter_type,
                });

                if self.current_is(&Token::Comma) {
                    self.advance();
                    continue;
                }

                break;
            }
        }

        self.expect_token(Token::RightParen, "expected ')' after parameter list")?;
        self.expect_token(Token::Arrow, "expected '->' before function return type")?;
        let return_type = self.parse_type_name()?;
        self.expect_token(Token::LeftBrace, "expected '{' before function body")?;

        let mut body = Vec::new();
        while !self.current_is(&Token::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated function body, expected '}'",
                    self.position,
                ));
            }

            body.push(self.parse_statement()?);
        }

        self.expect_token(Token::RightBrace, "expected '}' after function body")?;

        Ok(FunctionDeclaration {
            name,
            parameters,
            return_type,
            body,
        })
    }

    fn parse_return_statement(&mut self) -> ParseResult<ReturnStatement> {
        self.expect_token(Token::Return, "expected 'return'")?;
        let value = self.parse_expression()?;
        self.expect_token(Token::Semicolon, "expected ';' after return value")?;
        Ok(ReturnStatement { value })
    }

    fn parse_assignment_statement(&mut self) -> ParseResult<AssignmentStatement> {
        let target = self.expect_identifier()?;
        self.expect_token(Token::Equal, "expected '=' in assignment")?;
        let value = self.parse_expression()?;
        self.expect_token(Token::Semicolon, "expected ';' after assignment")?;
        Ok(AssignmentStatement { target, value })
    }

    fn parse_if_statement(&mut self) -> ParseResult<IfStatement> {
        self.expect_token(Token::If, "expected 'if'")?;
        let condition = self.parse_expression()?;
        self.expect_token(Token::LeftBrace, "expected '{' after if condition")?;

        let mut then_branch = Vec::new();
        while !self.current_is(&Token::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated if block, expected '}'",
                    self.position,
                ));
            }

            then_branch.push(self.parse_statement()?);
        }

        self.expect_token(Token::RightBrace, "expected '}' after if block")?;

        let else_branch = if self.current_is(&Token::Else) {
            self.advance();
            self.expect_token(Token::LeftBrace, "expected '{' after else")?;

            let mut branch = Vec::new();
            while !self.current_is(&Token::RightBrace) {
                if self.is_at_end() {
                    return Err(ParseError::new(
                        "unterminated else block, expected '}'",
                        self.position,
                    ));
                }

                branch.push(self.parse_statement()?);
            }

            self.expect_token(Token::RightBrace, "expected '}' after else block")?;
            Some(branch)
        } else {
            None
        };

        Ok(IfStatement {
            condition,
            then_branch,
            else_branch,
        })
    }

    fn parse_while_statement(&mut self) -> ParseResult<WhileStatement> {
        self.expect_token(Token::While, "expected 'while'")?;
        let condition = self.parse_expression()?;
        self.expect_token(Token::LeftBrace, "expected '{' after while condition")?;

        let mut body = Vec::new();
        while !self.current_is(&Token::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated while block, expected '}'",
                    self.position,
                ));
            }

            body.push(self.parse_statement()?);
        }

        self.expect_token(Token::RightBrace, "expected '}' after while block")?;

        Ok(WhileStatement { condition, body })
    }

    fn parse_comparison_expression(&mut self) -> ParseResult<Expression> {
        let mut expression = self.parse_additive_expression()?;

        loop {
            let operator = match self.current() {
                Token::Greater => BinaryOperator::Greater,
                Token::Less => BinaryOperator::Less,
                Token::GreaterEqual => BinaryOperator::GreaterEqual,
                Token::LessEqual => BinaryOperator::LessEqual,
                Token::EqualEqual => BinaryOperator::Equal,
                Token::NotEqual => BinaryOperator::NotEqual,
                _ => break,
            };

            self.advance();
            let right = self.parse_additive_expression()?;
            expression = Expression::Binary(BinaryExpression {
                left: Box::new(expression),
                operator,
                right: Box::new(right),
            });
        }

        Ok(expression)
    }

    fn parse_additive_expression(&mut self) -> ParseResult<Expression> {
        let mut expression = self.parse_multiplicative_expression()?;

        loop {
            let operator = match self.current() {
                Token::Plus => BinaryOperator::Add,
                Token::Minus => BinaryOperator::Subtract,
                _ => break,
            };

            self.advance();
            let right = self.parse_multiplicative_expression()?;
            expression = Expression::Binary(BinaryExpression {
                left: Box::new(expression),
                operator,
                right: Box::new(right),
            });
        }

        Ok(expression)
    }

    fn parse_multiplicative_expression(&mut self) -> ParseResult<Expression> {
        let mut expression = self.parse_primary_expression()?;

        loop {
            let operator = match self.current() {
                Token::Star => BinaryOperator::Multiply,
                Token::Slash => BinaryOperator::Divide,
                _ => break,
            };

            self.advance();
            let right = self.parse_primary_expression()?;
            expression = Expression::Binary(BinaryExpression {
                left: Box::new(expression),
                operator,
                right: Box::new(right),
            });
        }

        Ok(expression)
    }

    fn parse_primary_expression(&mut self) -> ParseResult<Expression> {
        match self.advance() {
            Token::IntegerLiteral(value) => Ok(Expression::IntegerLiteral(value)),
            Token::FloatLiteral(value) => Ok(Expression::FloatLiteral(value)),
            Token::True => Ok(Expression::BooleanLiteral(true)),
            Token::False => Ok(Expression::BooleanLiteral(false)),
            Token::StringLiteral(value) => Ok(Expression::StringLiteral(value)),
            Token::Identifier(name) => {
                if self.current_is(&Token::LeftParen) {
                    self.parse_call_expression(name)
                } else {
                    Ok(Expression::Identifier(name))
                }
            }
            Token::LeftParen => {
                let expression = self.parse_expression()?;
                self.expect_token(Token::RightParen, "expected ')' after expression")?;
                Ok(expression)
            }
            token => Err(ParseError::new(
                format!("unexpected token in expression: {token:?}"),
                self.position.saturating_sub(1),
            )),
        }
    }

    fn parse_call_expression(&mut self, callee: String) -> ParseResult<Expression> {
        self.expect_token(Token::LeftParen, "expected '(' after function name")?;

        let mut arguments = Vec::new();
        if !self.current_is(&Token::RightParen) {
            loop {
                arguments.push(self.parse_expression()?);

                if self.current_is(&Token::Comma) {
                    self.advance();
                    continue;
                }

                break;
            }
        }

        self.expect_token(Token::RightParen, "expected ')' after function arguments")?;

        Ok(Expression::Call(CallExpression { callee, arguments }))
    }

    fn parse_type_name(&mut self) -> ParseResult<TypeName> {
        match self.advance() {
            Token::IntType => Ok(TypeName::Int),
            Token::FloatType => Ok(TypeName::Float),
            Token::BoolType => Ok(TypeName::Bool),
            Token::TextType => Ok(TypeName::Text),
            Token::CharType => Ok(TypeName::Char),
            Token::VoidType => Ok(TypeName::Void),
            Token::Identifier(name) => Ok(TypeName::Named(name)),
            token => Err(ParseError::new(
                format!("expected type name, found {token:?}"),
                self.position.saturating_sub(1),
            )),
        }
    }

    fn expect_identifier(&mut self) -> ParseResult<String> {
        match self.advance() {
            Token::Identifier(name) => Ok(name),
            token => Err(ParseError::new(
                format!("expected identifier, found {token:?}"),
                self.position.saturating_sub(1),
            )),
        }
    }

    fn expect_token(&mut self, expected: Token, message: &str) -> ParseResult<()> {
        if self.current_is(&expected) {
            self.advance();
            Ok(())
        } else {
            Err(ParseError::new(message, self.position))
        }
    }

    fn current(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or_else(|| {
            self.tokens
                .last()
                .expect("parser token stream is never empty")
        })
    }

    fn current_is(&self, expected: &Token) -> bool {
        mem::discriminant(self.current()) == mem::discriminant(expected)
    }

    fn peek_is(&self, expected: &Token) -> bool {
        self.tokens
            .get(self.position + 1)
            .map(|token| mem::discriminant(token) == mem::discriminant(expected))
            .unwrap_or(false)
    }

    fn advance(&mut self) -> Token {
        let token = self.current().clone();

        if !self.is_at_end() {
            self.position += 1;
        }

        token
    }

    fn is_at_end(&self) -> bool {
        matches!(self.current(), Token::Eof)
    }
}

pub fn parse_program(tokens: Vec<Token>) -> ParseResult<Program> {
    Parser::new(tokens).parse_program()
}

#[cfg(test)]
mod tests {
    use loz_ast::{
        AssignmentStatement, BinaryExpression, BinaryOperator, CallExpression, Expression,
        FunctionDeclaration, FunctionParameter, IfStatement, Program, ReturnStatement, Statement,
        TypeName, VariableDeclaration, WhileStatement,
    };
    use loz_lexer::tokenize;

    use super::parse_program;

    #[test]
    fn parses_sample_program() {
        let source = r#"const x: Int = 10;

func add(a: Int, b: Int) -> Int {
    return a + b;
}
"#;

        let program = parse_program(tokenize(source)).unwrap();

        assert_eq!(
            program,
            Program {
                statements: vec![
                    Statement::VariableDeclaration(VariableDeclaration {
                        is_mutable: false,
                        name: "x".to_string(),
                        type_name: TypeName::Int,
                        value: Expression::IntegerLiteral(10),
                    }),
                    Statement::FunctionDeclaration(FunctionDeclaration {
                        name: "add".to_string(),
                        parameters: vec![
                            FunctionParameter {
                                name: "a".to_string(),
                                type_name: TypeName::Int,
                            },
                            FunctionParameter {
                                name: "b".to_string(),
                                type_name: TypeName::Int,
                            },
                        ],
                        return_type: TypeName::Int,
                        body: vec![Statement::Return(ReturnStatement {
                            value: Expression::Binary(BinaryExpression {
                                left: Box::new(Expression::Identifier("a".to_string())),
                                operator: BinaryOperator::Add,
                                right: Box::new(Expression::Identifier("b".to_string())),
                            }),
                        })],
                    }),
                ],
            }
        );
    }

    #[test]
    fn respects_operator_precedence() {
        let source = "const value: Int = 1 + 2 * 3;";

        let program = parse_program(tokenize(source)).unwrap();

        assert_eq!(
            program,
            Program {
                statements: vec![Statement::VariableDeclaration(VariableDeclaration {
                    is_mutable: false,
                    name: "value".to_string(),
                    type_name: TypeName::Int,
                    value: Expression::Binary(BinaryExpression {
                        left: Box::new(Expression::IntegerLiteral(1)),
                        operator: BinaryOperator::Add,
                        right: Box::new(Expression::Binary(BinaryExpression {
                            left: Box::new(Expression::IntegerLiteral(2)),
                            operator: BinaryOperator::Multiply,
                            right: Box::new(Expression::IntegerLiteral(3)),
                        })),
                    }),
                })],
            }
        );
    }

    #[test]
    fn parses_assignment_statement() {
        let source = r#"mut x: Int = 10;
x = x + 1;
"#;

        let program = parse_program(tokenize(source)).unwrap();

        assert_eq!(
            program,
            Program {
                statements: vec![
                    Statement::VariableDeclaration(VariableDeclaration {
                        is_mutable: true,
                        name: "x".to_string(),
                        type_name: TypeName::Int,
                        value: Expression::IntegerLiteral(10),
                    }),
                    Statement::Assignment(AssignmentStatement {
                        target: "x".to_string(),
                        value: Expression::Binary(BinaryExpression {
                            left: Box::new(Expression::Identifier("x".to_string())),
                            operator: BinaryOperator::Add,
                            right: Box::new(Expression::IntegerLiteral(1)),
                        }),
                    }),
                ],
            }
        );
    }

    #[test]
    fn parses_call_expression_statement() {
        let source = r#"print(add(x, 20));"#;

        let program = parse_program(tokenize(source)).unwrap();

        assert_eq!(
            program,
            Program {
                statements: vec![Statement::Expression(Expression::Call(CallExpression {
                    callee: "print".to_string(),
                    arguments: vec![Expression::Call(CallExpression {
                        callee: "add".to_string(),
                        arguments: vec![
                            Expression::Identifier("x".to_string()),
                            Expression::IntegerLiteral(20),
                        ],
                    })],
                }))],
            }
        );
    }

    #[test]
    fn parses_if_else_statement() {
        let source = r#"func main() -> Int {
    mut x: Int = 10;
    if x > 5 {
        x = 100;
    } else {
        x = 0;
    }
    return x;
}
"#;

        let program = parse_program(tokenize(source)).unwrap();

        assert_eq!(
            program,
            Program {
                statements: vec![Statement::FunctionDeclaration(FunctionDeclaration {
                    name: "main".to_string(),
                    parameters: vec![],
                    return_type: TypeName::Int,
                    body: vec![
                        Statement::VariableDeclaration(VariableDeclaration {
                            is_mutable: true,
                            name: "x".to_string(),
                            type_name: TypeName::Int,
                            value: Expression::IntegerLiteral(10),
                        }),
                        Statement::If(IfStatement {
                            condition: Expression::Binary(BinaryExpression {
                                left: Box::new(Expression::Identifier("x".to_string())),
                                operator: BinaryOperator::Greater,
                                right: Box::new(Expression::IntegerLiteral(5)),
                            }),
                            then_branch: vec![Statement::Assignment(AssignmentStatement {
                                target: "x".to_string(),
                                value: Expression::IntegerLiteral(100),
                            })],
                            else_branch: Some(vec![Statement::Assignment(AssignmentStatement {
                                target: "x".to_string(),
                                value: Expression::IntegerLiteral(0),
                            })]),
                        }),
                        Statement::Return(ReturnStatement {
                            value: Expression::Identifier("x".to_string()),
                        }),
                    ],
                })],
            }
        );
    }

    #[test]
    fn parses_while_statement() {
        let source = r#"func main() -> Int {
    mut x: Int = 0;
    while x < 10 {
        x = x + 1;
    }
    return x;
}
"#;

        let program = parse_program(tokenize(source)).unwrap();

        assert_eq!(
            program,
            Program {
                statements: vec![Statement::FunctionDeclaration(FunctionDeclaration {
                    name: "main".to_string(),
                    parameters: vec![],
                    return_type: TypeName::Int,
                    body: vec![
                        Statement::VariableDeclaration(VariableDeclaration {
                            is_mutable: true,
                            name: "x".to_string(),
                            type_name: TypeName::Int,
                            value: Expression::IntegerLiteral(0),
                        }),
                        Statement::While(WhileStatement {
                            condition: Expression::Binary(BinaryExpression {
                                left: Box::new(Expression::Identifier("x".to_string())),
                                operator: BinaryOperator::Less,
                                right: Box::new(Expression::IntegerLiteral(10)),
                            }),
                            body: vec![Statement::Assignment(AssignmentStatement {
                                target: "x".to_string(),
                                value: Expression::Binary(BinaryExpression {
                                    left: Box::new(Expression::Identifier("x".to_string())),
                                    operator: BinaryOperator::Add,
                                    right: Box::new(Expression::IntegerLiteral(1)),
                                }),
                            })],
                        }),
                        Statement::Return(ReturnStatement {
                            value: Expression::Identifier("x".to_string()),
                        }),
                    ],
                })],
            }
        );
    }
}
