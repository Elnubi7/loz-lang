use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use loz_ast::{
    AgentDeclaration, AssignmentTarget, AwaitExpression, Expression, ExpressionKind,
    FunctionDeclaration, ImplBlock, Program, SchemaDeclaration, Statement, StructDeclaration,
    ToolDeclaration, TypeName, WorkflowDeclaration, WorkflowTarget,
};

use super::{
    ExecutionEnvironment, ExecutionError, ExecutionResult, MapKey, RuntimeMap, RuntimeSet,
    RuntimeValue,
};
use serde_json::Value as JsonValue;

pub struct Interpreter {
    environment: ExecutionEnvironment,
    functions: HashMap<String, FunctionDeclaration>,
    module_functions: HashMap<(String, String), FunctionDeclaration>,
    async_tasks: HashMap<String, FunctionDeclaration>,
    tools: HashMap<String, ToolDeclaration>,
    agent_tasks: HashMap<(String, String), FunctionDeclaration>,
    workflows: HashMap<String, WorkflowDeclaration>,
    methods: HashMap<(String, String), FunctionDeclaration>,
    structs: HashMap<String, StructDeclaration>,
    schemas: HashMap<String, SchemaDeclaration>,
    input_lines: VecDeque<String>,
}

enum StatementControl {
    None,
    Return(RuntimeValue),
    Break,
    Continue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowStepOutcome {
    pub step_name: String,
    pub result: Option<RuntimeValue>,
}

impl Interpreter {
    pub fn new() -> Self {
        Self {
            environment: ExecutionEnvironment::new(),
            functions: HashMap::new(),
            module_functions: HashMap::new(),
            async_tasks: HashMap::new(),
            tools: HashMap::new(),
            agent_tasks: HashMap::new(),
            workflows: HashMap::new(),
            methods: HashMap::new(),
            structs: HashMap::new(),
            schemas: HashMap::new(),
            input_lines: VecDeque::new(),
        }
    }

    pub fn with_input_lines<I, S>(lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            input_lines: lines.into_iter().map(Into::into).collect(),
            ..Self::new()
        }
    }

    pub fn execute_program(&mut self, program: &Program) -> ExecutionResult<RuntimeValue> {
        self.initialize_program(program)?;
        self.execute_top_level_statements(program)?;
        self.execute_main()
    }

    pub fn execute_agent_task(
        &mut self,
        program: &Program,
        agent_name: &str,
        task_name: &str,
        arguments: Vec<RuntimeValue>,
    ) -> ExecutionResult<RuntimeValue> {
        self.initialize_program(program)?;
        self.execute_top_level_statements(program)?;

        let task = self
            .agent_tasks
            .get(&(agent_name.to_string(), task_name.to_string()))
            .cloned()
            .ok_or_else(|| {
                ExecutionError::new(format!(
                    "unknown agent task '{}.{}' at runtime",
                    agent_name, task_name
                ))
            })?;

        self.call_callable(&format!("{agent_name}.{task_name}"), &task, arguments)
    }

    pub fn execute_workflow(
        &mut self,
        program: &Program,
        workflow_name: &str,
    ) -> ExecutionResult<Vec<WorkflowStepOutcome>> {
        self.initialize_program(program)?;
        self.execute_top_level_statements(program)?;

        let workflow = self.workflows.get(workflow_name).cloned().ok_or_else(|| {
            ExecutionError::new(format!("unknown workflow '{}' at runtime", workflow_name))
        })?;

        let mut outcomes = Vec::new();
        for step in &workflow.steps {
            let result = self.execute_workflow_step(step)?;
            outcomes.push(WorkflowStepOutcome {
                step_name: step.name.clone(),
                result: (!matches!(result, RuntimeValue::Void)).then_some(result),
            });
        }

        Ok(outcomes)
    }

    fn initialize_program(&mut self, program: &Program) -> ExecutionResult<()> {
        self.collect_structs(program)?;
        self.collect_schemas(program)?;
        self.collect_functions(program)?;
        self.collect_async_tasks(program)?;
        self.collect_tools(program)?;
        self.collect_agent_tasks(program)?;
        self.collect_workflows(program)?;
        self.collect_methods(program)?;
        Ok(())
    }

    fn execute_top_level_statements(&mut self, program: &Program) -> ExecutionResult<()> {
        for statement in &program.statements {
            if matches!(
                statement,
                Statement::ModuleDeclaration(_)
                    | Statement::ImportDeclaration(_)
                    | Statement::FunctionDeclaration(_)
                    | Statement::AsyncTaskDeclaration(_)
                    | Statement::ToolDeclaration(_)
                    | Statement::AgentDeclaration(_)
                    | Statement::WorkflowDeclaration(_)
                    | Statement::StructDeclaration(_)
                    | Statement::SchemaDeclaration(_)
                    | Statement::ImplBlock(_)
            ) {
                continue;
            }

            match self.execute_statement(statement)? {
                StatementControl::None => {}
                StatementControl::Return(_) => {
                    return Err(ExecutionError::new(
                        "return statement is not allowed at the top level",
                    ));
                }
                StatementControl::Break => {
                    return Err(ExecutionError::new(
                        "break statement is not allowed at the top level",
                    ));
                }
                StatementControl::Continue => {
                    return Err(ExecutionError::new(
                        "continue statement is not allowed at the top level",
                    ));
                }
            }
        }

        Ok(())
    }

    pub fn execute_main(&mut self) -> ExecutionResult<RuntimeValue> {
        self.call_function("main", Vec::new())
    }

    fn collect_structs(&mut self, program: &Program) -> ExecutionResult<()> {
        for statement in &program.statements {
            if let Statement::StructDeclaration(struct_declaration) = statement {
                if self
                    .structs
                    .insert(struct_declaration.name.clone(), struct_declaration.clone())
                    .is_some()
                {
                    return Err(ExecutionError::new(format!(
                        "duplicate struct '{}' at runtime",
                        struct_declaration.name
                    )));
                }
            }
        }

        Ok(())
    }

