use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use loz_ast::{
    AgentDeclaration, AgentTaskDeclaration, ArrayLiteralExpression, AssignmentStatement,
    AsyncTaskDeclaration, AwaitExpression, BinaryExpression, BinaryOperator, CallExpression,
    CastExpression, DereferenceExpression, Expression, ExpressionKind, ForStatement,
    FunctionDeclaration, IfStatement, ImplBlock, IndexAccessExpression, MethodCallExpression,
    Program, ReturnStatement, Span, Statement, ToolDeclaration, VariableDeclaration,
    WhileStatement,
};

pub type OptimizeResult<T> = Result<T, OptimizerError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizerError {
    message: String,
}

impl OptimizerError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for OptimizerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for OptimizerError {}

#[derive(Debug, Clone, PartialEq)]
enum ConstantValue {
    Integer(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

impl ConstantValue {
    fn from_expression(expression: &Expression) -> Option<Self> {
        match &expression.kind {
            ExpressionKind::IntegerLiteral(value) => Some(Self::Integer(*value)),
            ExpressionKind::FloatLiteral(value) => Some(Self::Float(*value)),
            ExpressionKind::BooleanLiteral(value) => Some(Self::Bool(*value)),
            ExpressionKind::StringLiteral(value) => Some(Self::Text(value.clone())),
            _ => None,
        }
    }

    fn to_expression(&self, span: &Span) -> Expression {
        match self {
            Self::Integer(value) => {
                Expression::new(ExpressionKind::IntegerLiteral(*value), span.clone())
            }
            Self::Float(value) => {
                Expression::new(ExpressionKind::FloatLiteral(*value), span.clone())
            }
            Self::Bool(value) => {
                Expression::new(ExpressionKind::BooleanLiteral(*value), span.clone())
            }
            Self::Text(value) => {
                Expression::new(ExpressionKind::StringLiteral(value.clone()), span.clone())
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ConstantEnvironment {
    scopes: Vec<HashMap<String, Option<ConstantValue>>>,
}

impl ConstantEnvironment {
    fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define(&mut self, name: String, value: Option<ConstantValue>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, value);
        }
    }

    fn lookup(&self, name: &str) -> Option<&ConstantValue> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return value.as_ref();
            }
        }

        None
    }

    fn invalidate(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(value) = scope.get_mut(name) {
                *value = None;
                return;
            }
        }
    }
}

pub fn optimize_program(program: &Program) -> OptimizeResult<Program> {
    let mut environment = ConstantEnvironment::new();
    Ok(Program {
        statements: optimize_statement_list(&program.statements, &mut environment)?,
    })
}

fn optimize_statement_list(
    statements: &[Statement],
    environment: &mut ConstantEnvironment,
) -> OptimizeResult<Vec<Statement>> {
    let mut optimized = Vec::new();
    for statement in statements {
        optimized.extend(optimize_statement(statement, environment)?);
    }

    Ok(optimized)
}

