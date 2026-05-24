use std::fmt;
use std::mem;

use loz_ast::{
    AgentDeclaration, AgentTaskDeclaration, ArrayLiteralExpression, AssignmentStatement,
    AssignmentTarget, AsyncTaskDeclaration, AwaitExpression, BinaryExpression, BinaryOperator,
    CallExpression, CastExpression, DereferenceExpression, Diagnostic, Expression, ExpressionKind,
    FieldAccessExpression, ForStatement, FunctionDeclaration, FunctionParameter, IfStatement,
    ImplBlock, ImportDeclaration, IndexAccessExpression, MethodCallExpression, ModuleDeclaration,
    Program, ReferenceExpression, ReturnStatement, SchemaDeclaration, SchemaField, Span, Statement,
    StructDeclaration, StructField, ToolDeclaration, TypeName, VariableDeclaration, WhileStatement,
    WorkflowDeclaration, WorkflowStep, WorkflowTarget,
};
use loz_lexer::{Token, TokenKind};

pub type ParseResult<T> = Result<T, ParseError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub diagnostic: Diagnostic,
}

impl ParseError {
    fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            diagnostic: Diagnostic::error(message).with_span(span),
        }
    }

    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parse error: {}", self.diagnostic.message)
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
            vec![Token {
                kind: TokenKind::Eof,
                lexeme: String::new(),
                span: Span::default(),
            }]
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
        match &self.current().kind {
            TokenKind::Identifier(name)
                if name == "schema" && self.is_schema_declaration_ahead() =>
            {
                self.parse_schema_declaration()
                    .map(Statement::SchemaDeclaration)
            }
            TokenKind::Module => self
                .parse_module_declaration()
                .map(Statement::ModuleDeclaration),
            TokenKind::Import => self
                .parse_import_declaration()
                .map(Statement::ImportDeclaration),
            TokenKind::Const | TokenKind::Mut => self
                .parse_variable_declaration()
                .map(Statement::VariableDeclaration),
            TokenKind::Func => self
                .parse_function_declaration()
                .map(Statement::FunctionDeclaration),
            TokenKind::Async => self
                .parse_async_task_declaration()
                .map(Statement::AsyncTaskDeclaration),
            TokenKind::Tool => self
                .parse_tool_declaration()
                .map(Statement::ToolDeclaration),
            TokenKind::Agent => self
                .parse_agent_declaration()
                .map(Statement::AgentDeclaration),
            TokenKind::Workflow => self
                .parse_workflow_declaration()
                .map(Statement::WorkflowDeclaration),
            TokenKind::Struct => self
                .parse_struct_declaration()
                .map(Statement::StructDeclaration),
            TokenKind::Impl => self.parse_impl_block().map(Statement::ImplBlock),
            TokenKind::If => self.parse_if_statement().map(Statement::If),
            TokenKind::While => self.parse_while_statement().map(Statement::While),
            TokenKind::For => self.parse_for_statement().map(Statement::For),
            TokenKind::Break => self.parse_break_statement(),
            TokenKind::Continue => self.parse_continue_statement(),
            TokenKind::Return => self.parse_return_statement().map(Statement::Return),
            TokenKind::Star => self.parse_assignment_statement().map(Statement::Assignment),
            TokenKind::Identifier(_)
                if self.peek_is(&TokenKind::Equal)
                    || (self.peek_is(&TokenKind::Dot)
                        && self.peek_n_is(2, &TokenKind::Identifier(String::new()))
                        && self.peek_n_is(3, &TokenKind::Equal))
                    || self.is_index_assignment_ahead() =>
            {
                self.parse_assignment_statement().map(Statement::Assignment)
            }
            _ => {
                let expression = self.parse_expression()?;
                self.expect_token(&TokenKind::Semicolon, "expected ';' after expression")?;
                Ok(Statement::Expression(expression))
            }
        }
    }

    pub fn parse_expression(&mut self) -> ParseResult<Expression> {
        self.parse_comparison_expression()
    }

    fn parse_module_declaration(&mut self) -> ParseResult<ModuleDeclaration> {
        let start = self.expect_token(&TokenKind::Module, "expected 'module'")?;
        let name = self.expect_identifier()?;
        self.expect_token(
            &TokenKind::Semicolon,
            "expected ';' after module declaration",
        )?;
        Ok(ModuleDeclaration {
            name,
            span: start.span,
        })
    }

    fn parse_import_declaration(&mut self) -> ParseResult<ImportDeclaration> {
        let start = self.expect_token(&TokenKind::Import, "expected 'import'")?;
        let module_name = self.expect_identifier()?;
        self.expect_token(
            &TokenKind::Semicolon,
            "expected ';' after import declaration",
        )?;
        Ok(ImportDeclaration {
            module_name,
            span: start.span,
        })
    }

    fn parse_variable_declaration(&mut self) -> ParseResult<VariableDeclaration> {
        let start = self.current().span.clone();
        let is_mutable = match &self.current().kind {
            TokenKind::Const => {
                self.advance();
                false
            }
            TokenKind::Mut => {
                self.advance();
                true
            }
            _ => {
                return Err(ParseError::new(
                    "expected 'const' or 'mut' for variable declaration",
                    self.current().span.clone(),
                ));
            }
        };

        let name = self.expect_identifier()?;
        let type_name = if self.current_is(&TokenKind::Colon) {
            self.advance();
            Some(self.parse_type_name()?)
        } else {
            None
        };
        self.expect_token(&TokenKind::Equal, "expected '=' in variable declaration")?;
        let value = self.parse_expression()?;
        let end = self.expect_token(
            &TokenKind::Semicolon,
            "expected ';' after variable declaration",
        )?;

        Ok(VariableDeclaration {
            is_mutable,
            name,
            type_name,
            value,
            span: Span::cover(&start, &end.span),
        })
    }

    fn parse_function_declaration(&mut self) -> ParseResult<FunctionDeclaration> {
        self.expect_token(&TokenKind::Func, "expected 'func'")?;
        self.parse_callable_declaration()
    }

    fn parse_tool_declaration(&mut self) -> ParseResult<ToolDeclaration> {
        let start = self.expect_token(&TokenKind::Tool, "expected 'tool'")?;
        let callable = self.parse_callable_declaration()?;

        Ok(ToolDeclaration {
            name: callable.name,
            parameters: callable.parameters,
            return_type: callable.return_type,
            body: callable.body,
            span: Span::cover(&start.span, &callable.span),
        })
    }

    fn parse_async_task_declaration(&mut self) -> ParseResult<AsyncTaskDeclaration> {
        let start = self.expect_token(&TokenKind::Async, "expected 'async'")?;
        self.expect_token(&TokenKind::Task, "expected 'task' after 'async'")?;
        let callable = self.parse_callable_declaration()?;

        Ok(AsyncTaskDeclaration {
            name: callable.name,
            parameters: callable.parameters,
            return_type: callable.return_type,
            body: callable.body,
            span: Span::cover(&start.span, &callable.span),
        })
    }

    fn parse_agent_declaration(&mut self) -> ParseResult<AgentDeclaration> {
        let start = self.expect_token(&TokenKind::Agent, "expected 'agent'")?;
        let name = self.expect_identifier()?;
        let _left_brace =
            self.expect_token(&TokenKind::LeftBrace, "expected '{' after agent name")?;

        let mut model = None;
        let mut tools = None;
        let mut tasks = Vec::new();

        while !self.current_is(&TokenKind::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated agent body, expected '}'",
                    self.current().span.clone(),
                ));
            }

            match &self.current().kind {
                TokenKind::Task => tasks.push(self.parse_agent_task_declaration()?),
                TokenKind::Identifier(identifier) if identifier == "model" => {
                    self.advance();
                    self.expect_token(&TokenKind::Colon, "expected ':' after agent model")?;
                    model = Some(self.parse_expression()?);
                    self.expect_token(&TokenKind::Semicolon, "expected ';' after agent model")?;
                }
                TokenKind::Identifier(identifier) if identifier == "tools" => {
                    self.advance();
                    self.expect_token(&TokenKind::Colon, "expected ':' after agent tools")?;
                    tools = Some(self.parse_expression()?);
                    self.expect_token(&TokenKind::Semicolon, "expected ';' after agent tools")?;
                }
                TokenKind::Identifier(identifier) => {
                    return Err(ParseError::new(
                        format!("unknown agent field '{}'", identifier),
                        self.current().span.clone(),
                    ));
                }
                _ => {
                    return Err(ParseError::new(
                        "expected agent field declaration or task declaration",
                        self.current().span.clone(),
                    ));
                }
            }
        }

        let end = self.expect_token(&TokenKind::RightBrace, "expected '}' after agent body")?;

        Ok(AgentDeclaration {
            name,
            model,
            tools,
            tasks,
            span: Span::cover(&start.span, &end.span),
        })
    }

    fn parse_agent_task_declaration(&mut self) -> ParseResult<AgentTaskDeclaration> {
        let start = self.expect_token(&TokenKind::Task, "expected 'task'")?;
        let callable = self.parse_callable_declaration()?;

        Ok(AgentTaskDeclaration {
            name: callable.name,
            parameters: callable.parameters,
            return_type: callable.return_type,
            body: callable.body,
            span: Span::cover(&start.span, &callable.span),
        })
    }

    fn parse_workflow_declaration(&mut self) -> ParseResult<WorkflowDeclaration> {
        let start = self.expect_token(&TokenKind::Workflow, "expected 'workflow'")?;
        let name = self.expect_identifier()?;
        self.expect_token(&TokenKind::LeftBrace, "expected '{' after workflow name")?;

        let mut steps = Vec::new();
        while !self.current_is(&TokenKind::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated workflow body, expected '}'",
                    self.current().span.clone(),
                ));
            }

            steps.push(self.parse_workflow_step()?);
        }

        let end = self.expect_token(&TokenKind::RightBrace, "expected '}' after workflow body")?;

        Ok(WorkflowDeclaration {
            name,
            steps,
            span: Span::cover(&start.span, &end.span),
        })
    }

    fn parse_workflow_step(&mut self) -> ParseResult<WorkflowStep> {
        let start = self.expect_token(&TokenKind::Step, "expected 'step'")?;
        let first_identifier = self.expect_identifier()?;
        let (name, target) = if self.current_is(&TokenKind::Dot) {
            self.advance();
            let second_identifier = self.expect_identifier()?;
            let display_name = format!("{first_identifier}.{second_identifier}");
            (
                display_name,
                WorkflowTarget::AgentTask {
                    agent_name: first_identifier,
                    task_name: second_identifier,
                },
            )
        } else {
            (
                first_identifier.clone(),
                WorkflowTarget::FunctionOrTool(first_identifier),
            )
        };
        let end = self.expect_token(&TokenKind::Semicolon, "expected ';' after workflow step")?;

        Ok(WorkflowStep {
            name,
            target,
            span: Span::cover(&start.span, &end.span),
        })
    }

    fn parse_callable_declaration(&mut self) -> ParseResult<FunctionDeclaration> {
        let name_token = self.expect_identifier_token()?;
        self.expect_token(&TokenKind::LeftParen, "expected '(' after function name")?;

        let parameters = self.parse_parameter_list(None)?;
        self.expect_token(&TokenKind::RightParen, "expected ')' after parameter list")?;
        self.expect_token(
            &TokenKind::Arrow,
            "expected '->' before function return type",
        )?;
        let return_type = self.parse_type_name()?;
        self.expect_token(&TokenKind::LeftBrace, "expected '{' before function body")?;

        let body = self.parse_block_statements("function body")?;
        let end_span = body
            .last()
            .map(|statement| statement.span().clone())
            .unwrap_or_else(|| self.previous().span.clone());

        Ok(FunctionDeclaration {
            name: name_token.lexeme,
            parameters,
            return_type,
            body,
            span: Span::cover(&name_token.span, &end_span),
        })
    }

    fn parse_method_declaration(&mut self, target_name: &str) -> ParseResult<FunctionDeclaration> {
        self.expect_token(&TokenKind::Func, "expected 'func' in impl block")?;
        let name_token = self.expect_identifier_token()?;
        self.expect_token(&TokenKind::LeftParen, "expected '(' after method name")?;

        let parameters = self.parse_parameter_list(Some(target_name))?;
        self.expect_token(&TokenKind::RightParen, "expected ')' after parameter list")?;
        self.expect_token(&TokenKind::Arrow, "expected '->' before method return type")?;
        let return_type = self.parse_type_name()?;
        self.expect_token(&TokenKind::LeftBrace, "expected '{' before method body")?;

        let body = self.parse_block_statements("method body")?;
        let end_span = body
            .last()
            .map(|statement| statement.span().clone())
            .unwrap_or_else(|| self.previous().span.clone());

        Ok(FunctionDeclaration {
            name: name_token.lexeme,
            parameters,
            return_type,
            body,
            span: Span::cover(&name_token.span, &end_span),
        })
    }

    fn parse_parameter_list(
        &mut self,
        method_target_name: Option<&str>,
    ) -> ParseResult<Vec<FunctionParameter>> {
        let mut parameters = Vec::new();
        if self.current_is(&TokenKind::RightParen) {
            if method_target_name.is_some() {
                return Err(ParseError::new(
                    "methods in impl blocks must declare 'self' as the first parameter",
                    self.current().span.clone(),
                ));
            }

            return Ok(parameters);
        }

        if let Some(target_name) = method_target_name {
            let parameter_token = self.expect_identifier_token()?;
            if parameter_token.lexeme != "self" {
                return Err(ParseError::new(
                    "methods in impl blocks must declare 'self' as the first parameter",
                    parameter_token.span,
                ));
            }

            if self.current_is(&TokenKind::Colon) {
                return Err(ParseError::new(
                    "method self parameter must not declare an explicit type",
                    self.current().span.clone(),
                ));
            }

            parameters.push(FunctionParameter {
                name: "self".to_string(),
                type_name: TypeName::Named(target_name.to_string()),
                span: parameter_token.span,
            });

            if self.current_is(&TokenKind::Comma) {
                self.advance();
            } else {
                return Ok(parameters);
            }
        }

        loop {
            let parameter_token = self.expect_identifier_token()?;
            self.expect_token(&TokenKind::Colon, "expected ':' after parameter name")?;
            let parameter_type = self.parse_type_name()?;
            parameters.push(FunctionParameter {
                name: parameter_token.lexeme,
                type_name: parameter_type,
                span: parameter_token.span,
            });

            if self.current_is(&TokenKind::Comma) {
                self.advance();
                continue;
            }

            break;
        }

        Ok(parameters)
    }

    fn parse_struct_declaration(&mut self) -> ParseResult<StructDeclaration> {
        let start = self.expect_token(&TokenKind::Struct, "expected 'struct'")?;
        let name = self.expect_identifier()?;
        self.expect_token(&TokenKind::LeftBrace, "expected '{' after struct name")?;

        let mut fields = Vec::new();
        while !self.current_is(&TokenKind::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated struct body, expected '}'",
                    self.current().span.clone(),
                ));
            }

            let field_token = self.expect_identifier_token()?;
            self.expect_token(&TokenKind::Colon, "expected ':' after struct field name")?;
            let field_type = self.parse_type_name()?;
            fields.push(StructField {
                name: field_token.lexeme,
                type_name: field_type,
                span: field_token.span,
            });

            if self.current_is(&TokenKind::Comma) {
                self.advance();

                if self.current_is(&TokenKind::RightBrace) {
                    break;
                }
            } else {
                break;
            }
        }

        let end = self.expect_token(&TokenKind::RightBrace, "expected '}' after struct body")?;

        Ok(StructDeclaration {
            name,
            fields,
            span: Span::cover(&start.span, &end.span),
        })
    }

    fn parse_schema_declaration(&mut self) -> ParseResult<SchemaDeclaration> {
        let start = self.expect_identifier_lexeme("schema")?;
        let name = self.expect_identifier()?;
        self.expect_token(&TokenKind::LeftBrace, "expected '{' after schema name")?;

        let mut fields = Vec::new();
        while !self.current_is(&TokenKind::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated schema body, expected '}'",
                    self.current().span.clone(),
                ));
            }

            let field_token = self.expect_identifier_token()?;
            self.expect_token(&TokenKind::Colon, "expected ':' after schema field name")?;
            let field_type = self.parse_type_name()?;
            fields.push(SchemaField {
                name: field_token.lexeme,
                type_name: field_type,
                span: field_token.span,
            });

            if self.current_is(&TokenKind::Comma) {
                self.advance();

                if self.current_is(&TokenKind::RightBrace) {
                    break;
                }
            } else {
                break;
            }
        }

        let end = self.expect_token(&TokenKind::RightBrace, "expected '}' after schema body")?;

        Ok(SchemaDeclaration {
            name,
            fields,
            span: Span::cover(&start.span, &end.span),
        })
    }

    fn parse_impl_block(&mut self) -> ParseResult<ImplBlock> {
        let start = self.expect_token(&TokenKind::Impl, "expected 'impl'")?;
        let target_name = self.expect_identifier()?;
        self.expect_token(&TokenKind::LeftBrace, "expected '{' after impl target")?;

        let mut methods = Vec::new();
        while !self.current_is(&TokenKind::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated impl block, expected '}'",
                    self.current().span.clone(),
                ));
            }

            if !self.current_is(&TokenKind::Func) {
                return Err(ParseError::new(
                    "impl blocks may only contain method declarations",
                    self.current().span.clone(),
                ));
            }

            methods.push(self.parse_method_declaration(&target_name)?);
        }

        let end = self.expect_token(&TokenKind::RightBrace, "expected '}' after impl block")?;

        Ok(ImplBlock {
            target_name,
            methods,
            span: Span::cover(&start.span, &end.span),
        })
    }

    fn parse_return_statement(&mut self) -> ParseResult<ReturnStatement> {
        let start = self.expect_token(&TokenKind::Return, "expected 'return'")?;
        let value = self.parse_expression()?;
        let end = self.expect_token(&TokenKind::Semicolon, "expected ';' after return value")?;
        Ok(ReturnStatement {
            value,
            span: Span::cover(&start.span, &end.span),
        })
    }

    fn parse_assignment_statement(&mut self) -> ParseResult<AssignmentStatement> {
        let start = self.current().span.clone();

        if self.current_is(&TokenKind::Star) {
            let target = self.parse_unary_expression()?;
            let ExpressionKind::Dereference(dereference) = target.kind else {
                return Err(ParseError::new(
                    "expected dereference assignment target",
                    target.span,
                ));
            };

            self.expect_token(&TokenKind::Equal, "expected '=' in assignment")?;
            let value = self.parse_expression()?;
            let end = self.expect_token(&TokenKind::Semicolon, "expected ';' after assignment")?;
            return Ok(AssignmentStatement {
                target: AssignmentTarget::Dereference(dereference),
                value,
                span: Span::cover(&start, &end.span),
            });
        }

        let base_name = self.expect_identifier()?;
        let target = if self.current_is(&TokenKind::Dot) {
            self.expect_token(&TokenKind::Dot, "expected '.' in field assignment")?;
            let field_name = self.expect_identifier()?;

            if self.current_is(&TokenKind::Dot) {
                return Err(ParseError::new(
                    "nested field assignment is not supported in this phase",
                    self.current().span.clone(),
                ));
            }

            AssignmentTarget::FieldAccess(FieldAccessExpression {
                base_name,
                field_name,
            })
        } else if self.current_is(&TokenKind::LeftBracket) {
            self.expect_token(
                &TokenKind::LeftBracket,
                "expected '[' in array element assignment",
            )?;
            let index = self.parse_expression()?;
            self.expect_token(
                &TokenKind::RightBracket,
                "expected ']' after array element assignment index",
            )?;

            if self.current_is(&TokenKind::LeftBracket) {
                return Err(ParseError::new(
                    "nested array element assignment is not supported in this phase",
                    self.current().span.clone(),
                ));
            }

            AssignmentTarget::IndexAccess(IndexAccessExpression {
                base_name,
                index: Box::new(index),
            })
        } else {
            AssignmentTarget::Identifier(base_name)
        };

        self.expect_token(&TokenKind::Equal, "expected '=' in assignment")?;
        let value = self.parse_expression()?;
        let end = self.expect_token(&TokenKind::Semicolon, "expected ';' after assignment")?;
        Ok(AssignmentStatement {
            target,
            value,
            span: Span::cover(&start, &end.span),
        })
    }

    fn parse_if_statement(&mut self) -> ParseResult<IfStatement> {
        let start = self.expect_token(&TokenKind::If, "expected 'if'")?;
        let condition = self.parse_expression()?;
        self.expect_token(&TokenKind::LeftBrace, "expected '{' after if condition")?;

        let mut then_branch = Vec::new();
        while !self.current_is(&TokenKind::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated if block, expected '}'",
                    self.current().span.clone(),
                ));
            }

            then_branch.push(self.parse_statement()?);
        }

        let mut end = self.expect_token(&TokenKind::RightBrace, "expected '}' after if block")?;

        let else_branch = if self.current_is(&TokenKind::Else) {
            self.advance();
            self.expect_token(&TokenKind::LeftBrace, "expected '{' after else")?;

            let mut branch = Vec::new();
            while !self.current_is(&TokenKind::RightBrace) {
                if self.is_at_end() {
                    return Err(ParseError::new(
                        "unterminated else block, expected '}'",
                        self.current().span.clone(),
                    ));
                }

                branch.push(self.parse_statement()?);
            }

            end = self.expect_token(&TokenKind::RightBrace, "expected '}' after else block")?;
            Some(branch)
        } else {
            None
        };

        Ok(IfStatement {
            condition,
            then_branch,
            else_branch,
            span: Span::cover(&start.span, &end.span),
        })
    }

    fn parse_while_statement(&mut self) -> ParseResult<WhileStatement> {
        let start = self.expect_token(&TokenKind::While, "expected 'while'")?;
        let condition = self.parse_expression()?;
        self.expect_token(&TokenKind::LeftBrace, "expected '{' after while condition")?;

        let mut body = Vec::new();
        while !self.current_is(&TokenKind::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated while block, expected '}'",
                    self.current().span.clone(),
                ));
            }

            body.push(self.parse_statement()?);
        }

        let end = self.expect_token(&TokenKind::RightBrace, "expected '}' after while block")?;

        Ok(WhileStatement {
            condition,
            body,
            span: Span::cover(&start.span, &end.span),
        })
    }

    fn parse_for_statement(&mut self) -> ParseResult<ForStatement> {
        let start = self.expect_token(&TokenKind::For, "expected 'for'")?;

        let is_mutable = if self.current_is(&TokenKind::Mut) {
            self.advance();
            true
        } else {
            false
        };

        let variable_name = self.expect_identifier()?;
        self.expect_token(&TokenKind::In, "expected 'in' after loop variable")?;
        let iterable = self.parse_expression()?;
        self.expect_token(
            &TokenKind::LeftBrace,
            "expected '{' after for-loop iterable",
        )?;

        let mut body = Vec::new();
        while !self.current_is(&TokenKind::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    "unterminated for block, expected '}'",
                    self.current().span.clone(),
                ));
            }

            body.push(self.parse_statement()?);
        }

        let end = self.expect_token(&TokenKind::RightBrace, "expected '}' after for block")?;

        Ok(ForStatement {
            variable_name,
            is_mutable,
            iterable,
            body,
            span: Span::cover(&start.span, &end.span),
        })
    }

    fn parse_break_statement(&mut self) -> ParseResult<Statement> {
        let start = self.expect_token(&TokenKind::Break, "expected 'break'")?;
        let end = self.expect_token(&TokenKind::Semicolon, "expected ';' after break")?;
        Ok(Statement::Break(Span::cover(&start.span, &end.span)))
    }

    fn parse_continue_statement(&mut self) -> ParseResult<Statement> {
        let start = self.expect_token(&TokenKind::Continue, "expected 'continue'")?;
        let end = self.expect_token(&TokenKind::Semicolon, "expected ';' after continue")?;
        Ok(Statement::Continue(Span::cover(&start.span, &end.span)))
    }

    fn parse_comparison_expression(&mut self) -> ParseResult<Expression> {
        let mut expression = self.parse_additive_expression()?;

        loop {
            let operator = match &self.current().kind {
                TokenKind::Greater => BinaryOperator::Greater,
                TokenKind::Less => BinaryOperator::Less,
                TokenKind::GreaterEqual => BinaryOperator::GreaterEqual,
                TokenKind::LessEqual => BinaryOperator::LessEqual,
                TokenKind::EqualEqual => BinaryOperator::Equal,
                TokenKind::NotEqual => BinaryOperator::NotEqual,
                _ => break,
            };

            self.advance();
            let right = self.parse_additive_expression()?;
            let span = Span::cover(&expression.span, &right.span);
            expression = Expression::new(
                ExpressionKind::Binary(BinaryExpression {
                    left: Box::new(expression),
                    operator,
                    right: Box::new(right),
                }),
                span,
            );
        }

        Ok(expression)
    }

    fn parse_additive_expression(&mut self) -> ParseResult<Expression> {
        let mut expression = self.parse_multiplicative_expression()?;

        loop {
            let operator = match &self.current().kind {
                TokenKind::Plus => BinaryOperator::Add,
                TokenKind::Minus => BinaryOperator::Subtract,
                _ => break,
            };

            self.advance();
            let right = self.parse_multiplicative_expression()?;
            let span = Span::cover(&expression.span, &right.span);
            expression = Expression::new(
                ExpressionKind::Binary(BinaryExpression {
                    left: Box::new(expression),
                    operator,
                    right: Box::new(right),
                }),
                span,
            );
        }

        Ok(expression)
    }

    fn parse_multiplicative_expression(&mut self) -> ParseResult<Expression> {
        let mut expression = self.parse_unary_expression()?;

        loop {
            let operator = match &self.current().kind {
                TokenKind::Star => BinaryOperator::Multiply,
                TokenKind::Slash => BinaryOperator::Divide,
                _ => break,
            };

            self.advance();
            let right = self.parse_unary_expression()?;
            let span = Span::cover(&expression.span, &right.span);
            expression = Expression::new(
                ExpressionKind::Binary(BinaryExpression {
                    left: Box::new(expression),
                    operator,
                    right: Box::new(right),
                }),
                span,
            );
        }

        Ok(expression)
    }

    fn parse_unary_expression(&mut self) -> ParseResult<Expression> {
        match &self.current().kind {
            TokenKind::Await => {
                let start = self.advance();
                let expression = self.parse_unary_expression()?;
                let span = Span::cover(&start.span, &expression.span);
                Ok(Expression::new(
                    ExpressionKind::Await(AwaitExpression {
                        expression: Box::new(expression),
                    }),
                    span,
                ))
            }
            TokenKind::Star => {
                let start = self.advance();
                let value = self.parse_unary_expression()?;
                let span = Span::cover(&start.span, &value.span);
                Ok(Expression::new(
                    ExpressionKind::Dereference(DereferenceExpression {
                        value: Box::new(value),
                    }),
                    span,
                ))
            }
            TokenKind::Ref => self.parse_reference_expression(false),
            TokenKind::Mut => {
                self.advance();
                self.expect_token(&TokenKind::Ref, "expected 'ref' after 'mut'")?;
                self.parse_reference_expression(true)
            }
            _ => self.parse_primary_expression(),
        }
    }

    fn parse_primary_expression(&mut self) -> ParseResult<Expression> {
        let token = self.advance();
        match token.kind {
            TokenKind::IntegerLiteral(value) => Ok(Expression::new(
                ExpressionKind::IntegerLiteral(value),
                token.span,
            )),
            TokenKind::FloatLiteral(value) => {
                Ok(Expression::new(
                    ExpressionKind::FloatLiteral(value.parse::<f64>().map_err(|_| {
                        ParseError::new("invalid float literal", token.span.clone())
                    })?),
                    token.span,
                ))
            }
            TokenKind::True => Ok(Expression::new(
                ExpressionKind::BooleanLiteral(true),
                token.span,
            )),
            TokenKind::False => Ok(Expression::new(
                ExpressionKind::BooleanLiteral(false),
                token.span,
            )),
            TokenKind::StringLiteral(value) => Ok(Expression::new(
                ExpressionKind::StringLiteral(value),
                token.span,
            )),
            TokenKind::Identifier(name) => {
                if name == "as" && self.current_is(&TokenKind::Less) {
                    self.parse_cast_expression(token.span)
                } else if self.current_is(&TokenKind::LeftParen) {
                    self.parse_call_expression(name, token.span)
                } else if self.current_is(&TokenKind::Dot) {
                    self.parse_postfix_dot_expression(name, token.span)
                } else if self.current_is(&TokenKind::LeftBracket) {
                    self.parse_index_access_expression(name, token.span)
                } else {
                    Ok(Expression::new(
                        ExpressionKind::Identifier(name),
                        token.span,
                    ))
                }
            }
            TokenKind::LeftBracket => self.parse_array_literal_expression(token.span),
            TokenKind::LeftParen => {
                let expression = self.parse_expression()?;
                self.expect_token(&TokenKind::RightParen, "expected ')' after expression")?;
                Ok(expression)
            }
            token_kind => Err(ParseError::new(
                format!("unexpected token in expression: {token_kind:?}"),
                token.span,
            )),
        }
    }

    fn parse_call_expression(
        &mut self,
        callee: String,
        start_span: Span,
    ) -> ParseResult<Expression> {
        self.expect_token(&TokenKind::LeftParen, "expected '(' after function name")?;

        let mut arguments = Vec::new();
        if !self.current_is(&TokenKind::RightParen) {
            loop {
                arguments.push(self.parse_expression()?);

                if self.current_is(&TokenKind::Comma) {
                    self.advance();
                    continue;
                }

                break;
            }
        }

        let end = self.expect_token(
            &TokenKind::RightParen,
            "expected ')' after function arguments",
        )?;

        Ok(Expression::new(
            ExpressionKind::Call(CallExpression { callee, arguments }),
            Span::cover(&start_span, &end.span),
        ))
    }

    fn parse_cast_expression(&mut self, start_span: Span) -> ParseResult<Expression> {
        self.expect_token(&TokenKind::Less, "expected '<' after 'as'")?;
        let target_type = self.parse_type_name()?;
        self.expect_token(&TokenKind::Greater, "expected '>' after cast target type")?;
        self.expect_token(&TokenKind::LeftParen, "expected '(' after cast target type")?;
        let value = self.parse_expression()?;
        let end =
            self.expect_token(&TokenKind::RightParen, "expected ')' after cast expression")?;

        Ok(Expression::new(
            ExpressionKind::Cast(CastExpression {
                target_type,
                value: Box::new(value),
            }),
            Span::cover(&start_span, &end.span),
        ))
    }

    fn parse_reference_expression(&mut self, is_mutable: bool) -> ParseResult<Expression> {
        let start = if !is_mutable {
            self.expect_token(&TokenKind::Ref, "expected 'ref'")?
        } else {
            self.previous().clone()
        };

        let target = self.expect_identifier_token()?;
        Ok(Expression::new(
            ExpressionKind::Reference(ReferenceExpression {
                target_name: target.lexeme,
                is_mutable,
            }),
            Span::cover(&start.span, &target.span),
        ))
    }

    fn parse_postfix_dot_expression(
        &mut self,
        base_name: String,
        start_span: Span,
    ) -> ParseResult<Expression> {
        self.expect_token(&TokenKind::Dot, "expected '.' for field access")?;
        let member_name = self.expect_identifier()?;

        if self.current_is(&TokenKind::LeftParen) {
            return self.parse_method_call_expression(base_name, member_name, start_span);
        }

        if self.current_is(&TokenKind::Dot) {
            return Err(ParseError::new(
                "nested field access is not supported in this phase",
                self.current().span.clone(),
            ));
        }

        let end = self.previous().span.clone();
        Ok(Expression::new(
            ExpressionKind::FieldAccess(FieldAccessExpression {
                base_name,
                field_name: member_name,
            }),
            Span::cover(&start_span, &end),
        ))
    }

    fn parse_method_call_expression(
        &mut self,
        base_name: String,
        method_name: String,
        start_span: Span,
    ) -> ParseResult<Expression> {
        self.expect_token(&TokenKind::LeftParen, "expected '(' after method name")?;

        let mut arguments = Vec::new();
        if !self.current_is(&TokenKind::RightParen) {
            loop {
                arguments.push(self.parse_expression()?);

                if self.current_is(&TokenKind::Comma) {
                    self.advance();
                    continue;
                }

                break;
            }
        }

        let end = self.expect_token(
            &TokenKind::RightParen,
            "expected ')' after method arguments",
        )?;

        Ok(Expression::new(
            ExpressionKind::MethodCall(MethodCallExpression {
                base_name,
                method_name,
                arguments,
            }),
            Span::cover(&start_span, &end.span),
        ))
    }

    fn parse_block_statements(&mut self, context: &str) -> ParseResult<Vec<Statement>> {
        let mut body = Vec::new();
        while !self.current_is(&TokenKind::RightBrace) {
            if self.is_at_end() {
                return Err(ParseError::new(
                    format!("unterminated {context}, expected '}}'"),
                    self.current().span.clone(),
                ));
            }

            body.push(self.parse_statement()?);
        }

        self.expect_token(
            &TokenKind::RightBrace,
            &format!("expected '}}' after {context}"),
        )?;
        Ok(body)
    }

    fn parse_array_literal_expression(&mut self, start_span: Span) -> ParseResult<Expression> {
        let mut elements = Vec::new();
        if !self.current_is(&TokenKind::RightBracket) {
            loop {
                elements.push(self.parse_expression()?);

                if self.current_is(&TokenKind::Comma) {
                    self.advance();
                    continue;
                }

                break;
            }
        }

        let end =
            self.expect_token(&TokenKind::RightBracket, "expected ']' after array literal")?;

        Ok(Expression::new(
            ExpressionKind::ArrayLiteral(ArrayLiteralExpression { elements }),
            Span::cover(&start_span, &end.span),
        ))
    }

    fn parse_index_access_expression(
        &mut self,
        base_name: String,
        start_span: Span,
    ) -> ParseResult<Expression> {
        self.expect_token(&TokenKind::LeftBracket, "expected '[' for index access")?;
        let index = self.parse_expression()?;
        let end = self.expect_token(
            &TokenKind::RightBracket,
            "expected ']' after index expression",
        )?;

        if self.current_is(&TokenKind::LeftBracket) {
            return Err(ParseError::new(
                "nested index access is not supported in this phase",
                self.current().span.clone(),
            ));
        }

        Ok(Expression::new(
            ExpressionKind::IndexAccess(IndexAccessExpression {
                base_name,
                index: Box::new(index),
            }),
            Span::cover(&start_span, &end.span),
        ))
    }

    fn parse_type_name(&mut self) -> ParseResult<TypeName> {
        match self.advance().kind {
            TokenKind::I8Type => Ok(TypeName::I8),
            TokenKind::I16Type => Ok(TypeName::I16),
            TokenKind::I32Type => Ok(TypeName::I32),
            TokenKind::I64Type | TokenKind::IntType => Ok(TypeName::I64),
            TokenKind::U8Type => Ok(TypeName::U8),
            TokenKind::U16Type => Ok(TypeName::U16),
            TokenKind::U32Type => Ok(TypeName::U32),
            TokenKind::U64Type => Ok(TypeName::U64),
            TokenKind::F32Type => Ok(TypeName::F32),
            TokenKind::F64Type | TokenKind::FloatType => Ok(TypeName::F64),
            TokenKind::BoolType => Ok(TypeName::Bool),
            TokenKind::TextType => Ok(TypeName::Text),
            TokenKind::Identifier(name) if name == "Json" => Ok(TypeName::Json),
            TokenKind::CharType => Ok(TypeName::Char),
            TokenKind::VoidType => Ok(TypeName::Void),
            TokenKind::Identifier(name) if name == "Array" => {
                self.expect_token(&TokenKind::Less, "expected '<' after Array")?;
                let element_type = self.parse_type_name()?;
                self.expect_token(&TokenKind::Greater, "expected '>' after array element type")?;
                Ok(TypeName::Array(Box::new(element_type), None))
            }
            TokenKind::Identifier(name) if name == "Map" => {
                self.expect_token(&TokenKind::Less, "expected '<' after Map")?;
                let key_type = self.parse_type_name()?;
                self.expect_token(&TokenKind::Comma, "expected ',' after map key type")?;
                let value_type = self.parse_type_name()?;
                self.expect_token(&TokenKind::Greater, "expected '>' after map value type")?;
                Ok(TypeName::Map(Box::new(key_type), Box::new(value_type)))
            }
            TokenKind::Identifier(name) if name == "Set" => {
                self.expect_token(&TokenKind::Less, "expected '<' after Set")?;
                let element_type = self.parse_type_name()?;
                self.expect_token(&TokenKind::Greater, "expected '>' after set element type")?;
                Ok(TypeName::Set(Box::new(element_type)))
            }
            TokenKind::Identifier(name) => Ok(TypeName::Named(name)),
            token => Err(ParseError::new(
                format!("expected type name, found {token:?}"),
                self.previous().span.clone(),
            )),
        }
    }

    fn expect_identifier(&mut self) -> ParseResult<String> {
        Ok(self.expect_identifier_token()?.lexeme)
    }

    fn expect_identifier_token(&mut self) -> ParseResult<Token> {
        let token = self.advance();
        match token.kind {
            TokenKind::Identifier(_) => Ok(token),
            other => Err(ParseError::new(
                format!("expected identifier, found {other:?}"),
                token.span,
            )),
        }
    }

    fn expect_identifier_lexeme(&mut self, expected: &str) -> ParseResult<Token> {
        let token = self.advance();
        match &token.kind {
            TokenKind::Identifier(name) if name == expected => Ok(token),
            other => Err(ParseError::new(
                format!("expected identifier '{expected}', found {other:?}"),
                token.span,
            )),
        }
    }

    fn expect_token(&mut self, expected: &TokenKind, message: &str) -> ParseResult<Token> {
        if self.current_is(expected) {
            Ok(self.advance())
        } else {
            Err(ParseError::new(message, self.expected_token_span()))
        }
    }

    fn current(&self) -> &Token {
        self.tokens.get(self.position).unwrap_or_else(|| {
            self.tokens
                .last()
                .expect("parser token stream is never empty")
        })
    }

    fn previous(&self) -> &Token {
        if self.position == 0 {
            self.current()
        } else {
            &self.tokens[self.position - 1]
        }
    }

    fn current_is(&self, expected: &TokenKind) -> bool {
        mem::discriminant(&self.current().kind) == mem::discriminant(expected)
    }

    fn peek_is(&self, expected: &TokenKind) -> bool {
        self.tokens
            .get(self.position + 1)
            .map(|token| mem::discriminant(&token.kind) == mem::discriminant(expected))
            .unwrap_or(false)
    }

    fn peek_n_is(&self, offset: usize, expected: &TokenKind) -> bool {
        self.tokens
            .get(self.position + offset)
            .map(|token| mem::discriminant(&token.kind) == mem::discriminant(expected))
            .unwrap_or(false)
    }

    fn advance(&mut self) -> Token {
        let token = self.current().clone();
        if !self.is_at_end() {
            self.position += 1;
        }
        token
    }

    fn expected_token_span(&self) -> Span {
        let current = self.current();
        let previous = self.previous();

        if current.span.line > previous.span.line {
            Span::new(
                previous.span.file_path.clone(),
                previous.span.byte_end,
                previous.span.byte_end,
                previous.span.line,
                previous
                    .span
                    .column
                    .saturating_add(previous.lexeme.chars().count()),
            )
        } else {
            Span::new(
                current.span.file_path.clone(),
                current.span.byte_start,
                current.span.byte_start,
                current.span.line,
                current.span.column,
            )
        }
    }

    fn is_index_assignment_ahead(&self) -> bool {
        if !self.peek_is(&TokenKind::LeftBracket) {
            return false;
        }

        let mut cursor = self.position + 1;
        let mut bracket_depth = 0usize;

        while let Some(token) = self.tokens.get(cursor) {
            match token.kind {
                TokenKind::LeftBracket => bracket_depth += 1,
                TokenKind::RightBracket => {
                    if bracket_depth == 0 {
                        return false;
                    }

                    bracket_depth -= 1;
                    if bracket_depth == 0 {
                        return matches!(
                            self.tokens.get(cursor + 1).map(|token| &token.kind),
                            Some(TokenKind::Equal)
                        );
                    }
                }
                TokenKind::Eof => return false,
                _ => {}
            }

            cursor += 1;
        }

        false
    }

    fn is_schema_declaration_ahead(&self) -> bool {
        matches!(&self.current().kind, TokenKind::Identifier(name) if name == "schema")
            && self.peek_n_is(1, &TokenKind::Identifier(String::new()))
            && self.peek_n_is(2, &TokenKind::LeftBrace)
    }

    fn is_at_end(&self) -> bool {
        matches!(self.current().kind, TokenKind::Eof)
    }
}