    fn collect_functions(&mut self, program: &Program) -> ExecutionResult<()> {
        let mut current_module = None;

        for statement in &program.statements {
            if let Statement::ModuleDeclaration(module_declaration) = statement {
                current_module = Some(module_declaration.name.clone());
                continue;
            }
            if matches!(statement, Statement::ImportDeclaration(_)) {
                current_module = None;
                continue;
            }

            if let Statement::FunctionDeclaration(function) = statement {
                if let Some(module_name) = &current_module {
                    self.module_functions.insert(
                        (module_name.clone(), function.name.clone()),
                        function.clone(),
                    );
                }

                if self.functions.contains_key(&function.name)
                    || self.tools.contains_key(&function.name)
                {
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

    fn collect_async_tasks(&mut self, program: &Program) -> ExecutionResult<()> {
        for statement in &program.statements {
            if let Statement::AsyncTaskDeclaration(task) = statement {
                if self.async_tasks.contains_key(&task.name)
                    || self.functions.contains_key(&task.name)
                    || self.tools.contains_key(&task.name)
                {
                    return Err(ExecutionError::new(format!(
                        "duplicate async task '{}' at runtime",
                        task.name
                    )));
                }

                self.async_tasks.insert(
                    task.name.clone(),
                    FunctionDeclaration {
                        name: task.name.clone(),
                        parameters: task.parameters.clone(),
                        return_type: task.return_type.clone(),
                        body: task.body.clone(),
                        span: task.span.clone(),
                    },
                );
            }
        }

        Ok(())
    }

    fn collect_tools(&mut self, program: &Program) -> ExecutionResult<()> {
        for statement in &program.statements {
            if let Statement::ToolDeclaration(tool) = statement {
                if self.tools.contains_key(&tool.name) || self.functions.contains_key(&tool.name) {
                    return Err(ExecutionError::new(format!(
                        "duplicate tool '{}' at runtime",
                        tool.name
                    )));
                }

                self.tools.insert(tool.name.clone(), tool.clone());
            }
        }

        Ok(())
    }

    fn collect_agent_tasks(&mut self, program: &Program) -> ExecutionResult<()> {
        for statement in &program.statements {
            let Statement::AgentDeclaration(AgentDeclaration { name, tasks, .. }) = statement
            else {
                continue;
            };

            for task in tasks {
                let key = (name.clone(), task.name.clone());
                if self.agent_tasks.contains_key(&key) {
                    return Err(ExecutionError::new(format!(
                        "duplicate agent task '{}.{}' at runtime",
                        name, task.name
                    )));
                }

                self.agent_tasks.insert(
                    key,
                    FunctionDeclaration {
                        name: format!("{name}.{}", task.name),
                        parameters: task.parameters.clone(),
                        return_type: task.return_type.clone(),
                        body: task.body.clone(),
                        span: task.span.clone(),
                    },
                );
            }
        }

        Ok(())
    }

    fn collect_workflows(&mut self, program: &Program) -> ExecutionResult<()> {
        for statement in &program.statements {
            let Statement::WorkflowDeclaration(workflow) = statement else {
                continue;
            };

            if self
                .workflows
                .insert(workflow.name.clone(), workflow.clone())
                .is_some()
            {
                return Err(ExecutionError::new(format!(
                    "duplicate workflow '{}' at runtime",
                    workflow.name
                )));
            }
        }

        Ok(())
    }

    fn collect_schemas(&mut self, program: &Program) -> ExecutionResult<()> {
        for statement in &program.statements {
            if let Statement::SchemaDeclaration(schema_declaration) = statement {
                if self
                    .schemas
                    .insert(schema_declaration.name.clone(), schema_declaration.clone())
                    .is_some()
                {
                    return Err(ExecutionError::new(format!(
                        "duplicate schema '{}' at runtime",
                        schema_declaration.name
                    )));
                }
            }
        }

        Ok(())
    }

    fn collect_methods(&mut self, program: &Program) -> ExecutionResult<()> {
        for statement in &program.statements {
            let Statement::ImplBlock(ImplBlock {
                target_name,
                methods,
                ..
            }) = statement
            else {
                continue;
            };

            for method in methods {
                let key = (target_name.clone(), method.name.clone());
                if self.methods.contains_key(&key) {
                    return Err(ExecutionError::new(format!(
                        "duplicate method '{}.{}' at runtime",
                        target_name, method.name
                    )));
                }

                self.methods.insert(key, method.clone());
            }
        }

        Ok(())
    }

    fn execute_statement(&mut self, statement: &Statement) -> ExecutionResult<StatementControl> {
        match statement {
            Statement::VariableDeclaration(declaration) => {
                let value = self.evaluate_expression(&declaration.value)?;
                self.environment
                    .define(declaration.name.clone(), value, declaration.is_mutable)?;
                Ok(StatementControl::None)
            }
            Statement::Assignment(assignment) => self.execute_assignment_statement(assignment),
            Statement::Return(return_statement) => {
                let value = self.evaluate_expression(&return_statement.value)?;
                Ok(StatementControl::Return(value))
            }
            Statement::If(if_statement) => self.execute_if_statement(if_statement),
            Statement::While(while_statement) => self.execute_while_statement(while_statement),
            Statement::For(for_statement) => self.execute_for_statement(for_statement),
            Statement::Break(_) => Ok(StatementControl::Break),
            Statement::Continue(_) => Ok(StatementControl::Continue),
            Statement::Expression(expression) => {
                self.evaluate_expression(expression)?;
                Ok(StatementControl::None)
            }
            Statement::ModuleDeclaration(_)
            | Statement::ImportDeclaration(_)
            | Statement::FunctionDeclaration(_)
            | Statement::AsyncTaskDeclaration(_)
            | Statement::ToolDeclaration(_)
            | Statement::AgentDeclaration(_)
            | Statement::WorkflowDeclaration(_)
            | Statement::StructDeclaration(_)
            | Statement::SchemaDeclaration(_)
            | Statement::ImplBlock(_) => Ok(StatementControl::None),
        }
    }

    fn execute_workflow_step(
        &mut self,
        step: &loz_ast::WorkflowStep,
    ) -> ExecutionResult<RuntimeValue> {
        match &step.target {
            WorkflowTarget::FunctionOrTool(target_name) => {
                if self.functions.contains_key(target_name) {
                    self.call_function(target_name, Vec::new())
                } else if self.tools.contains_key(target_name) {
                    self.call_tool(target_name, Vec::new())
                } else {
                    Err(ExecutionError::new(format!(
                        "unknown workflow step target '{}'",
                        target_name
                    )))
                }
            }
            WorkflowTarget::AgentTask {
                agent_name,
                task_name,
            } => {
                let task = self
                    .agent_tasks
                    .get(&(agent_name.clone(), task_name.clone()))
                    .cloned()
                    .ok_or_else(|| {
                        ExecutionError::new(format!(
                            "unknown workflow step target '{}.{}'",
                            agent_name, task_name
                        ))
                    })?;

                self.call_callable(&format!("{agent_name}.{task_name}"), &task, Vec::new())
            }
        }
    }

    fn execute_if_statement(
        &mut self,
        if_statement: &loz_ast::IfStatement,
    ) -> ExecutionResult<StatementControl> {
        if self.evaluate_condition(&if_statement.condition)? {
            self.execute_scoped_block(&if_statement.then_branch)
        } else if let Some(else_branch) = &if_statement.else_branch {
            self.execute_scoped_block(else_branch)
        } else {
            Ok(StatementControl::None)
        }
    }

    fn execute_while_statement(
        &mut self,
        while_statement: &loz_ast::WhileStatement,
    ) -> ExecutionResult<StatementControl> {
        while self.evaluate_condition(&while_statement.condition)? {
            match self.execute_scoped_block(&while_statement.body)? {
                StatementControl::None | StatementControl::Continue => {}
                StatementControl::Break => break,
                StatementControl::Return(value) => return Ok(StatementControl::Return(value)),
            }
        }

        Ok(StatementControl::None)
    }

    fn execute_for_statement(
        &mut self,
        for_statement: &loz_ast::ForStatement,
    ) -> ExecutionResult<StatementControl> {
        let iterable_value = self.evaluate_expression(&for_statement.iterable)?;
        let RuntimeValue::Array(values) = iterable_value else {
            return Err(ExecutionError::new(
                "for loop source must evaluate to an Array runtime value",
            ));
        };

        for value in values {
            self.environment.push_scope();
            self.environment.define(
                for_statement.variable_name.clone(),
                value,
                for_statement.is_mutable,
            )?;

            let result = self.execute_block(&for_statement.body);
            self.environment.pop_scope();

            match result? {
                StatementControl::None | StatementControl::Continue => {}
                StatementControl::Break => break,
                StatementControl::Return(return_value) => {
                    return Ok(StatementControl::Return(return_value));
                }
            }
        }

        Ok(StatementControl::None)
    }

    fn evaluate_expression(&mut self, expression: &Expression) -> ExecutionResult<RuntimeValue> {
        match &expression.kind {
            ExpressionKind::IntegerLiteral(value) => Ok(RuntimeValue::Int(*value)),
            ExpressionKind::FloatLiteral(value) => Ok(RuntimeValue::Float(*value)),
            ExpressionKind::BooleanLiteral(value) => Ok(RuntimeValue::Bool(*value)),
            ExpressionKind::StringLiteral(value) => Ok(RuntimeValue::Text(value.clone())),
            ExpressionKind::Await(await_expression) => {
                self.execute_await_expression(await_expression)
            }
            ExpressionKind::Reference(reference_expression) => Ok(RuntimeValue::Reference {
                target: reference_expression.target_name.clone(),
                is_mutable: reference_expression.is_mutable,
            }),
            ExpressionKind::Dereference(dereference_expression) => {
                let reference_value = self.evaluate_expression(&dereference_expression.value)?;
                let RuntimeValue::Reference { target, .. } = reference_value else {
                    return Err(ExecutionError::new(
                        "cannot dereference a non-reference runtime value",
                    ));
                };

                self.environment.get(&target)
            }
            ExpressionKind::Cast(cast_expression) => {
                let value = self.evaluate_expression(&cast_expression.value)?;
                self.cast_runtime_value(value, &cast_expression.target_type)
            }
            ExpressionKind::Identifier(name) => self.environment.get(name),
            ExpressionKind::ArrayLiteral(array_literal) => {
                let mut values = Vec::with_capacity(array_literal.elements.len());
                for element in &array_literal.elements {
                    values.push(self.evaluate_expression(element)?);
                }

                Ok(RuntimeValue::Array(values))
            }
            ExpressionKind::IndexAccess(index_access) => {
                let base_value = self.environment.get(&index_access.base_name)?;
                let RuntimeValue::Array(values) = base_value else {
                    return Err(ExecutionError::new(format!(
                        "cannot index non-array runtime value '{}'",
                        index_access.base_name
                    )));
                };

                let index_value = self.evaluate_expression(&index_access.index)?;
                let RuntimeValue::Int(index) = index_value else {
                    return Err(ExecutionError::new(format!(
                        "array index for '{}' must be an Int runtime value",
                        index_access.base_name
                    )));
                };

                let index: usize = index.try_into().map_err(|_| {
                    ExecutionError::new(format!(
                        "array index for '{}' cannot be negative",
                        index_access.base_name
                    ))
                })?;

                values.get(index).cloned().ok_or_else(|| {
                    ExecutionError::new(format!(
                        "array '{}' index {} is out of bounds",
                        index_access.base_name, index
                    ))
                })
            }
            ExpressionKind::FieldAccess(field_access) => {
                let base_value = self.environment.get(&field_access.base_name)?;
                let RuntimeValue::Struct { name, fields } = base_value else {
                    return Err(ExecutionError::new(format!(
                        "cannot access field '{}' on non-struct runtime value '{}'",
                        field_access.field_name, field_access.base_name
                    )));
                };

                let struct_declaration = self.structs.get(&name).ok_or_else(|| {
                    ExecutionError::new(format!(
                        "unknown runtime struct '{}' during field access",
                        name
                    ))
                })?;

                let field_index = struct_declaration
                    .fields
                    .iter()
                    .position(|field| field.name == field_access.field_name)
                    .ok_or_else(|| {
                        ExecutionError::new(format!(
                            "struct '{}' has no field '{}'",
                            name, field_access.field_name
                        ))
                    })?;

                fields.get(field_index).cloned().ok_or_else(|| {
                    ExecutionError::new(format!(
                        "field '{}' is missing from runtime struct '{}'",
                        field_access.field_name, name
                    ))
                })
            }
            ExpressionKind::MethodCall(method_call) => self.execute_method_call(method_call),
            ExpressionKind::Call(call) => {
                if call.callee == "Map" {
                    if !call.arguments.is_empty() {
                        return Err(ExecutionError::new(format!(
                            "built-in constructor 'Map()' expects 0 arguments, found {}",
                            call.arguments.len()
                        )));
                    }

                    return Ok(RuntimeValue::Map(RuntimeMap {
                        entries: HashMap::new(),
                    }));
                }

                if call.callee == "Set" {
                    if !call.arguments.is_empty() {
                        return Err(ExecutionError::new(format!(
                            "built-in constructor 'Set()' expects 0 arguments, found {}",
                            call.arguments.len()
                        )));
                    }

                    return Ok(RuntimeValue::Set(RuntimeSet {
                        entries: HashSet::new(),
                    }));
                }

                if let Some(struct_declaration) = self.structs.get(&call.callee).cloned() {
                    return self.execute_struct_constructor(&struct_declaration, &call.arguments);
                }

                if self.async_tasks.contains_key(&call.callee) {
                    return Err(ExecutionError::new(format!(
                        "async task '{}' must be awaited",
                        call.callee
                    )));
                }

                let mut arguments = Vec::with_capacity(call.arguments.len());
                for argument in &call.arguments {
                    arguments.push(self.evaluate_expression(argument)?);
                }

                if call.callee == "print" {
                    self.execute_print(arguments)
                } else {
                    self.call_function_or_tool(&call.callee, arguments)
                }
            }
            ExpressionKind::Binary(binary) => {
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
                    (
                        RuntimeValue::Int(lhs),
                        RuntimeValue::Int(rhs),
                        loz_ast::BinaryOperator::Greater,
                    ) => Ok(RuntimeValue::Bool(lhs > rhs)),
                    (
                        RuntimeValue::Int(lhs),
                        RuntimeValue::Int(rhs),
                        loz_ast::BinaryOperator::Less,
                    ) => Ok(RuntimeValue::Bool(lhs < rhs)),
                    (
                        RuntimeValue::Int(lhs),
                        RuntimeValue::Int(rhs),
                        loz_ast::BinaryOperator::GreaterEqual,
                    ) => Ok(RuntimeValue::Bool(lhs >= rhs)),
                    (
                        RuntimeValue::Int(lhs),
                        RuntimeValue::Int(rhs),
                        loz_ast::BinaryOperator::LessEqual,
                    ) => Ok(RuntimeValue::Bool(lhs <= rhs)),
                    (
                        RuntimeValue::Int(lhs),
                        RuntimeValue::Int(rhs),
                        loz_ast::BinaryOperator::Equal,
                    ) => Ok(RuntimeValue::Bool(lhs == rhs)),
                    (
                        RuntimeValue::Int(lhs),
                        RuntimeValue::Int(rhs),
                        loz_ast::BinaryOperator::NotEqual,
                    ) => Ok(RuntimeValue::Bool(lhs != rhs)),
                    (
                        RuntimeValue::Float(lhs),
                        RuntimeValue::Float(rhs),
                        loz_ast::BinaryOperator::Greater,
                    ) => Ok(RuntimeValue::Bool(lhs > rhs)),
                    (
                        RuntimeValue::Float(lhs),
                        RuntimeValue::Float(rhs),
                        loz_ast::BinaryOperator::Less,
                    ) => Ok(RuntimeValue::Bool(lhs < rhs)),
                    (
                        RuntimeValue::Float(lhs),
                        RuntimeValue::Float(rhs),
                        loz_ast::BinaryOperator::GreaterEqual,
                    ) => Ok(RuntimeValue::Bool(lhs >= rhs)),
                    (
                        RuntimeValue::Float(lhs),
                        RuntimeValue::Float(rhs),
                        loz_ast::BinaryOperator::LessEqual,
                    ) => Ok(RuntimeValue::Bool(lhs <= rhs)),
                    (
                        RuntimeValue::Float(lhs),
                        RuntimeValue::Float(rhs),
                        loz_ast::BinaryOperator::Equal,
                    ) => Ok(RuntimeValue::Bool(lhs == rhs)),
                    (
                        RuntimeValue::Float(lhs),
                        RuntimeValue::Float(rhs),
                        loz_ast::BinaryOperator::NotEqual,
                    ) => Ok(RuntimeValue::Bool(lhs != rhs)),
                    (
                        RuntimeValue::Bool(lhs),
                        RuntimeValue::Bool(rhs),
                        loz_ast::BinaryOperator::Equal,
                    ) => Ok(RuntimeValue::Bool(lhs == rhs)),
                    (
                        RuntimeValue::Bool(lhs),
                        RuntimeValue::Bool(rhs),
                        loz_ast::BinaryOperator::NotEqual,
                    ) => Ok(RuntimeValue::Bool(lhs != rhs)),
                    _ => Err(ExecutionError::new(format!(
                        "unsupported runtime operands for binary expression: left={left:?}, right={right:?}"
                    ))),
                }
            }
        }
    }

    fn execute_await_expression(
        &mut self,
        await_expression: &AwaitExpression,
    ) -> ExecutionResult<RuntimeValue> {
        let ExpressionKind::Call(call) = &await_expression.expression.kind else {
            return Err(ExecutionError::new(
                "await currently requires an async task call expression",
            ));
        };

        let mut arguments = Vec::with_capacity(call.arguments.len());
        for argument in &call.arguments {
            arguments.push(self.evaluate_expression(argument)?);
        }

        self.call_async_task(&call.callee, arguments)
    }

    fn execute_method_call(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
    ) -> ExecutionResult<RuntimeValue> {
        if method_call.base_name == "io" {
            return self.execute_io_method_call(method_call);
        }
        if method_call.base_name == "json" {
            return self.execute_json_method_call(method_call);
        }
        if method_call.base_name == "schema" {
            return self.execute_schema_method_call(method_call);
        }
        if method_call.base_name == "python" {
            return self.execute_python_method_call(method_call);
        }
        if method_call.base_name == "llm" {
            return self.execute_llm_method_call(method_call);
        }

        if let Some(function) = self
            .module_functions
            .get(&(
                method_call.base_name.clone(),
                method_call.method_name.clone(),
            ))
            .cloned()
        {
            let mut arguments = Vec::with_capacity(method_call.arguments.len());
            for argument in &method_call.arguments {
                arguments.push(self.evaluate_expression(argument)?);
            }

            return self.call_callable(
                &format!("{}.{}", method_call.base_name, method_call.method_name),
                &function,
                arguments,
            );
        }

        let (base_value, is_mutable) = self.environment.get_binding(&method_call.base_name)?;

        match base_value {
            RuntimeValue::Array(values) => {
                self.execute_array_method_call(method_call, values, is_mutable)
            }
            RuntimeValue::Map(map) => self.execute_map_method_call(method_call, map, is_mutable),
            RuntimeValue::Set(set) => self.execute_set_method_call(method_call, set, is_mutable),
            RuntimeValue::Struct { name, .. } => self.execute_struct_method_call(
                &name,
                method_call,
                self.environment.get(&method_call.base_name)?,
            ),
            other => Err(ExecutionError::new(format!(
                "cannot call method '{}()' on runtime value {:?}",
                method_call.method_name, other
            ))),
        }
    }

    fn execute_io_method_call(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
    ) -> ExecutionResult<RuntimeValue> {
        self.ensure_zero_io_arguments(method_call)?;

        match method_call.method_name.as_str() {
            "read_line" => self.read_stdin_line_trimmed().map(RuntimeValue::Text),
            "read_i8" => self
                .read_signed_integer::<i8>("read_i8")
                .map(|value| RuntimeValue::Int(i64::from(value))),
            "read_i16" => self
                .read_signed_integer::<i16>("read_i16")
                .map(|value| RuntimeValue::Int(i64::from(value))),
            "read_i32" => self
                .read_signed_integer::<i32>("read_i32")
                .map(|value| RuntimeValue::Int(i64::from(value))),
            "read_i64" => self
                .read_signed_integer::<i64>("read_i64")
                .map(RuntimeValue::Int),
            "read_u8" => self
                .read_unsigned_integer::<u8>("read_u8")
                .map(|value| RuntimeValue::Int(i64::from(value))),
            "read_u16" => self
                .read_unsigned_integer::<u16>("read_u16")
                .map(|value| RuntimeValue::Int(i64::from(value))),
            "read_u32" => self
                .read_unsigned_integer::<u32>("read_u32")
                .map(|value| RuntimeValue::Int(i64::from(value))),
            "read_u64" => self
                .read_unsigned_integer::<u64>("read_u64")
                .and_then(|value| {
                    i64::try_from(value).map(RuntimeValue::Int).map_err(|_| {
                        ExecutionError::new(format!(
                            "io.read_u64() input '{value}' exceeds interpreter integer range"
                        ))
                    })
                }),
            "read_f32" => self
                .read_float::<f32>("read_f32")
                .map(|value| RuntimeValue::Float(f64::from(value))),
            "read_f64" => self.read_float::<f64>("read_f64").map(RuntimeValue::Float),
            "read_bool" => self.read_bool(),
            other => Err(ExecutionError::new(format!(
                "unknown io method '{}()' at runtime",
                other
            ))),
        }
    }

    fn execute_json_method_call(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
    ) -> ExecutionResult<RuntimeValue> {
        match method_call.method_name.as_str() {
            "parse" => {
                let [text_argument] = method_call.arguments.as_slice() else {
                    return Err(ExecutionError::new(format!(
                        "json.parse() expects 1 argument at runtime, found {}",
                        method_call.arguments.len()
                    )));
                };
                let text = self.evaluate_text_argument(text_argument, "json.parse")?;
                let value: JsonValue = serde_json::from_str(&text).map_err(|error| {
                    ExecutionError::new(format!("invalid JSON in json.parse(): {error}"))
                })?;
                Ok(RuntimeValue::Json(value))
            }
            "stringify" => {
                let [json_argument] = method_call.arguments.as_slice() else {
                    return Err(ExecutionError::new(format!(
                        "json.stringify() expects 1 argument at runtime, found {}",
                        method_call.arguments.len()
                    )));
                };
                let json_value = self.evaluate_json_argument(json_argument, "json.stringify")?;
                serde_json::to_string(&json_value)
                    .map(RuntimeValue::Text)
                    .map_err(|error| {
                        ExecutionError::new(format!(
                            "failed to stringify Json in json.stringify(): {error}"
                        ))
                    })
            }
            "get_text" => self.execute_json_getter(method_call, "get_text", |this, value, key| {
                let json_value = this.require_json_object_field(value, key, "json.get_text")?;
                let text = json_value.as_str().ok_or_else(|| {
                    ExecutionError::new(format!(
                        "key '{}' has wrong type in json.get_text(): expected Text",
                        key
                    ))
                })?;
                Ok(RuntimeValue::Text(text.to_string()))
            }),
            "get_i32" => self.execute_json_getter(method_call, "get_i32", |this, value, key| {
                let json_value = this.require_json_object_field(value, key, "json.get_i32")?;
                let int = json_value.as_i64().ok_or_else(|| {
                    ExecutionError::new(format!(
                        "key '{}' has wrong type in json.get_i32(): expected i32",
                        key
                    ))
                })?;
                let int = i32::try_from(int).map_err(|_| {
                    ExecutionError::new(format!("key '{}' is out of range in json.get_i32()", key))
                })?;
                Ok(RuntimeValue::Int(i64::from(int)))
            }),
            "get_i64" => self.execute_json_getter(method_call, "get_i64", |this, value, key| {
                let json_value = this.require_json_object_field(value, key, "json.get_i64")?;
                let int = json_value.as_i64().ok_or_else(|| {
                    ExecutionError::new(format!(
                        "key '{}' has wrong type in json.get_i64(): expected i64",
                        key
                    ))
                })?;
                Ok(RuntimeValue::Int(int))
            }),
            "get_f64" => self.execute_json_getter(method_call, "get_f64", |this, value, key| {
                let json_value = this.require_json_object_field(value, key, "json.get_f64")?;
                let float = json_value.as_f64().ok_or_else(|| {
                    ExecutionError::new(format!(
                        "key '{}' has wrong type in json.get_f64(): expected f64",
                        key
                    ))
                })?;
                Ok(RuntimeValue::Float(float))
            }),
            "get_bool" => self.execute_json_getter(method_call, "get_bool", |this, value, key| {
                let json_value = this.require_json_object_field(value, key, "json.get_bool")?;
                let boolean = json_value.as_bool().ok_or_else(|| {
                    ExecutionError::new(format!(
                        "key '{}' has wrong type in json.get_bool(): expected Bool",
                        key
                    ))
                })?;
                Ok(RuntimeValue::Bool(boolean))
            }),
            "has" => {
                let (json_value, key) =
                    self.evaluate_json_key_arguments(method_call, "json.has")?;
                let object = json_value.as_object().ok_or_else(|| {
                    ExecutionError::new("Json value is not an object in json.has()")
                })?;
                Ok(RuntimeValue::Bool(object.contains_key(&key)))
            }
            other => Err(ExecutionError::new(format!(
                "unknown json method '{}()' at runtime",
                other
            ))),
        }
    }

    fn execute_schema_method_call(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
    ) -> ExecutionResult<RuntimeValue> {
        let (schema, json_value) = self.evaluate_schema_arguments(method_call)?;
        let validation = self.validate_json_against_schema(&schema, &json_value);

        match method_call.method_name.as_str() {
            "validate" => Ok(RuntimeValue::Bool(validation.is_ok())),
            "require" => match validation {
                Ok(()) => Ok(RuntimeValue::Json(json_value)),
                Err(error) => Err(ExecutionError::new(format!(
                    "schema.require() failed for schema '{}': {error}",
                    schema.name
                ))),
            },
            other => Err(ExecutionError::new(format!(
                "unknown schema method '{}()' at runtime",
                other
            ))),
        }
    }

    fn execute_python_method_call(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
    ) -> ExecutionResult<RuntimeValue> {
        match method_call.method_name.as_str() {
            "call" => {
                let [path_argument, input_argument] = method_call.arguments.as_slice() else {
                    return Err(ExecutionError::new(format!(
                        "python.call() expects 2 arguments at runtime, found {}",
                        method_call.arguments.len()
                    )));
                };

                let function_path = self.evaluate_text_argument(path_argument, "python.call")?;
                let input_json = self.evaluate_json_argument(input_argument, "python.call")?;
                let output_json = self.run_python_bridge(&function_path, &input_json)?;
                Ok(RuntimeValue::Json(output_json))
            }
            other => Err(ExecutionError::new(format!(
                "unknown python method '{}()' at runtime",
                other
            ))),
        }
    }

    fn execute_llm_method_call(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
    ) -> ExecutionResult<RuntimeValue> {
        match method_call.method_name.as_str() {
            "ask" => {
                let [prompt_argument] = method_call.arguments.as_slice() else {
                    return Err(ExecutionError::new(format!(
                        "llm.ask() expects 1 argument at runtime, found {}",
                        method_call.arguments.len()
                    )));
                };

                let prompt = self.evaluate_text_argument(prompt_argument, "llm.ask")?;
                let response = run_llm_request(&prompt).map_err(ExecutionError::new)?;
                Ok(RuntimeValue::Text(response))
            }
            other => Err(ExecutionError::new(format!(
                "unknown llm method '{}()' at runtime",
                other
            ))),
        }
    }

    fn execute_json_getter<F>(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
        method_name: &str,
        getter: F,
    ) -> ExecutionResult<RuntimeValue>
    where
        F: Fn(&Self, &JsonValue, &str) -> ExecutionResult<RuntimeValue>,
    {
        let (json_value, key) =
            self.evaluate_json_key_arguments(method_call, &format!("json.{method_name}"))?;
        getter(self, &json_value, &key)
    }

    fn evaluate_json_key_arguments(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
        method_name: &str,
    ) -> ExecutionResult<(JsonValue, String)> {
        let [json_argument, key_argument] = method_call.arguments.as_slice() else {
            return Err(ExecutionError::new(format!(
                "{method_name}() expects 2 arguments at runtime, found {}",
                method_call.arguments.len()
            )));
        };
        let json_value = self.evaluate_json_argument(json_argument, method_name)?;
        let key = self.evaluate_text_argument(key_argument, method_name)?;
        Ok((json_value, key))
    }

    fn evaluate_json_argument(
        &mut self,
        argument: &Expression,
        context: &str,
    ) -> ExecutionResult<JsonValue> {
        match self.evaluate_expression(argument)? {
            RuntimeValue::Json(value) => Ok(value),
            other => Err(ExecutionError::new(format!(
                "{context}() expected Json argument, found {:?}",
                other
            ))),
        }
    }

    fn evaluate_schema_arguments(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
    ) -> ExecutionResult<(SchemaDeclaration, JsonValue)> {
        let [schema_name_argument, json_argument] = method_call.arguments.as_slice() else {
            return Err(ExecutionError::new(format!(
                "schema.{}() expects 2 arguments at runtime, found {}",
                method_call.method_name,
                method_call.arguments.len()
            )));
        };

        let schema_name = self.evaluate_text_argument(
            schema_name_argument,
            &format!("schema.{}", method_call.method_name),
        )?;
        let schema =
            self.schemas.get(&schema_name).cloned().ok_or_else(|| {
                ExecutionError::new(format!("unknown schema name '{}'", schema_name))
            })?;
        let json_value = self.evaluate_json_argument(
            json_argument,
            &format!("schema.{}", method_call.method_name),
        )?;

        Ok((schema, json_value))
    }

    fn validate_json_against_schema(
        &self,
        schema: &SchemaDeclaration,
        json_value: &JsonValue,
    ) -> ExecutionResult<()> {
        let object = json_value.as_object().ok_or_else(|| {
            ExecutionError::new(format!(
                "Json value is not an object for schema '{}'",
                schema.name
            ))
        })?;

        for field in &schema.fields {
            let value = object.get(&field.name).ok_or_else(|| {
                ExecutionError::new(format!(
                    "missing key '{}' for schema '{}'",
                    field.name, schema.name
                ))
            })?;
            self.validate_schema_field_type(&field.type_name, value, &schema.name, &field.name)?;
        }

        Ok(())
    }

    fn validate_schema_field_type(
        &self,
        type_name: &TypeName,
        value: &JsonValue,
        schema_name: &str,
        field_name: &str,
    ) -> ExecutionResult<()> {
        let is_valid = match type_name {
            TypeName::Text => value.is_string(),
            TypeName::Bool => value.is_boolean(),
            TypeName::I32 => value
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .is_some(),
            TypeName::I64 => value.as_i64().is_some(),
            TypeName::F64 => value.as_f64().is_some(),
            TypeName::Json => true,
            other => {
                return Err(ExecutionError::new(format!(
                    "unsupported schema field type {:?} in schema '{}'",
                    other, schema_name
                )));
            }
        };

        if is_valid {
            Ok(())
        } else {
            Err(ExecutionError::new(format!(
                "key '{}' has wrong type for schema '{}'",
                field_name, schema_name
            )))
        }
    }

    fn evaluate_text_argument(
        &mut self,
        argument: &Expression,
        context: &str,
    ) -> ExecutionResult<String> {
        match self.evaluate_expression(argument)? {
            RuntimeValue::Text(value) => Ok(value),
            other => Err(ExecutionError::new(format!(
                "{context}() expected Text argument, found {:?}",
                other
            ))),
        }
    }

    fn run_python_bridge(
        &self,
        function_path: &str,
        input_json: &JsonValue,
    ) -> ExecutionResult<JsonValue> {
        let python_executable = python_executable();
        let bridge_script = python_bridge_script_path()?;
        let input_text = serde_json::to_string(input_json).map_err(|error| {
            ExecutionError::new(format!(
                "failed to serialize Json payload for python.call(): {error}"
            ))
        })?;

        let mut child = Command::new(&python_executable)
            .arg(&bridge_script)
            .arg(function_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                ExecutionError::new(format!(
                    "failed to launch Python executable '{}' for python.call(): {error}",
                    python_executable.to_string_lossy()
                ))
            })?;

        let mut stdin = child.stdin.take().ok_or_else(|| {
            ExecutionError::new("failed to open stdin for Python bridge in python.call()")
        })?;
        stdin.write_all(input_text.as_bytes()).map_err(|error| {
            ExecutionError::new(format!(
                "failed to write Json payload to Python bridge stdin: {error}"
            ))
        })?;
        drop(stdin);

        let output = child.wait_with_output().map_err(|error| {
            ExecutionError::new(format!(
                "failed to wait for Python bridge in python.call(): {error}"
            ))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if stderr.is_empty() {
                format!("Python bridge exited with status {}", output.status)
            } else {
                stderr
            };
            return Err(ExecutionError::new(format!(
                "python.call() failed for '{}': {}",
                function_path, detail
            )));
        }

        serde_json::from_slice(&output.stdout).map_err(|error| {
            ExecutionError::new(format!(
                "python.call() returned invalid JSON for '{}': {}",
                function_path, error
            ))
        })
    }

    fn require_json_object_field<'a>(
        &self,
        value: &'a JsonValue,
        key: &str,
        context: &str,
    ) -> ExecutionResult<&'a JsonValue> {
        let object = value.as_object().ok_or_else(|| {
            ExecutionError::new(format!("Json value is not an object in {context}()"))
        })?;
        object
            .get(key)
            .ok_or_else(|| ExecutionError::new(format!("missing key '{}' in {context}()", key)))
    }

    fn ensure_zero_io_arguments(
        &self,
        method_call: &loz_ast::MethodCallExpression,
    ) -> ExecutionResult<()> {
        if method_call.arguments.is_empty() {
            Ok(())
        } else {
            Err(ExecutionError::new(format!(
                "io.{}() expects 0 arguments at runtime, found {}",
                method_call.method_name,
                method_call.arguments.len()
            )))
        }
    }

    fn read_stdin_line_trimmed(&mut self) -> ExecutionResult<String> {
        if let Some(line) = self.input_lines.pop_front() {
            return Ok(line);
        }

        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|error| ExecutionError::new(format!("failed to read from stdin: {error}")))?;

        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }

        Ok(line)
    }

    fn read_signed_integer<T>(&mut self, method_name: &str) -> ExecutionResult<T>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        let line = self.read_stdin_line_trimmed()?;
        line.parse::<T>().map_err(|error| {
            ExecutionError::new(format!(
                "failed to parse io.{method_name}() input '{line}': {error}"
            ))
        })
    }

    fn read_unsigned_integer<T>(&mut self, method_name: &str) -> ExecutionResult<T>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        let line = self.read_stdin_line_trimmed()?;
        line.parse::<T>().map_err(|error| {
            ExecutionError::new(format!(
                "failed to parse io.{method_name}() input '{line}': {error}"
            ))
        })
    }

    fn read_float<T>(&mut self, method_name: &str) -> ExecutionResult<T>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        let line = self.read_stdin_line_trimmed()?;
        line.parse::<T>().map_err(|error| {
            ExecutionError::new(format!(
                "failed to parse io.{method_name}() input '{line}': {error}"
            ))
        })
    }

    fn read_bool(&mut self) -> ExecutionResult<RuntimeValue> {
        let line = self.read_stdin_line_trimmed()?;
        let normalized = line.trim().to_ascii_lowercase();

        let value = match normalized.as_str() {
            "true" | "1" | "yes" => true,
            "false" | "0" | "no" => false,
            _ => {
                return Err(ExecutionError::new(format!(
                    "failed to parse io.read_bool() input '{line}' as Bool"
                )));
            }
        };

        Ok(RuntimeValue::Bool(value))
    }

    fn execute_array_method_call(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
        mut values: Vec<RuntimeValue>,
        is_mutable: bool,
    ) -> ExecutionResult<RuntimeValue> {
        match method_call.method_name.as_str() {
            "len" => Ok(RuntimeValue::Int(values.len() as i64)),
            "push" => {
                if !is_mutable {
                    return Err(ExecutionError::new(format!(
                        "cannot call push() on immutable array '{}'",
                        method_call.base_name
                    )));
                }

                let value = method_call
                    .arguments
                    .first()
                    .ok_or_else(|| ExecutionError::new("push() requires an argument at runtime"))
                    .and_then(|argument| self.evaluate_expression(argument))?;
                values.push(value);
                self.environment
                    .assign(&method_call.base_name, RuntimeValue::Array(values))?;
                Ok(RuntimeValue::Void)
            }
            "pop" => {
                if !is_mutable {
                    return Err(ExecutionError::new(format!(
                        "cannot call pop() on immutable array '{}'",
                        method_call.base_name
                    )));
                }

                let value = values.pop().ok_or_else(|| {
                    ExecutionError::new(format!(
                        "cannot pop() from empty array '{}'",
                        method_call.base_name
                    ))
                })?;
                self.environment
                    .assign(&method_call.base_name, RuntimeValue::Array(values))?;
                Ok(value)
            }
            other => Err(ExecutionError::new(format!(
                "unknown array method '{}()' at runtime",
                other
            ))),
        }
    }

    fn execute_struct_method_call(
        &mut self,
        struct_name: &str,
        method_call: &loz_ast::MethodCallExpression,
        receiver: RuntimeValue,
    ) -> ExecutionResult<RuntimeValue> {
        let method = self
            .methods
            .get(&(struct_name.to_string(), method_call.method_name.clone()))
            .cloned()
            .ok_or_else(|| {
                ExecutionError::new(format!(
                    "struct '{}' has no method '{}()' at runtime",
                    struct_name, method_call.method_name
                ))
            })?;

        let mut arguments = Vec::with_capacity(method_call.arguments.len() + 1);
        arguments.push(receiver);
        for argument in &method_call.arguments {
            arguments.push(self.evaluate_expression(argument)?);
        }

        self.call_callable(
            &format!("{}.{}()", struct_name, method_call.method_name),
            &method,
            arguments,
        )
    }

    fn execute_map_method_call(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
        mut map: RuntimeMap,
        is_mutable: bool,
    ) -> ExecutionResult<RuntimeValue> {
        match method_call.method_name.as_str() {
            "insert" => {
                if !is_mutable {
                    return Err(ExecutionError::new(format!(
                        "cannot call insert() on immutable map '{}'",
                        method_call.base_name
                    )));
                }

                let key_value = self.evaluate_expression(&method_call.arguments[0])?;
                let key = self.runtime_hash_key_from_value(&key_value)?;
                let value = self.evaluate_expression(&method_call.arguments[1])?;
                map.entries.insert(key, value);
                self.environment
                    .assign(&method_call.base_name, RuntimeValue::Map(map))?;
                Ok(RuntimeValue::Void)
            }
            "get" => {
                let key_value = self.evaluate_expression(&method_call.arguments[0])?;
                let key = self.runtime_hash_key_from_value(&key_value)?;
                map.entries.get(&key).cloned().ok_or_else(|| {
                    ExecutionError::new(format!(
                        "map '{}' does not contain key",
                        method_call.base_name
                    ))
                })
            }
            "contains" => {
                let key_value = self.evaluate_expression(&method_call.arguments[0])?;
                let key = self.runtime_hash_key_from_value(&key_value)?;
                Ok(RuntimeValue::Bool(map.entries.contains_key(&key)))
            }
            other => Err(ExecutionError::new(format!(
                "unknown map method '{}()' at runtime",
                other
            ))),
        }
    }

    fn execute_set_method_call(
        &mut self,
        method_call: &loz_ast::MethodCallExpression,
        mut set: RuntimeSet,
        is_mutable: bool,
    ) -> ExecutionResult<RuntimeValue> {
        match method_call.method_name.as_str() {
            "add" => {
                if !is_mutable {
                    return Err(ExecutionError::new(format!(
                        "cannot call add() on immutable set '{}'",
                        method_call.base_name
                    )));
                }

                let value = self.evaluate_expression(&method_call.arguments[0])?;
                let key = self.runtime_hash_key_from_value(&value)?;
                set.entries.insert(key);
                self.environment
                    .assign(&method_call.base_name, RuntimeValue::Set(set))?;
                Ok(RuntimeValue::Void)
            }
            "contains" => {
                let value = self.evaluate_expression(&method_call.arguments[0])?;
                let key = self.runtime_hash_key_from_value(&value)?;
                Ok(RuntimeValue::Bool(set.entries.contains(&key)))
            }
            other => Err(ExecutionError::new(format!(
                "unknown set method '{}()' at runtime",
                other
            ))),
        }
    }

    fn runtime_hash_key_from_value(&self, value: &RuntimeValue) -> ExecutionResult<MapKey> {
        match value {
            RuntimeValue::Int(value) => Ok(MapKey::Int(*value)),
            RuntimeValue::Bool(value) => Ok(MapKey::Bool(*value)),
            RuntimeValue::Text(value) => Ok(MapKey::Text(value.clone())),
            other => Err(ExecutionError::new(format!(
                "unsupported runtime hash collection value {:?}",
                other
            ))),
        }
    }

    fn execute_assignment_statement(
        &mut self,
        assignment: &loz_ast::AssignmentStatement,
    ) -> ExecutionResult<StatementControl> {
        match &assignment.target {
            AssignmentTarget::Identifier(name) => {
                let value = self.evaluate_expression(&assignment.value)?;
                self.environment.assign(name, value)?;
                Ok(StatementControl::None)
            }
            AssignmentTarget::Dereference(dereference) => {
                let reference_value = self.evaluate_expression(&dereference.value)?;
                let RuntimeValue::Reference { target, is_mutable } = reference_value else {
                    return Err(ExecutionError::new(
                        "cannot assign through a non-reference runtime value",
                    ));
                };

                if !is_mutable {
                    return Err(ExecutionError::new(
                        "cannot assign through an immutable reference",
                    ));
                }

                let value = self.evaluate_expression(&assignment.value)?;
                self.environment.assign(&target, value)?;
                Ok(StatementControl::None)
            }
            AssignmentTarget::FieldAccess(field_access) => {
                let (base_value, is_mutable) =
                    self.environment.get_binding(&field_access.base_name)?;

                if !is_mutable {
                    return Err(ExecutionError::new(format!(
                        "cannot assign to field '{}.{}' because '{}' is immutable",
                        field_access.base_name, field_access.field_name, field_access.base_name
                    )));
                }

                let RuntimeValue::Struct { name, mut fields } = base_value else {
                    return Err(ExecutionError::new(format!(
                        "cannot assign to field '{}.{}' on non-struct runtime value",
                        field_access.base_name, field_access.field_name
                    )));
                };

                let struct_declaration = self.structs.get(&name).ok_or_else(|| {
                    ExecutionError::new(format!(
                        "unknown runtime struct '{}' during field assignment",
                        name
                    ))
                })?;

                let field_index = struct_declaration
                    .fields
                    .iter()
                    .position(|field| field.name == field_access.field_name)
                    .ok_or_else(|| {
                        ExecutionError::new(format!(
                            "struct '{}' has no field '{}'",
                            name, field_access.field_name
                        ))
                    })?;

                let value = self.evaluate_expression(&assignment.value)?;
                if let Some(slot) = fields.get_mut(field_index) {
                    *slot = value;
                } else {
                    return Err(ExecutionError::new(format!(
                        "field '{}' is missing from runtime struct '{}'",
                        field_access.field_name, name
                    )));
                }

                self.environment.assign(
                    &field_access.base_name,
                    RuntimeValue::Struct { name, fields },
                )?;
                Ok(StatementControl::None)
            }
            AssignmentTarget::IndexAccess(index_access) => {
                let (base_value, is_mutable) =
                    self.environment.get_binding(&index_access.base_name)?;

                if !is_mutable {
                    return Err(ExecutionError::new(format!(
                        "cannot assign to array element '{}[...]' because '{}' is immutable",
                        index_access.base_name, index_access.base_name
                    )));
                }

                let RuntimeValue::Array(mut values) = base_value else {
                    return Err(ExecutionError::new(format!(
                        "cannot assign to array element '{}[...]' on non-array runtime value",
                        index_access.base_name
                    )));
                };

                let index_value = self.evaluate_expression(&index_access.index)?;
                let RuntimeValue::Int(index) = index_value else {
                    return Err(ExecutionError::new(format!(
                        "array element assignment index for '{}' must be an Int runtime value",
                        index_access.base_name
                    )));
                };

                let index: usize = index.try_into().map_err(|_| {
                    ExecutionError::new(format!(
                        "array element assignment index for '{}' cannot be negative",
                        index_access.base_name
                    ))
                })?;

                let value = self.evaluate_expression(&assignment.value)?;
                if let Some(slot) = values.get_mut(index) {
                    *slot = value;
                } else {
                    return Err(ExecutionError::new(format!(
                        "array '{}' index {} is out of bounds",
                        index_access.base_name, index
                    )));
                }

                self.environment
                    .assign(&index_access.base_name, RuntimeValue::Array(values))?;
                Ok(StatementControl::None)
            }
        }
    }

    fn execute_struct_constructor(
        &mut self,
        struct_declaration: &StructDeclaration,
        arguments: &[Expression],
    ) -> ExecutionResult<RuntimeValue> {
        if struct_declaration.fields.len() != arguments.len() {
            return Err(ExecutionError::new(format!(
                "struct constructor '{}' expected {} arguments, found {}",
                struct_declaration.name,
                struct_declaration.fields.len(),
                arguments.len()
            )));
        }

        let mut values = Vec::with_capacity(arguments.len());
        for argument in arguments {
            values.push(self.evaluate_expression(argument)?);
        }

        Ok(RuntimeValue::Struct {
            name: struct_declaration.name.clone(),
            fields: values,
        })
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

    fn cast_runtime_value(
        &self,
        value: RuntimeValue,
        target_type: &TypeName,
    ) -> ExecutionResult<RuntimeValue> {
        match target_type {
            type_name if type_name.is_integer() => {
                let cast_value = match value {
                    RuntimeValue::Int(value) => value,
                    RuntimeValue::Float(value) => value as i64,
                    RuntimeValue::Bool(value) => i64::from(value),
                    other => {
                        return Err(ExecutionError::new(format!(
                            "cannot cast runtime value {:?} to {:?}",
                            other, target_type
                        )));
                    }
                };

                Ok(RuntimeValue::Int(
                    self.normalize_integer_for_type(cast_value, target_type),
                ))
            }
            type_name if type_name.is_float() => {
                let cast_value = match value {
                    RuntimeValue::Int(value) => value as f64,
                    RuntimeValue::Float(value) => value,
                    other => {
                        return Err(ExecutionError::new(format!(
                            "cannot cast runtime value {:?} to {:?}",
                            other, target_type
                        )));
                    }
                };

                if *target_type == TypeName::F32 {
                    Ok(RuntimeValue::Float((cast_value as f32) as f64))
                } else {
                    Ok(RuntimeValue::Float(cast_value))
                }
            }
            TypeName::Bool => match value {
                RuntimeValue::Int(value) => Ok(RuntimeValue::Bool(value != 0)),
                other => Err(ExecutionError::new(format!(
                    "cannot cast runtime value {:?} to Bool",
                    other
                ))),
            },
            _ => Err(ExecutionError::new(format!(
                "unsupported cast target at runtime: {:?}",
                target_type
            ))),
        }
    }

    fn normalize_integer_for_type(&self, value: i64, target_type: &TypeName) -> i64 {
        match target_type {
            TypeName::I8 => (value as i8) as i64,
            TypeName::I16 => (value as i16) as i64,
            TypeName::I32 => (value as i32) as i64,
            TypeName::I64 => value,
            TypeName::U8 => (value as u8) as i64,
            TypeName::U16 => (value as u16) as i64,
            TypeName::U32 => (value as u32) as i64,
            TypeName::U64 => (value as u64) as i64,
            _ => value,
        }
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

        self.call_callable(name, &function, arguments)
    }

    fn call_tool(
        &mut self,
        name: &str,
        arguments: Vec<RuntimeValue>,
    ) -> ExecutionResult<RuntimeValue> {
        let tool = self
            .tools
            .get(name)
            .cloned()
            .ok_or_else(|| ExecutionError::new(format!("unknown tool '{}'", name)))?;
        let callable = self.tool_as_function(&tool);

        self.call_callable(name, &callable, arguments)
    }

    fn call_function_or_tool(
        &mut self,
        name: &str,
        arguments: Vec<RuntimeValue>,
    ) -> ExecutionResult<RuntimeValue> {
        if self.functions.contains_key(name) {
            self.call_function(name, arguments)
        } else if self.tools.contains_key(name) {
            self.call_tool(name, arguments)
        } else {
            Err(ExecutionError::new(format!(
                "unknown function or tool '{}'",
                name
            )))
        }
    }

    fn call_async_task(
        &mut self,
        name: &str,
        arguments: Vec<RuntimeValue>,
    ) -> ExecutionResult<RuntimeValue> {
        let task = self
            .async_tasks
            .get(name)
            .cloned()
            .ok_or_else(|| ExecutionError::new(format!("unknown async task '{}'", name)))?;

        self.call_callable(name, &task, arguments)
    }

    fn tool_as_function(&self, tool: &ToolDeclaration) -> FunctionDeclaration {
        FunctionDeclaration {
            name: tool.name.clone(),
            parameters: tool.parameters.clone(),
            return_type: tool.return_type.clone(),
            body: tool.body.clone(),
            span: tool.span.clone(),
        }
    }

    fn call_callable(
        &mut self,
        name: &str,
        function: &FunctionDeclaration,
        arguments: Vec<RuntimeValue>,
    ) -> ExecutionResult<RuntimeValue> {
        if function.parameters.len() != arguments.len() {
            return Err(ExecutionError::new(format!(
                "call '{}' expected {} arguments, found {}",
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
            StatementControl::Return(value) => Ok(value),
            StatementControl::None => Ok(RuntimeValue::Void),
            StatementControl::Break => Err(ExecutionError::new(
                "break statement escaped function body at runtime",
            )),
            StatementControl::Continue => Err(ExecutionError::new(
                "continue statement escaped function body at runtime",
            )),
        }
    }

    fn execute_block(&mut self, statements: &[Statement]) -> ExecutionResult<StatementControl> {
        for statement in statements {
            match self.execute_statement(statement)? {
                StatementControl::None => {}
                control => return Ok(control),
            }
        }

        Ok(StatementControl::None)
    }

    fn execute_scoped_block(
        &mut self,
        statements: &[Statement],
    ) -> ExecutionResult<StatementControl> {
        self.environment.push_scope();
        let result = self.execute_block(statements);
        self.environment.pop_scope();
        result
    }

    fn evaluate_condition(&mut self, expression: &Expression) -> ExecutionResult<bool> {
        let value = self.evaluate_expression(expression)?;
        let RuntimeValue::Bool(condition) = value else {
            return Err(ExecutionError::new(
                "loop or if condition did not evaluate to a Bool runtime value",
            ));
        };

        Ok(condition)
    }
}

fn python_executable() -> OsString {
    env::var_os("LOZ_PYTHON_PATH")
        .filter(|value| !value.is_empty())
        .or_else(|| find_command_on_path("python3"))
        .or_else(|| find_command_on_path("python"))
        .unwrap_or_else(|| OsString::from("python3"))
}

fn python_bridge_script_path() -> ExecutionResult<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/python_bridge.py");
    if path.is_file() {
        Ok(path)
    } else {
        Err(ExecutionError::new(format!(
            "Python bridge script is missing at '{}'",
            path.display()
        )))
    }
}

fn find_command_on_path(command: &str) -> Option<OsString> {
    let path_value = env::var_os("PATH")?;
    let windows_exts = windows_path_extensions();
    let has_extension = Path::new(command).extension().is_some();

    for directory in env::split_paths(&path_value) {
        let direct = directory.join(command);
        if direct.is_file() {
            return Some(direct.into_os_string());
        }

        if cfg!(target_os = "windows") && !has_extension {
            for extension in &windows_exts {
                let candidate = directory.join(format!("{command}{extension}"));
                if candidate.is_file() {
                    return Some(candidate.into_os_string());
                }
            }
        }
    }

    None
}

fn windows_path_extensions() -> Vec<String> {
    env::var("PATHEXT")
        .ok()
        .map(|value| {
            value
                .split(';')
                .filter(|part| !part.is_empty())
                .map(|part| part.to_string())
                .collect::<Vec<_>>()
        })
        .filter(|extensions| !extensions.is_empty())
        .unwrap_or_else(|| {
            vec![
                ".exe".to_string(),
                ".cmd".to_string(),
                ".bat".to_string(),
                ".com".to_string(),
            ]
        })
}

fn run_llm_request(prompt: &str) -> Result<String, String> {
    let provider = env::var("LOZ_LLM_PROVIDER").unwrap_or_else(|_| "mock".to_string());

    match provider.as_str() {
        "mock" => Ok(mock_llm_response(prompt)),
        "ollama" => run_ollama_request(prompt),
        "github" => run_github_models_request(prompt),
        other => Err(format!("runtime error: unknown LLM provider '{}'", other)),
    }
}

fn mock_llm_response(prompt: &str) -> String {
    env::var("LOZ_LLM_MOCK_RESPONSE").unwrap_or_else(|_| format!("[mock] {prompt}"))
}

fn run_ollama_request(prompt: &str) -> Result<String, String> {
    let base_url =
        env::var("LOZ_OLLAMA_BASE_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());
    let model = env::var("LOZ_MODEL").unwrap_or_else(|_| "qwen2.5:0.5b".to_string());
    let endpoint = format!("{}/api/generate", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("runtime error: failed to create HTTP client: {error}"))?;

    let response = client
        .post(&endpoint)
        .json(&serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false
        }))
        .send()
        .map_err(|error| format!("runtime error: failed to call Ollama at {base_url}: {error}"))?;

    let status = response.status();
    let body_text = response
        .text()
        .map_err(|error| format!("runtime error: failed to call Ollama at {base_url}: {error}"))?;
    let body: JsonValue = serde_json::from_str(&body_text)
        .map_err(|_| "runtime error: invalid LLM provider response".to_string())?;

    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(JsonValue::as_str)
            .unwrap_or("request failed");
        return Err(format!(
            "runtime error: failed to call Ollama at {base_url}: HTTP {status} {detail}"
        ));
    }

    body.get("response")
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "runtime error: invalid LLM provider response".to_string())
}