fn optimize_statement(
    statement: &Statement,
    environment: &mut ConstantEnvironment,
) -> OptimizeResult<Vec<Statement>> {
    match statement {
        Statement::ModuleDeclaration(_)
        | Statement::ImportDeclaration(_)
        | Statement::Break(_)
        | Statement::Continue(_)
        | Statement::WorkflowDeclaration(_)
        | Statement::StructDeclaration(_)
        | Statement::SchemaDeclaration(_) => Ok(vec![statement.clone()]),
        Statement::VariableDeclaration(declaration) => {
            let value = optimize_expression(&declaration.value, environment)?;
            let optimized = VariableDeclaration {
                value: value.clone(),
                ..declaration.clone()
            };

            let constant_value = if optimized.is_mutable {
                None
            } else {
                ConstantValue::from_expression(&value)
            };
            environment.define(optimized.name.clone(), constant_value);

            Ok(vec![Statement::VariableDeclaration(optimized)])
        }
        Statement::FunctionDeclaration(function) => Ok(vec![Statement::FunctionDeclaration(
            optimize_function_declaration(function)?,
        )]),
        Statement::AsyncTaskDeclaration(task) => Ok(vec![Statement::AsyncTaskDeclaration(
            optimize_async_task_declaration(task)?,
        )]),
        Statement::ToolDeclaration(tool) => Ok(vec![Statement::ToolDeclaration(
            optimize_tool_declaration(tool)?,
        )]),
        Statement::AgentDeclaration(agent) => Ok(vec![Statement::AgentDeclaration(
            optimize_agent_declaration(agent)?,
        )]),
        Statement::ImplBlock(impl_block) => {
            Ok(vec![Statement::ImplBlock(optimize_impl_block(impl_block)?)])
        }
        Statement::If(if_statement) => optimize_if_statement(if_statement, environment),
        Statement::While(while_statement) => Ok(vec![Statement::While(optimize_while_statement(
            while_statement,
            environment,
        )?)]),
        Statement::For(for_statement) => Ok(vec![Statement::For(optimize_for_statement(
            for_statement,
            environment,
        )?)]),
        Statement::Return(return_statement) => Ok(vec![Statement::Return(ReturnStatement {
            value: optimize_expression(&return_statement.value, environment)?,
            span: return_statement.span.clone(),
        })]),
        Statement::Assignment(assignment) => {
            if let loz_ast::AssignmentTarget::Identifier(name) = &assignment.target {
                environment.invalidate(name);
            }

            Ok(vec![Statement::Assignment(AssignmentStatement {
                target: assignment.target.clone(),
                value: optimize_expression(&assignment.value, environment)?,
                span: assignment.span.clone(),
            })])
        }
        Statement::Expression(expression) => Ok(vec![Statement::Expression(optimize_expression(
            expression,
            environment,
        )?)]),
    }
}

fn optimize_function_declaration(
    function: &FunctionDeclaration,
) -> OptimizeResult<FunctionDeclaration> {
    Ok(FunctionDeclaration {
        body: optimize_callable_body(&function.body)?,
        ..function.clone()
    })
}

fn optimize_tool_declaration(tool: &ToolDeclaration) -> OptimizeResult<ToolDeclaration> {
    Ok(ToolDeclaration {
        body: optimize_callable_body(&tool.body)?,
        ..tool.clone()
    })
}

fn optimize_async_task_declaration(
    task: &AsyncTaskDeclaration,
) -> OptimizeResult<AsyncTaskDeclaration> {
    Ok(AsyncTaskDeclaration {
        body: optimize_callable_body(&task.body)?,
        ..task.clone()
    })
}

fn optimize_agent_declaration(agent: &AgentDeclaration) -> OptimizeResult<AgentDeclaration> {
    let mut environment = ConstantEnvironment::new();
    let model = agent
        .model
        .as_ref()
        .map(|expression| optimize_expression(expression, &mut environment))
        .transpose()?;
    let tools = agent
        .tools
        .as_ref()
        .map(|expression| optimize_expression(expression, &mut environment))
        .transpose()?;
    let tasks = agent
        .tasks
        .iter()
        .map(optimize_agent_task_declaration)
        .collect::<OptimizeResult<Vec<_>>>()?;

    Ok(AgentDeclaration {
        model,
        tools,
        tasks,
        ..agent.clone()
    })
}

fn optimize_agent_task_declaration(
    task: &AgentTaskDeclaration,
) -> OptimizeResult<AgentTaskDeclaration> {
    Ok(AgentTaskDeclaration {
        body: optimize_callable_body(&task.body)?,
        ..task.clone()
    })
}

fn optimize_impl_block(impl_block: &ImplBlock) -> OptimizeResult<ImplBlock> {
    let methods = impl_block
        .methods
        .iter()
        .map(optimize_function_declaration)
        .collect::<OptimizeResult<Vec<_>>>()?;

    Ok(ImplBlock {
        methods,
        ..impl_block.clone()
    })
}