pub fn parse_program(tokens: Vec<Token>) -> ParseResult<Program> {
    Parser::new(tokens).parse_program()
}

#[cfg(test)]
mod tests {
    use loz_lexer::tokenize_with_file_path;

    use super::parse_program;

    #[test]
    fn reports_missing_semicolon_with_line_column_and_caret() {
        let source = "func main() -> i32 {\n    print(\"hello\")\n    return 0;\n}\n";
        let error =
            parse_program(tokenize_with_file_path(source, "src/main.loz").unwrap()).unwrap_err();
        let rendered = error.diagnostic.render_with_source(Some(source));

        assert!(rendered.contains("src/main.loz:2:19"));
        assert!(rendered.contains("expected ';' after expression"));
        assert!(rendered.contains("print(\"hello\")"));
        assert!(rendered.contains("^"));
    }

    #[test]
    fn reports_missing_closing_paren_with_line_column_and_caret() {
        let source =
            "func main() -> i32 {\n    print(json.get_text(user, \"name\";\n    return 0;\n}\n";
        let error =
            parse_program(tokenize_with_file_path(source, "src/main.loz").unwrap()).unwrap_err();
        let rendered = error.diagnostic.render_with_source(Some(source));

        assert!(rendered.contains("src/main.loz:2:37"));
        assert!(rendered.contains("expected ')' after method arguments"));
        assert!(rendered.contains("print(json.get_text(user, \"name\";"));
        assert!(rendered.contains("^"));
    }
}