fn run_github_models_request(prompt: &str) -> Result<String, String> {
    let token_env_name = github_token_env_name();
    let token = env::var(&token_env_name).map_err(|_| {
        format!(
            "runtime error: {} is required for LOZ_LLM_PROVIDER=github",
            token_env_name
        )
    })?;
    let model = env::var("LOZ_MODEL").map_err(|_| {
        "runtime error: LOZ_MODEL is required for LOZ_LLM_PROVIDER=github".to_string()
    })?;
    let base_url = env::var("LOZ_GITHUB_MODELS_BASE_URL")
        .unwrap_or_else(|_| "https://models.github.ai/inference".to_string());
    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("runtime error: failed to create HTTP client: {error}"))?;

    let response = client
        .post(&endpoint)
        .bearer_auth(token)
        .json(&serde_json::json!({
            "model": model,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ]
        }))
        .send()
        .map_err(|error| {
            format!("runtime error: failed to call GitHub Models at {base_url}: {error}")
        })?;

    let status = response.status();
    let body_text = response.text().map_err(|error| {
        format!("runtime error: failed to call GitHub Models at {base_url}: {error}")
    })?;
    let body: JsonValue = serde_json::from_str(&body_text)
        .map_err(|_| "runtime error: invalid LLM provider response".to_string())?;

    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(|value| value.get("message"))
            .and_then(JsonValue::as_str)
            .or_else(|| body.get("message").and_then(JsonValue::as_str))
            .unwrap_or("request failed");
        return Err(format!(
            "runtime error: failed to call GitHub Models at {base_url}: HTTP {status} {detail}"
        ));
    }

    body.get("choices")
        .and_then(JsonValue::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "runtime error: invalid LLM provider response".to_string())
}

fn github_token_env_name() -> String {
    env::var("LOZ_GITHUB_TOKEN_ENV").unwrap_or_else(|_| "GITHUB_TOKEN".to_string())
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn execute(program: &Program) -> ExecutionResult<RuntimeValue> {
    Interpreter::new().execute_program(program)
}