fn optimize_callable_body(statements: &[Statement]) -> OptimizeResult<Vec<Statement>> {
    let mut environment = ConstantEnvironment::new();
    optimize_statement_list(statements, &mut environment)
}

fn optimize_if_statement(
    if_statement: &IfStatement,
    environment: &mut ConstantEnvironment,
) -> OptimizeResult<Vec<Statement>> {
    let condition = optimize_expression(&if_statement.condition, environment)?;

    if let ExpressionKind::BooleanLiteral(condition_value) = condition.kind {
        if condition_value {
            let then_branch = optimize_branch(&if_statement.then_branch, environment)?;
            return Ok(vec![Statement::If(IfStatement {
                condition: Expression::new(
                    ExpressionKind::BooleanLiteral(true),
                    condition.span.clone(),
                ),
                then_branch,
                else_branch: None,
                span: if_statement.span.clone(),
            })]);
        }

        if let Some(else_branch) = &if_statement.else_branch {
            let optimized_else_branch = optimize_branch(else_branch, environment)?;
            return Ok(vec![Statement::If(IfStatement {
                condition: Expression::new(
                    ExpressionKind::BooleanLiteral(true),
                    condition.span.clone(),
                ),
                then_branch: optimized_else_branch,
                else_branch: None,
                span: if_statement.span.clone(),
            })]);
        }

        return Ok(Vec::new());
    }

    let then_branch = optimize_branch(&if_statement.then_branch, environment)?;
    let else_branch = if_statement
        .else_branch
        .as_ref()
        .map(|branch| optimize_branch(branch, environment))
        .transpose()?;

    Ok(vec![Statement::If(IfStatement {
        condition,
        then_branch,
        else_branch,
        span: if_statement.span.clone(),
    })])
}

fn optimize_branch(
    statements: &[Statement],
    environment: &ConstantEnvironment,
) -> OptimizeResult<Vec<Statement>> {
    let mut branch_environment = environment.clone();
    branch_environment.push_scope();
    let optimized = optimize_statement_list(statements, &mut branch_environment)?;
    branch_environment.pop_scope();
    Ok(optimized)
}

fn optimize_while_statement(
    while_statement: &WhileStatement,
    environment: &ConstantEnvironment,
) -> OptimizeResult<WhileStatement> {
    let mut loop_environment = environment.clone();
    loop_environment.push_scope();
    let body = optimize_statement_list(&while_statement.body, &mut loop_environment)?;
    loop_environment.pop_scope();

    Ok(WhileStatement {
        condition: optimize_expression(&while_statement.condition, &mut environment.clone())?,
        body,
        span: while_statement.span.clone(),
    })
}

fn optimize_for_statement(
    for_statement: &ForStatement,
    environment: &ConstantEnvironment,
) -> OptimizeResult<ForStatement> {
    let mut loop_environment = environment.clone();
    loop_environment.push_scope();
    loop_environment.define(for_statement.variable_name.clone(), None);
    let body = optimize_statement_list(&for_statement.body, &mut loop_environment)?;
    loop_environment.pop_scope();

    Ok(ForStatement {
        iterable: optimize_expression(&for_statement.iterable, &mut environment.clone())?,
        body,
        ..for_statement.clone()
    })
}

