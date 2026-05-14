use std::collections::HashMap;

use loz_ast::{Expression, FunctionDeclaration, Program, Statement};

use super::{ExecutionEnvironment, ExecutionError, ExecutionResult, RuntimeValue};

pub struct Interpreter {
    environment: ExecutionEnvironment,
    functions: HashMap<String, FunctionDeclaration>,
}

impl Interpreter {
    pub fn new() -> Self {
        Self {
            environment: ExecutionEnvironment::new(),
            functions: HashMap::new(),
        }
    }

    pub fn execute_program(&mut self, program: &Program) -> ExecutionResult<RuntimeValue> {
        self.collect_functions(program)?;

        for statement in &program.statements {
            if matches!(statement, Statement::FunctionDeclaration(_)) {
                continue;
            }

            if self.execute_statement(statement)?.is_some() {
                return Err(ExecutionError::new(
                    "return statement is not allowed at the top level",
                ));
            }
        }

        self.execute_main()
    }

    pub fn execute_main(&mut self) -> ExecutionResult<RuntimeValue> {
        self.call_function("main", Vec::new())
    }

    fn collect_functions(&mut self, program: &Program) -> ExecutionResult<()> {
        for statement in &program.statements {
            if let Statement::FunctionDeclaration(function) = statement {
                if self.functions.contains_key(&function.name) {
                    return Err(ExecutionError::new(format!(
                        "duplicate function '{}' at runtime",
                        function.name
                    )));
                }

                self.functions
                    .insert(function.name.clone(), function.clone());
            }
        }

        Ok(())
    }

    fn execute_statement(
        &mut self,
        statement: &Statement,
    ) -> ExecutionResult<Option<RuntimeValue>> {
        match statement {
            Statement::VariableDeclaration(declaration) => {
                let value = self.evaluate_expression(&declaration.value)?;
                self.environment
                    .define(declaration.name.clone(), value, declaration.is_mutable)?;
                Ok(None)
            }
            Statement::Assignment(assignment) => {
                let value = self.evaluate_expression(&assignment.value)?;
                self.environment.assign(&assignment.target, value)?;
                Ok(None)
            }
            Statement::Return(return_statement) => {
                let value = self.evaluate_expression(&return_statement.value)?;
                Ok(Some(value))
            }
            Statement::If(_) => Err(ExecutionError::new(
                "if statements are not supported in the tree-walk interpreter yet",
            )),
            Statement::While(_) => Err(ExecutionError::new(
                "while statements are not supported in the tree-walk interpreter yet",
            )),
            Statement::Expression(expression) => {
                self.evaluate_expression(expression)?;
                Ok(None)
            }
            Statement::FunctionDeclaration(_) => Ok(None),
        }
    }

    fn evaluate_expression(&mut self, expression: &Expression) -> ExecutionResult<RuntimeValue> {
        match expression {
            Expression::IntegerLiteral(value) => Ok(RuntimeValue::Int(*value)),
            Expression::FloatLiteral(value) => Ok(RuntimeValue::Float(*value)),
            Expression::BooleanLiteral(value) => Ok(RuntimeValue::Bool(*value)),
            Expression::StringLiteral(value) => Ok(RuntimeValue::Text(value.clone())),
            Expression::Identifier(name) => self.environment.get(name),
            Expression::Call(call) => {
                let mut arguments = Vec::with_capacity(call.arguments.len());
                for argument in &call.arguments {
                    arguments.push(self.evaluate_expression(argument)?);
                }

                if call.callee == "print" {
                    self.execute_print(arguments)
                } else {
                    self.call_function(&call.callee, arguments)
                }
            }
            Expression::Binary(binary) => {
                let left = self.evaluate_expression(&binary.left)?;
                let right = self.evaluate_expression(&binary.right)?;

                match (&left, &right, &binary.operator) {
                    (
                        RuntimeValue::Int(lhs),
                        RuntimeValue::Int(rhs),
                        loz_ast::BinaryOperator::Add,
                    ) => Ok(RuntimeValue::Int(lhs + rhs)),
                    (
                        RuntimeValue::Int(lhs),
                        RuntimeValue::Int(rhs),
                        loz_ast::BinaryOperator::Subtract,
                    ) => Ok(RuntimeValue::Int(lhs - rhs)),
                    (
                        RuntimeValue::Int(lhs),
                        RuntimeValue::Int(rhs),
                        loz_ast::BinaryOperator::Multiply,
                    ) => Ok(RuntimeValue::Int(lhs * rhs)),
                    (
                        RuntimeValue::Int(lhs),
                        RuntimeValue::Int(rhs),
                        loz_ast::BinaryOperator::Divide,
                    ) => Ok(RuntimeValue::Int(lhs / rhs)),
                    (
                        RuntimeValue::Float(lhs),
                        RuntimeValue::Float(rhs),
                        loz_ast::BinaryOperator::Add,
                    ) => Ok(RuntimeValue::Float(lhs + rhs)),
                    (
                        RuntimeValue::Float(lhs),
                        RuntimeValue::Float(rhs),
                        loz_ast::BinaryOperator::Subtract,
                    ) => Ok(RuntimeValue::Float(lhs - rhs)),
                    (
                        RuntimeValue::Float(lhs),
                        RuntimeValue::Float(rhs),
                        loz_ast::BinaryOperator::Multiply,
                    ) => Ok(RuntimeValue::Float(lhs * rhs)),
                    (
                        RuntimeValue::Float(lhs),
                        RuntimeValue::Float(rhs),
                        loz_ast::BinaryOperator::Divide,
                    ) => Ok(RuntimeValue::Float(lhs / rhs)),
                    _ => Err(ExecutionError::new(format!(
                        "unsupported runtime operands for binary expression: left={left:?}, right={right:?}"
                    ))),
                }
            }
        }
    }

    fn execute_print(&mut self, arguments: Vec<RuntimeValue>) -> ExecutionResult<RuntimeValue> {
        if arguments.len() != 1 {
            return Err(ExecutionError::new(
                "built-in function 'print' expects exactly one argument",
            ));
        }

        println!("{}", arguments[0]);
        Ok(RuntimeValue::Void)
    }

    fn call_function(
        &mut self,
        name: &str,
        arguments: Vec<RuntimeValue>,
    ) -> ExecutionResult<RuntimeValue> {
        let function = self
            .functions
            .get(name)
            .cloned()
            .ok_or_else(|| ExecutionError::new(format!("unknown function '{}'", name)))?;

        if function.parameters.len() != arguments.len() {
            return Err(ExecutionError::new(format!(
                "function '{}' expected {} arguments, found {}",
                name,
                function.parameters.len(),
                arguments.len()
            )));
        }

        self.environment.push_scope();
        for (parameter, argument) in function.parameters.iter().zip(arguments) {
            self.environment
                .define(parameter.name.clone(), argument, false)?;
        }

        let result = self.execute_block(&function.body);
        self.environment.pop_scope();

        match result? {
            Some(value) => Ok(value),
            None => Ok(RuntimeValue::Void),
        }
    }

    fn execute_block(&mut self, statements: &[Statement]) -> ExecutionResult<Option<RuntimeValue>> {
        for statement in statements {
            if let Some(value) = self.execute_statement(statement)? {
                return Ok(Some(value));
            }
        }

        Ok(None)
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn execute(program: &Program) -> ExecutionResult<RuntimeValue> {
    Interpreter::new().execute_program(program)
}