fn optimize_expression(
    expression: &Expression,
    environment: &mut ConstantEnvironment,
) -> OptimizeResult<Expression> {
    match &expression.kind {
        ExpressionKind::IntegerLiteral(_)
        | ExpressionKind::FloatLiteral(_)
        | ExpressionKind::BooleanLiteral(_)
        | ExpressionKind::StringLiteral(_) => Ok(expression.clone()),
        ExpressionKind::Await(await_expression) => Ok(Expression::new(
            ExpressionKind::Await(AwaitExpression {
                expression: Box::new(optimize_expression(
                    &await_expression.expression,
                    environment,
                )?),
            }),
            expression.span.clone(),
        )),
        ExpressionKind::Identifier(name) => Ok(environment
            .lookup(name)
            .map(|value| value.to_expression(&expression.span))
            .unwrap_or_else(|| expression.clone())),
        ExpressionKind::Reference(_)
        | ExpressionKind::FieldAccess(_)
        | ExpressionKind::MethodCall(MethodCallExpression {
            base_name: _,
            method_name: _,
            arguments: _,
        }) => optimize_non_literal_expression(expression, environment),
        ExpressionKind::Dereference(dereference_expression) => Ok(Expression::new(
            ExpressionKind::Dereference(DereferenceExpression {
                value: Box::new(optimize_expression(
                    &dereference_expression.value,
                    environment,
                )?),
            }),
            expression.span.clone(),
        )),
        ExpressionKind::Cast(cast_expression) => Ok(Expression::new(
            ExpressionKind::Cast(CastExpression {
                target_type: cast_expression.target_type.clone(),
                value: Box::new(optimize_expression(&cast_expression.value, environment)?),
            }),
            expression.span.clone(),
        )),
        ExpressionKind::ArrayLiteral(array_literal) => Ok(Expression::new(
            ExpressionKind::ArrayLiteral(ArrayLiteralExpression {
                elements: array_literal
                    .elements
                    .iter()
                    .map(|element| optimize_expression(element, environment))
                    .collect::<OptimizeResult<Vec<_>>>()?,
            }),
            expression.span.clone(),
        )),
        ExpressionKind::IndexAccess(index_access) => Ok(Expression::new(
            ExpressionKind::IndexAccess(IndexAccessExpression {
                base_name: index_access.base_name.clone(),
                index: Box::new(optimize_expression(&index_access.index, environment)?),
            }),
            expression.span.clone(),
        )),
        ExpressionKind::Call(call_expression) => Ok(Expression::new(
            ExpressionKind::Call(CallExpression {
                callee: call_expression.callee.clone(),
                arguments: call_expression
                    .arguments
                    .iter()
                    .map(|argument| optimize_expression(argument, environment))
                    .collect::<OptimizeResult<Vec<_>>>()?,
            }),
            expression.span.clone(),
        )),
        ExpressionKind::Binary(binary) => {
            let left = optimize_expression(&binary.left, environment)?;
            let right = optimize_expression(&binary.right, environment)?;
            if let Some(folded) = fold_binary_expression(&left, &binary.operator, &right)? {
                Ok(folded)
            } else {
                Ok(Expression::new(
                    ExpressionKind::Binary(BinaryExpression {
                        left: Box::new(left),
                        operator: binary.operator.clone(),
                        right: Box::new(right),
                    }),
                    expression.span.clone(),
                ))
            }
        }
    }
}

fn optimize_non_literal_expression(
    expression: &Expression,
    environment: &mut ConstantEnvironment,
) -> OptimizeResult<Expression> {
    match &expression.kind {
        ExpressionKind::Reference(_) | ExpressionKind::FieldAccess(_) => Ok(expression.clone()),
        ExpressionKind::MethodCall(method_call) => Ok(Expression::new(
            ExpressionKind::MethodCall(MethodCallExpression {
                base_name: method_call.base_name.clone(),
                method_name: method_call.method_name.clone(),
                arguments: method_call
                    .arguments
                    .iter()
                    .map(|argument| optimize_expression(argument, environment))
                    .collect::<OptimizeResult<Vec<_>>>()?,
            }),
            expression.span.clone(),
        )),
        _ => Ok(expression.clone()),
    }
}

fn fold_binary_expression(
    left: &Expression,
    operator: &BinaryOperator,
    right: &Expression,
) -> OptimizeResult<Option<Expression>> {
    let span = Span::cover(&left.span, &right.span);

    match (&left.kind, &right.kind, operator) {
        (
            ExpressionKind::IntegerLiteral(left),
            ExpressionKind::IntegerLiteral(right),
            BinaryOperator::Add,
        ) => Ok(Some(Expression::new(
            ExpressionKind::IntegerLiteral(left + right),
            span,
        ))),
        (
            ExpressionKind::IntegerLiteral(left),
            ExpressionKind::IntegerLiteral(right),
            BinaryOperator::Subtract,
        ) => Ok(Some(Expression::new(
            ExpressionKind::IntegerLiteral(left - right),
            span,
        ))),
        (
            ExpressionKind::IntegerLiteral(left),
            ExpressionKind::IntegerLiteral(right),
            BinaryOperator::Multiply,
        ) => Ok(Some(Expression::new(
            ExpressionKind::IntegerLiteral(left * right),
            span,
        ))),
        (
            ExpressionKind::IntegerLiteral(_),
            ExpressionKind::IntegerLiteral(0),
            BinaryOperator::Divide,
        ) => Err(OptimizerError::new(
            "compile-time constant division by zero",
        )),
        (
            ExpressionKind::IntegerLiteral(left),
            ExpressionKind::IntegerLiteral(right),
            BinaryOperator::Divide,
        ) => Ok(Some(Expression::new(
            ExpressionKind::IntegerLiteral(left / right),
            span,
        ))),
        (
            ExpressionKind::FloatLiteral(_),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::Divide,
        ) if *right == 0.0 => Err(OptimizerError::new(
            "compile-time constant division by zero",
        )),
        (
            ExpressionKind::FloatLiteral(left),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::Add,
        ) => Ok(Some(Expression::new(
            ExpressionKind::FloatLiteral(left + right),
            span,
        ))),
        (
            ExpressionKind::FloatLiteral(left),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::Subtract,
        ) => Ok(Some(Expression::new(
            ExpressionKind::FloatLiteral(left - right),
            span,
        ))),
        (
            ExpressionKind::FloatLiteral(left),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::Multiply,
        ) => Ok(Some(Expression::new(
            ExpressionKind::FloatLiteral(left * right),
            span,
        ))),
        (
            ExpressionKind::FloatLiteral(left),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::Divide,
        ) => Ok(Some(Expression::new(
            ExpressionKind::FloatLiteral(left / right),
            span,
        ))),
        (
            ExpressionKind::IntegerLiteral(left),
            ExpressionKind::IntegerLiteral(right),
            BinaryOperator::Greater,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left > right),
            span,
        ))),
        (
            ExpressionKind::IntegerLiteral(left),
            ExpressionKind::IntegerLiteral(right),
            BinaryOperator::Less,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left < right),
            span,
        ))),
        (
            ExpressionKind::IntegerLiteral(left),
            ExpressionKind::IntegerLiteral(right),
            BinaryOperator::GreaterEqual,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left >= right),
            span,
        ))),
        (
            ExpressionKind::IntegerLiteral(left),
            ExpressionKind::IntegerLiteral(right),
            BinaryOperator::LessEqual,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left <= right),
            span,
        ))),
        (
            ExpressionKind::IntegerLiteral(left),
            ExpressionKind::IntegerLiteral(right),
            BinaryOperator::Equal,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left == right),
            span,
        ))),
        (
            ExpressionKind::IntegerLiteral(left),
            ExpressionKind::IntegerLiteral(right),
            BinaryOperator::NotEqual,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left != right),
            span,
        ))),
        (
            ExpressionKind::FloatLiteral(left),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::Greater,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left > right),
            span,
        ))),
        (
            ExpressionKind::FloatLiteral(left),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::Less,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left < right),
            span,
        ))),
        (
            ExpressionKind::FloatLiteral(left),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::GreaterEqual,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left >= right),
            span,
        ))),
        (
            ExpressionKind::FloatLiteral(left),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::LessEqual,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left <= right),
            span,
        ))),
        (
            ExpressionKind::FloatLiteral(left),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::Equal,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left == right),
            span,
        ))),
        (
            ExpressionKind::FloatLiteral(left),
            ExpressionKind::FloatLiteral(right),
            BinaryOperator::NotEqual,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left != right),
            span,
        ))),
        (
            ExpressionKind::BooleanLiteral(left),
            ExpressionKind::BooleanLiteral(right),
            BinaryOperator::Equal,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left == right),
            span,
        ))),
        (
            ExpressionKind::BooleanLiteral(left),
            ExpressionKind::BooleanLiteral(right),
            BinaryOperator::NotEqual,
        ) => Ok(Some(Expression::new(
            ExpressionKind::BooleanLiteral(left != right),
            span,
        ))),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use loz_ast::{BinaryOperator, ExpressionKind, Statement};
    use loz_codegen::{Interpreter, RuntimeValue, generate_llvm_ir};
    use loz_lexer::tokenize;
    use loz_parser::parse_program;
    use loz_semantic::analyze;

    use super::{OptimizerError, optimize_program};

    fn parse_checked_program(source: &str) -> loz_ast::Program {
        let program = parse_program(tokenize(source).unwrap()).unwrap();
        analyze(&program).unwrap();
        program
    }

    fn optimize_checked_program(source: &str) -> loz_ast::Program {
        optimize_program(&parse_checked_program(source)).unwrap()
    }

    #[test]
    fn folds_integer_arithmetic() {
        let program = optimize_checked_program(
            r#"func main() -> Int {
    const x: Int = 15 + 10;
    return x;
}
"#,
        );

        let Statement::FunctionDeclaration(function) = &program.statements[0] else {
            panic!("expected function declaration");
        };
        let Statement::VariableDeclaration(variable) = &function.body[0] else {
            panic!("expected variable declaration");
        };
        assert_eq!(variable.value.kind, ExpressionKind::IntegerLiteral(25));
    }

    #[test]
    fn folds_float_arithmetic_and_comparisons() {
        let program = optimize_checked_program(
            r#"func main() -> Bool {
    const ok: Bool = 10.0 / 2.0 == 5.0;
    return ok;
}
"#,
        );

        let Statement::FunctionDeclaration(function) = &program.statements[0] else {
            panic!("expected function declaration");
        };
        let Statement::VariableDeclaration(variable) = &function.body[0] else {
            panic!("expected variable declaration");
        };
        assert_eq!(variable.value.kind, ExpressionKind::BooleanLiteral(true));
    }

    #[test]
    fn folds_boolean_comparisons() {
        let program = optimize_checked_program(
            r#"func main() -> Bool {
    const ok: Bool = true != false;
    return ok;
}
"#,
        );

        let Statement::FunctionDeclaration(function) = &program.statements[0] else {
            panic!("expected function declaration");
        };
        let Statement::VariableDeclaration(variable) = &function.body[0] else {
            panic!("expected variable declaration");
        };
        assert_eq!(variable.value.kind, ExpressionKind::BooleanLiteral(true));
    }

    #[test]
    fn propagates_local_const_values() {
        let program = optimize_checked_program(
            r#"func main() -> Int {
    const x: Int = 15 + 10;
    const y: Int = x * 2;
    return y;
}
"#,
        );

        let Statement::FunctionDeclaration(function) = &program.statements[0] else {
            panic!("expected function declaration");
        };
        let Statement::VariableDeclaration(first) = &function.body[0] else {
            panic!("expected first variable declaration");
        };
        let Statement::VariableDeclaration(second) = &function.body[1] else {
            panic!("expected second variable declaration");
        };
        let Statement::Return(return_statement) = &function.body[2] else {
            panic!("expected return");
        };

        assert_eq!(first.value.kind, ExpressionKind::IntegerLiteral(25));
        assert_eq!(second.value.kind, ExpressionKind::IntegerLiteral(50));
        assert_eq!(
            return_statement.value.kind,
            ExpressionKind::IntegerLiteral(50)
        );
    }

    #[test]
    fn does_not_propagate_mutable_variables() {
        let program = optimize_checked_program(
            r#"func main() -> Int {
    mut x: Int = 10;
    const y: Int = x + 5;
    return y;
}
"#,
        );

        let Statement::FunctionDeclaration(function) = &program.statements[0] else {
            panic!("expected function declaration");
        };
        let Statement::VariableDeclaration(variable) = &function.body[1] else {
            panic!("expected variable declaration");
        };

        let ExpressionKind::Binary(binary) = &variable.value.kind else {
            panic!("expected binary expression");
        };
        assert_eq!(
            binary.left.kind,
            ExpressionKind::Identifier("x".to_string())
        );
        assert_eq!(binary.operator, BinaryOperator::Add);
        assert_eq!(binary.right.kind, ExpressionKind::IntegerLiteral(5));
    }

    #[test]
    fn does_not_fold_function_calls_or_runtime_side_effects() {
        let program = optimize_checked_program(
            r#"func add(a: Int, b: Int) -> Int {
    return a + b;
}

func main() -> Text {
    const x: Int = add(10, 20);
    const y: Text = llm.ask("hello");
    const z: Json = python.call("tools.analyze", json.parse("{\"text\":\"hello\"}"));
    const name: Text = io.read_line();
    return name;
}
"#,
        );

        let Statement::FunctionDeclaration(function) = &program.statements[1] else {
            panic!("expected main function declaration");
        };
        let Statement::VariableDeclaration(x) = &function.body[0] else {
            panic!("expected x declaration");
        };
        let Statement::VariableDeclaration(y) = &function.body[1] else {
            panic!("expected y declaration");
        };
        let Statement::VariableDeclaration(z) = &function.body[2] else {
            panic!("expected z declaration");
        };
        let Statement::VariableDeclaration(name) = &function.body[3] else {
            panic!("expected name declaration");
        };

        assert!(matches!(x.value.kind, ExpressionKind::Call(_)));
        assert!(matches!(y.value.kind, ExpressionKind::MethodCall(_)));
        assert!(matches!(z.value.kind, ExpressionKind::MethodCall(_)));
        assert!(matches!(name.value.kind, ExpressionKind::MethodCall(_)));
    }

    #[test]
    fn eliminates_dead_if_branches() {
        let program = optimize_checked_program(
            r#"func main() -> Int {
    if 10 > 5 {
        const x: Int = 10 + 5;
        print(x);
    } else {
        print(0);
    }

    return 0;
}
"#,
        );

        let Statement::FunctionDeclaration(function) = &program.statements[0] else {
            panic!("expected function declaration");
        };
        let Statement::If(if_statement) = &function.body[0] else {
            panic!("expected if statement");
        };

        assert_eq!(
            if_statement.condition.kind,
            ExpressionKind::BooleanLiteral(true)
        );
        assert!(if_statement.else_branch.is_none());
        let Statement::VariableDeclaration(variable) = &if_statement.then_branch[0] else {
            panic!("expected optimized then branch");
        };
        assert_eq!(variable.value.kind, ExpressionKind::IntegerLiteral(15));
    }

    #[test]
    fn respects_nested_scopes() {
        let program = optimize_checked_program(
            r#"func main() -> Int {
    const x: Int = 10;
    if true {
        const x: Int = 20;
        print(x);
    }

    return x;
}
"#,
        );

        let Statement::FunctionDeclaration(function) = &program.statements[0] else {
            panic!("expected function declaration");
        };
        let Statement::If(if_statement) = &function.body[1] else {
            panic!("expected if statement");
        };
        let Statement::VariableDeclaration(inner) = &if_statement.then_branch[0] else {
            panic!("expected inner variable declaration");
        };
        let Statement::Return(return_statement) = &function.body[2] else {
            panic!("expected return");
        };

        assert_eq!(inner.value.kind, ExpressionKind::IntegerLiteral(20));
        assert_eq!(
            return_statement.value.kind,
            ExpressionKind::IntegerLiteral(10)
        );
    }

    #[test]
    fn reports_compile_time_division_by_zero() {
        let program = parse_checked_program(
            r#"func main() -> Int {
    const x: Int = 10 / 0;
    return x;
}
"#,
        );

        let error = optimize_program(&program).unwrap_err();
        assert_eq!(
            error,
            OptimizerError::new("compile-time constant division by zero")
        );
    }

    #[test]
    fn optimized_program_executes_in_interpreter() {
        let program = optimize_checked_program(
            r#"func main() -> Int {
    const x: Int = 15 + 10;
    const y: Int = x * 2;
    return y;
}
"#,
        );

        let result = Interpreter::new().execute_program(&program).unwrap();
        assert_eq!(result, RuntimeValue::Int(50));
    }

    #[test]
    fn optimized_program_emits_folded_llvm_ir() {
        let program = optimize_checked_program(
            r#"func main() -> Int {
    const x: Int = 15 + 10;
    print(x);
    return 0;
}
"#,
        );

        let ir = generate_llvm_ir(&program).unwrap();
        assert!(ir.contains("store i64 25"));
        assert!(!ir.contains("add i64 15, 10"));
    }

    #[test]
    fn optimized_program_eliminates_constant_if_in_llvm_ir() {
        let program = optimize_checked_program(
            r#"func main() -> Int {
    if 10 > 5 {
        print("yes");
    } else {
        print("no");
    }

    return 0;
}
"#,
        );

        let ir = generate_llvm_ir(&program).unwrap();
        assert!(!ir.contains("br i1 true"));
        assert!(!ir.contains("c\"no\\00\""));
    }

    #[test]
    fn optimized_workflow_program_still_executes() {
        let program = optimize_checked_program(
            r#"func prepare() -> Text {
    const message: Text = "prepared";
    return message;
}

workflow Onboarding {
    step prepare;
}

func main() -> Int {
    return 0;
}
"#,
        );

        let outcomes = Interpreter::new()
            .execute_workflow(&program, "Onboarding")
            .unwrap();

        assert_eq!(
            outcomes[0].result,
            Some(RuntimeValue::Text("prepared".to_string()))
        );
    }

    #[test]
    fn optimizer_preserves_async_tasks_and_await() {
        let program = optimize_checked_program(
            r#"async task get_name() -> Text {
    return "Ahmed";
}

func main() -> Text {
    return await get_name();
}
"#,
        );

        let Statement::AsyncTaskDeclaration(task) = &program.statements[0] else {
            panic!("expected async task declaration");
        };
        let Statement::FunctionDeclaration(function) = &program.statements[1] else {
            panic!("expected function declaration");
        };
        let Statement::Return(return_statement) = &function.body[0] else {
            panic!("expected return statement");
        };

        assert_eq!(task.name, "get_name");
        assert!(matches!(
            return_statement.value.kind,
            ExpressionKind::Await(_)
        ));
    }

    #[test]
    fn optimizer_does_not_constant_evaluate_await() {
        let program = optimize_checked_program(
            r#"async task get_value() -> Int {
    return 10;
}

func main() -> Int {
    const x: Int = await get_value();
    return x;
}
"#,
        );

        let Statement::FunctionDeclaration(function) = &program.statements[1] else {
            panic!("expected function declaration");
        };
        let Statement::VariableDeclaration(variable) = &function.body[0] else {
            panic!("expected variable declaration");
        };

        assert!(matches!(variable.value.kind, ExpressionKind::Await(_)));
    }
}
