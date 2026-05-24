use std::collections::{HashMap, HashSet};
use std::fmt;

use loz_ast::{
    AgentDeclaration, AgentTaskDeclaration, AssignmentTarget, AsyncTaskDeclaration,
    AwaitExpression, BinaryOperator, Diagnostic, Expression, ExpressionKind, ForStatement,
    FunctionDeclaration, IfStatement, ImplBlock, ImportDeclaration, MethodCallExpression,
    ModuleDeclaration, Program, SchemaDeclaration, Span, Statement, StructDeclaration,
    ToolDeclaration, TypeName, VariableDeclaration, WhileStatement, WorkflowDeclaration,
    WorkflowTarget,
};

pub type SemanticResult<T> = Result<T, SemanticError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError {
    pub diagnostic: Diagnostic,
}

impl SemanticError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            diagnostic: Diagnostic::error(message),
        }
    }

    fn with_span(mut self, span: Span) -> Self {
        self.diagnostic = self.diagnostic.with_span(span);
        self
    }

    fn at_expression(message: impl Into<String>, expression: &Expression) -> Self {
        Self::new(message).with_span(expression.span.clone())
    }

    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "semantic error: {}", self.diagnostic.message)
    }
}

impl std::error::Error for SemanticError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub type_name: TypeName,
    pub is_mutable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSymbol {
    pub parameter_types: Vec<TypeName>,
    pub return_type: TypeName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExpressionType {
    Concrete(TypeName),
    IntegerLiteral,
    FloatLiteral,
    Array(Box<ExpressionType>, usize),
    EmptyMap,
    EmptySet,
}

#[derive(Debug, Default)]
pub struct SymbolTable {
    scopes: Vec<HashMap<String, Symbol>>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn insert(&mut self, name: String, symbol: Symbol) -> SemanticResult<()> {
        let current_scope = self
            .scopes
            .last_mut()
            .expect("symbol table always has at least one scope");

        if current_scope.contains_key(&name) {
            return Err(SemanticError::new(format!(
                "duplicate variable name '{}' in the same scope",
                name
            )));
        }

        current_scope.insert(name, symbol);
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }
}

pub struct SemanticAnalyzer {
    modules: HashSet<String>,
    symbols: SymbolTable,
    functions: HashMap<String, FunctionSymbol>,
    module_functions: HashMap<(String, String), FunctionSymbol>,
    async_tasks: HashMap<String, FunctionSymbol>,
    tools: HashMap<String, FunctionSymbol>,
    agent_names: HashSet<String>,
    agent_tasks: HashMap<(String, String), FunctionSymbol>,
    workflows: HashSet<String>,
    methods: HashMap<(String, String), FunctionSymbol>,
    structs: HashMap<String, StructDeclaration>,
    schemas: HashMap<String, SchemaDeclaration>,
    loop_depth: usize,
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        let mut functions = HashMap::new();
        let modules = ["io", "json", "schema", "python", "llm"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect();
        functions.insert(
            "print".to_string(),
            FunctionSymbol {
                parameter_types: vec![TypeName::Named("Any".to_string())],
                return_type: TypeName::Void,
            },
        );

        Self {
            modules,
            symbols: SymbolTable::new(),
            functions,
            module_functions: HashMap::new(),
            async_tasks: HashMap::new(),
            tools: HashMap::new(),
            agent_names: HashSet::new(),
            agent_tasks: HashMap::new(),
            workflows: HashSet::new(),
            methods: HashMap::new(),
            structs: HashMap::new(),
            schemas: HashMap::new(),
            loop_depth: 0,
        }
    }

    pub fn analyze(&mut self, program: &Program) -> SemanticResult<()> {
        self.collect_modules(program)?;
        self.validate_imports(program)?;
        self.collect_structs(program)?;
        self.collect_schemas(program)?;
        self.collect_functions(program)?;
        self.collect_async_tasks(program)?;
        self.collect_tools(program)?;
        self.collect_agent_tasks(program)?;
        self.collect_workflows(program)?;
        self.collect_methods(program)?;

        for statement in &program.statements {
            self.analyze_statement(statement, None, true)?;
        }

        Ok(())
    }

    fn collect_modules(&mut self, program: &Program) -> SemanticResult<()> {
        for statement in &program.statements {
            if let Statement::ModuleDeclaration(module_declaration) = statement {
                if !self.modules.insert(module_declaration.name.clone()) {
                    return Err(SemanticError::new(format!(
                        "duplicate module name '{}'",
                        module_declaration.name
                    )));
                }
            }
        }

        Ok(())
    }

    fn validate_imports(&self, program: &Program) -> SemanticResult<()> {
        let mut seen_imports = HashSet::new();

        for statement in &program.statements {
            match statement {
                Statement::ModuleDeclaration(_) => {
                    seen_imports.clear();
                }
                Statement::ImportDeclaration(import_declaration) => {
                    if !seen_imports.insert(import_declaration.module_name.clone()) {
                        return Err(SemanticError::new(format!(
                            "duplicate import '{}'",
                            import_declaration.module_name
                        )));
                    }

                    if !self.modules.contains(&import_declaration.module_name) {
                        return Err(SemanticError::new(format!(
                            "imported module '{}' does not exist",
                            import_declaration.module_name
                        )));
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn collect_structs(&mut self, program: &Program) -> SemanticResult<()> {
        for statement in &program.statements {
            if let Statement::StructDeclaration(struct_declaration) = statement {
                if self
                    .structs
                    .insert(struct_declaration.name.clone(), struct_declaration.clone())
                    .is_some()
                {
                    return Err(SemanticError::new(format!(
                        "duplicate struct name '{}'",
                        struct_declaration.name
                    )));
                }
            }
        }

        Ok(())
    }

    fn collect_functions(&mut self, program: &Program) -> SemanticResult<()> {
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
                let symbol = FunctionSymbol {
                    parameter_types: function
                        .parameters
                        .iter()
                        .map(|parameter| parameter.type_name.clone())
                        .collect(),
                    return_type: function.return_type.clone(),
                };

                if let Some(module_name) = &current_module {
                    if self
                        .module_functions
                        .insert((module_name.clone(), function.name.clone()), symbol.clone())
                        .is_some()
                    {
                        return Err(SemanticError::new(format!(
                            "duplicate function name '{}'",
                            function.name
                        )));
                    }
                }

                if self.functions.contains_key(&function.name)
                    || self.tools.contains_key(&function.name)
                {
                    return Err(SemanticError::new(format!(
                        "duplicate function name '{}'",
                        function.name
                    )));
                }

                self.functions.insert(function.name.clone(), symbol);
            }
        }

        Ok(())
    }

    fn collect_tools(&mut self, program: &Program) -> SemanticResult<()> {
        for statement in &program.statements {
            if let Statement::ToolDeclaration(tool) = statement {
                if self.tools.contains_key(&tool.name) || self.functions.contains_key(&tool.name) {
                    return Err(SemanticError::new(format!(
                        "duplicate tool name '{}'",
                        tool.name
                    )));
                }

                self.tools.insert(
                    tool.name.clone(),
                    FunctionSymbol {
                        parameter_types: tool
                            .parameters
                            .iter()
                            .map(|parameter| parameter.type_name.clone())
                            .collect(),
                        return_type: tool.return_type.clone(),
                    },
                );
            }
        }

        Ok(())
    }

    fn collect_async_tasks(&mut self, program: &Program) -> SemanticResult<()> {
        for statement in &program.statements {
            if let Statement::AsyncTaskDeclaration(task) = statement {
                if self.async_tasks.contains_key(&task.name)
                    || self.functions.contains_key(&task.name)
                    || self.tools.contains_key(&task.name)
                {
                    return Err(SemanticError::new(format!(
                        "duplicate async task name '{}'",
                        task.name
                    )));
                }

                if self.workflows.contains(&task.name)
                    || self.structs.contains_key(&task.name)
                    || self.schemas.contains_key(&task.name)
                    || self.agent_names.contains(&task.name)
                {
                    return Err(SemanticError::new(format!(
                        "async task name '{}' conflicts with an existing top-level declaration",
                        task.name
                    )));
                }

                self.async_tasks.insert(
                    task.name.clone(),
                    FunctionSymbol {
                        parameter_types: task
                            .parameters
                            .iter()
                            .map(|parameter| parameter.type_name.clone())
                            .collect(),
                        return_type: task.return_type.clone(),
                    },
                );
            }
        }

        Ok(())
    }

    fn collect_agent_tasks(&mut self, program: &Program) -> SemanticResult<()> {
        for statement in &program.statements {
            let Statement::AgentDeclaration(agent) = statement else {
                continue;
            };

            if self.async_tasks.contains_key(&agent.name) {
                return Err(SemanticError::new(format!(
                    "agent name '{}' conflicts with an existing async task",
                    agent.name
                )));
            }

            if !self.agent_names.insert(agent.name.clone()) {
                return Err(SemanticError::new(format!(
                    "duplicate agent name '{}'",
                    agent.name
                )));
            }

            let mut seen_task_names = HashSet::new();
            for task in &agent.tasks {
                if !seen_task_names.insert(task.name.clone()) {
                    return Err(SemanticError::new(format!(
                        "duplicate task name '{}' in agent '{}'",
                        task.name, agent.name
                    )));
                }

                let key = (agent.name.clone(), task.name.clone());
                self.agent_tasks.insert(
                    key,
                    FunctionSymbol {
                        parameter_types: task
                            .parameters
                            .iter()
                            .map(|parameter| parameter.type_name.clone())
                            .collect(),
                        return_type: task.return_type.clone(),
                    },
                );
            }
        }

        Ok(())
    }

    fn collect_workflows(&mut self, program: &Program) -> SemanticResult<()> {
        for statement in &program.statements {
            let Statement::WorkflowDeclaration(workflow) = statement else {
                continue;
            };

            if !self.workflows.insert(workflow.name.clone()) {
                return Err(SemanticError::new(format!(
                    "duplicate workflow name '{}'",
                    workflow.name
                )));
            }

            if self.functions.contains_key(&workflow.name)
                || self.async_tasks.contains_key(&workflow.name)
                || self.tools.contains_key(&workflow.name)
                || self.structs.contains_key(&workflow.name)
                || self.schemas.contains_key(&workflow.name)
                || self.agent_names.contains(&workflow.name)
            {
                return Err(SemanticError::new(format!(
                    "workflow name '{}' conflicts with an existing top-level declaration",
                    workflow.name
                )));
            }
        }

        Ok(())
    }

    fn collect_schemas(&mut self, program: &Program) -> SemanticResult<()> {
        for statement in &program.statements {
            if let Statement::SchemaDeclaration(schema_declaration) = statement {
                if self.structs.contains_key(&schema_declaration.name) {
                    return Err(SemanticError::new(format!(
                        "schema name '{}' conflicts with an existing struct",
                        schema_declaration.name
                    )));
                }

                if self
                    .schemas
                    .insert(schema_declaration.name.clone(), schema_declaration.clone())
                    .is_some()
                {
                    return Err(SemanticError::new(format!(
                        "duplicate schema name '{}'",
                        schema_declaration.name
                    )));
                }
            }
        }

        Ok(())
    }

    fn collect_methods(&mut self, program: &Program) -> SemanticResult<()> {
        for statement in &program.statements {
            let Statement::ImplBlock(impl_block) = statement else {
                continue;
            };

            if !self.structs.contains_key(&impl_block.target_name) {
                return Err(SemanticError::new(format!(
                    "impl target '{}' must be a known struct",
                    impl_block.target_name
                )));
            }

            let mut seen_method_names = HashSet::new();
            for method in &impl_block.methods {
                if !seen_method_names.insert(method.name.clone()) {
                    return Err(SemanticError::new(format!(
                        "duplicate method name '{}' in impl '{}'",
                        method.name, impl_block.target_name
                    )));
                }

                let self_parameter = method.parameters.first().ok_or_else(|| {
                    SemanticError::new(format!(
                        "method '{}.{}' must declare self as the first parameter",
                        impl_block.target_name, method.name
                    ))
                })?;
                if self_parameter.name != "self" {
                    return Err(SemanticError::new(format!(
                        "method '{}.{}' must declare self as the first parameter",
                        impl_block.target_name, method.name
                    )));
                }
                if self_parameter.type_name != TypeName::Named(impl_block.target_name.clone()) {
                    return Err(SemanticError::new(format!(
                        "method '{}.{}' self parameter must match impl target type '{}'",
                        impl_block.target_name, method.name, impl_block.target_name
                    )));
                }

                let key = (impl_block.target_name.clone(), method.name.clone());
                if self.methods.contains_key(&key) {
                    return Err(SemanticError::new(format!(
                        "duplicate method name '{}' for struct '{}'",
                        method.name, impl_block.target_name
                    )));
                }

                self.methods.insert(
                    key,
                    FunctionSymbol {
                        parameter_types: method
                            .parameters
                            .iter()
                            .skip(1)
                            .map(|parameter| parameter.type_name.clone())
                            .collect(),
                        return_type: method.return_type.clone(),
                    },
                );
            }
        }

        Ok(())
    }

    fn analyze_statement(
        &mut self,
        statement: &Statement,
        current_return_type: Option<&TypeName>,
        allow_struct_declarations: bool,
    ) -> SemanticResult<()> {
        match statement {
            Statement::ModuleDeclaration(module_declaration) => {
                self.analyze_module_declaration(module_declaration, allow_struct_declarations)
            }
            Statement::ImportDeclaration(import_declaration) => {
                self.analyze_import_declaration(import_declaration, allow_struct_declarations)
            }
            Statement::VariableDeclaration(declaration) => {
                self.analyze_variable_declaration(declaration)
            }
            Statement::FunctionDeclaration(function) => self.analyze_function_declaration(function),
            Statement::AsyncTaskDeclaration(task) => {
                self.analyze_async_task_declaration(task, allow_struct_declarations)
            }
            Statement::ToolDeclaration(tool) => {
                self.analyze_tool_declaration(tool, allow_struct_declarations)
            }
            Statement::AgentDeclaration(agent) => {
                self.analyze_agent_declaration(agent, allow_struct_declarations)
            }
            Statement::WorkflowDeclaration(workflow) => {
                self.analyze_workflow_declaration(workflow, allow_struct_declarations)
            }
            Statement::StructDeclaration(struct_declaration) => {
                self.analyze_struct_declaration(struct_declaration, allow_struct_declarations)
            }
            Statement::SchemaDeclaration(schema_declaration) => {
                self.analyze_schema_declaration(schema_declaration, allow_struct_declarations)
            }
            Statement::ImplBlock(impl_block) => {
                self.analyze_impl_block(impl_block, allow_struct_declarations)
            }
            Statement::If(if_statement) => {
                self.analyze_if_statement(if_statement, current_return_type)
            }
            Statement::While(while_statement) => {
                self.analyze_while_statement(while_statement, current_return_type)
            }
            Statement::For(for_statement) => {
                self.analyze_for_statement(for_statement, current_return_type)
            }
            Statement::Break(_) => self.analyze_break_statement(),
            Statement::Continue(_) => self.analyze_continue_statement(),
            Statement::Return(return_statement) => {
                let expected_type = current_return_type.ok_or_else(|| {
                    SemanticError::new("return statement is not allowed outside a function")
                })?;

                let actual_type = self.infer_expression_type(&return_statement.value)?;
                self.ensure_type_matches_at(
                    expected_type,
                    &actual_type,
                    "return statement",
                    Some(&return_statement.value),
                )
            }
            Statement::Assignment(assignment) => self.analyze_assignment_statement(assignment),
            Statement::Expression(expression) => {
                self.infer_expression_type(expression)?;
                Ok(())
            }
        }
    }

    fn analyze_assignment_statement(
        &self,
        assignment: &loz_ast::AssignmentStatement,
    ) -> SemanticResult<()> {
        match &assignment.target {
            AssignmentTarget::Identifier(name) => {
                let symbol = self.symbols.lookup(name).cloned().ok_or_else(|| {
                    SemanticError::new(format!("assignment to undeclared identifier '{}'", name))
                })?;

                if !symbol.is_mutable {
                    return Err(SemanticError::new(format!(
                        "cannot reassign immutable value '{}'",
                        name
                    )));
                }

                let value_type = self.infer_expression_type(&assignment.value)?;
                self.ensure_type_matches_at(
                    &symbol.type_name,
                    &value_type,
                    "assignment",
                    Some(&assignment.value),
                )
            }
            AssignmentTarget::Dereference(dereference) => {
                let reference_type = self.infer_expression_type(&Expression::new(
                    ExpressionKind::Dereference(dereference.clone()),
                    assignment.span.clone(),
                ))?;
                let ExpressionType::Concrete(reference_inner_type) = reference_type else {
                    return Err(SemanticError::new(
                        "dereference assignment target did not resolve to a concrete type",
                    ));
                };

                let referenced_value_type = self.infer_expression_type(&dereference.value)?;
                let ExpressionType::Concrete(TypeName::Reference {
                    inner: _,
                    is_mutable,
                }) = referenced_value_type
                else {
                    return Err(SemanticError::new(
                        "cannot assign through a non-reference value",
                    ));
                };

                if !is_mutable {
                    return Err(SemanticError::new(
                        "cannot assign through an immutable reference",
                    ));
                }

                let value_type = self.infer_expression_type(&assignment.value)?;
                self.ensure_type_matches(
                    &reference_inner_type,
                    &value_type,
                    "dereference assignment",
                )
            }
            AssignmentTarget::FieldAccess(field_access) => {
                let symbol = self
                    .symbols
                    .lookup(&field_access.base_name)
                    .cloned()
                    .ok_or_else(|| {
                        SemanticError::new(format!(
                            "assignment to undeclared identifier '{}'",
                            field_access.base_name
                        ))
                    })?;

                if !symbol.is_mutable {
                    return Err(SemanticError::new(format!(
                        "cannot assign to field '{}.{}' because '{}' is immutable",
                        field_access.base_name, field_access.field_name, field_access.base_name
                    )));
                }

                let TypeName::Named(struct_name) = symbol.type_name else {
                    return Err(SemanticError::new(format!(
                        "cannot assign to field '{}.{}' on non-struct value",
                        field_access.base_name, field_access.field_name
                    )));
                };

                let struct_declaration = self.structs.get(&struct_name).ok_or_else(|| {
                    SemanticError::new(format!(
                        "unknown struct type '{}' for field assignment",
                        struct_name
                    ))
                })?;

                let field = struct_declaration
                    .fields
                    .iter()
                    .find(|field| field.name == field_access.field_name)
                    .ok_or_else(|| {
                        SemanticError::new(format!(
                            "struct '{}' has no field '{}'",
                            struct_name, field_access.field_name
                        ))
                    })?;

                let value_type = self.infer_expression_type(&assignment.value)?;
                self.ensure_type_matches_at(
                    &field.type_name,
                    &value_type,
                    "field assignment",
                    Some(&assignment.value),
                )
            }
            AssignmentTarget::IndexAccess(index_access) => {
                let symbol = self
                    .symbols
                    .lookup(&index_access.base_name)
                    .cloned()
                    .ok_or_else(|| {
                        SemanticError::new(format!(
                            "assignment to undeclared identifier '{}'",
                            index_access.base_name
                        ))
                    })?;

                if !symbol.is_mutable {
                    return Err(SemanticError::new(format!(
                        "cannot assign to array element '{}[...]' because '{}' is immutable",
                        index_access.base_name, index_access.base_name
                    )));
                }

                let index_type = self.infer_expression_type(&index_access.index)?;
                self.ensure_integer_type(&index_type, "array element assignment index")?;

                let TypeName::Array(element_type, _) = symbol.type_name else {
                    return Err(SemanticError::new(format!(
                        "cannot assign to array element '{}[...]' on non-array value",
                        index_access.base_name
                    )));
                };

                let value_type = self.infer_expression_type(&assignment.value)?;
                self.ensure_type_matches_at(
                    &element_type,
                    &value_type,
                    "array element assignment",
                    Some(&assignment.value),
                )
            }
        }
    }

    fn analyze_variable_declaration(
        &mut self,
        declaration: &VariableDeclaration,
    ) -> SemanticResult<()> {
        let value_type = self.infer_expression_type(&declaration.value)?;
        let type_name = if let Some(type_name) = &declaration.type_name {
            self.ensure_type_matches_at(
                type_name,
                &value_type,
                "variable declaration",
                Some(&declaration.value),
            )?;
            type_name.clone()
        } else {
            self.expression_type_to_type_name(&value_type, "variable declaration")?
        };

        self.symbols.insert(
            declaration.name.clone(),
            Symbol {
                type_name,
                is_mutable: declaration.is_mutable,
            },
        )
    }

    fn analyze_if_statement(
        &mut self,
        if_statement: &IfStatement,
        current_return_type: Option<&TypeName>,
    ) -> SemanticResult<()> {
        let condition_type = self.infer_expression_type(&if_statement.condition)?;
        self.ensure_type_matches_at(
            &TypeName::Bool,
            &condition_type,
            "if condition",
            Some(&if_statement.condition),
        )?;

        self.symbols.push_scope();
        for statement in &if_statement.then_branch {
            self.analyze_statement(statement, current_return_type, false)?;
        }
        self.symbols.pop_scope();

        if let Some(else_branch) = &if_statement.else_branch {
            self.symbols.push_scope();
            for statement in else_branch {
                self.analyze_statement(statement, current_return_type, false)?;
            }
            self.symbols.pop_scope();
        }

        Ok(())
    }

    fn analyze_while_statement(
        &mut self,
        while_statement: &WhileStatement,
        current_return_type: Option<&TypeName>,
    ) -> SemanticResult<()> {
        let condition_type = self.infer_expression_type(&while_statement.condition)?;
        self.ensure_type_matches_at(
            &TypeName::Bool,
            &condition_type,
            "while condition",
            Some(&while_statement.condition),
        )?;

        self.loop_depth += 1;
        self.symbols.push_scope();
        for statement in &while_statement.body {
            self.analyze_statement(statement, current_return_type, false)?;
        }
        self.symbols.pop_scope();
        self.loop_depth -= 1;

        Ok(())
    }

    fn analyze_for_statement(
        &mut self,
        for_statement: &ForStatement,
        current_return_type: Option<&TypeName>,
    ) -> SemanticResult<()> {
        let iterable_type = self.infer_expression_type(&for_statement.iterable)?;
        let element_type = match &iterable_type {
            ExpressionType::Concrete(TypeName::Array(element_type, _)) => (**element_type).clone(),
            ExpressionType::Array(_, _) => {
                let concrete_iterable_type =
                    self.expression_type_to_type_name(&iterable_type, "for loop iterable")?;
                self.array_element_type(&concrete_iterable_type, "for loop iterable")?
            }
            other => {
                return Err(SemanticError::new(format!(
                    "for loop source must be an Array value, found {:?}",
                    other
                )));
            }
        };

        self.loop_depth += 1;
        self.symbols.push_scope();
        self.symbols.insert(
            for_statement.variable_name.clone(),
            Symbol {
                type_name: element_type,
                is_mutable: for_statement.is_mutable,
            },
        )?;

        for statement in &for_statement.body {
            self.analyze_statement(statement, current_return_type, false)?;
        }
        self.symbols.pop_scope();
        self.loop_depth -= 1;

        Ok(())
    }

    fn analyze_break_statement(&self) -> SemanticResult<()> {
        if self.loop_depth == 0 {
            return Err(SemanticError::new(
                "break statement is only allowed inside a loop",
            ));
        }

        Ok(())
    }

    fn analyze_continue_statement(&self) -> SemanticResult<()> {
        if self.loop_depth == 0 {
            return Err(SemanticError::new(
                "continue statement is only allowed inside a loop",
            ));
        }

        Ok(())
    }

    fn array_element_type(&self, type_name: &TypeName, context: &str) -> SemanticResult<TypeName> {
        let TypeName::Array(element_type, _) = type_name else {
            return Err(SemanticError::new(format!(
                "{context} must be an Array type, found {:?}",
                type_name
            )));
        };

        Ok((**element_type).clone())
    }

    fn analyze_function_declaration(
        &mut self,
        function: &FunctionDeclaration,
    ) -> SemanticResult<()> {
        self.ensure_declared_type_exists(
            &function.return_type,
            &format!("return type of function '{}'", function.name),
            true,
        )?;

        self.symbols.push_scope();

        for parameter in &function.parameters {
            self.ensure_declared_type_exists(
                &parameter.type_name,
                &format!(
                    "parameter '{}' in function '{}'",
                    parameter.name, function.name
                ),
                false,
            )?;

            self.symbols.insert(
                parameter.name.clone(),
                Symbol {
                    type_name: parameter.type_name.clone(),
                    is_mutable: false,
                },
            )?;
        }

        let mut saw_return = false;
        for statement in &function.body {
            if matches!(statement, Statement::Return(_)) {
                saw_return = true;
            }

            self.analyze_statement(statement, Some(&function.return_type), false)?;
        }

        self.symbols.pop_scope();

        if function.return_type != TypeName::Void && !saw_return {
            return Err(SemanticError::new(format!(
                "function '{}' is missing a return statement for return type {:?}",
                function.name, function.return_type
            )));
        }

        Ok(())
    }

    fn analyze_tool_declaration(
        &mut self,
        tool: &ToolDeclaration,
        allow_top_level_declarations: bool,
    ) -> SemanticResult<()> {
        if !allow_top_level_declarations {
            return Err(SemanticError::new(format!(
                "tool declaration '{}' is only allowed at the top level",
                tool.name
            )));
        }

        self.ensure_declared_type_exists(
            &tool.return_type,
            &format!("return type of tool '{}'", tool.name),
            true,
        )?;

        self.symbols.push_scope();

        for parameter in &tool.parameters {
            self.ensure_declared_type_exists(
                &parameter.type_name,
                &format!("parameter '{}' in tool '{}'", parameter.name, tool.name),
                false,
            )?;

            self.symbols.insert(
                parameter.name.clone(),
                Symbol {
                    type_name: parameter.type_name.clone(),
                    is_mutable: false,
                },
            )?;
        }

        let mut saw_return = false;
        for statement in &tool.body {
            if matches!(statement, Statement::Return(_)) {
                saw_return = true;
            }

            self.analyze_statement(statement, Some(&tool.return_type), false)?;
        }

        self.symbols.pop_scope();

        if tool.return_type != TypeName::Void && !saw_return {
            return Err(SemanticError::new(format!(
                "tool '{}' is missing a return statement for return type {:?}",
                tool.name, tool.return_type
            )));
        }

        Ok(())
    }

    fn analyze_async_task_declaration(
        &mut self,
        task: &AsyncTaskDeclaration,
        allow_top_level_declarations: bool,
    ) -> SemanticResult<()> {
        if !allow_top_level_declarations {
            return Err(SemanticError::new(format!(
                "async task declaration '{}' is only allowed at the top level",
                task.name
            )));
        }

        self.ensure_declared_type_exists(
            &task.return_type,
            &format!("return type of async task '{}'", task.name),
            true,
        )?;

        self.symbols.push_scope();

        for parameter in &task.parameters {
            self.ensure_declared_type_exists(
                &parameter.type_name,
                &format!(
                    "parameter '{}' in async task '{}'",
                    parameter.name, task.name
                ),
                false,
            )?;

            self.symbols.insert(
                parameter.name.clone(),
                Symbol {
                    type_name: parameter.type_name.clone(),
                    is_mutable: false,
                },
            )?;
        }

        let mut saw_return = false;
        for statement in &task.body {
            if matches!(statement, Statement::Return(_)) {
                saw_return = true;
            }

            self.analyze_statement(statement, Some(&task.return_type), false)?;
        }

        self.symbols.pop_scope();

        if task.return_type != TypeName::Void && !saw_return {
            return Err(SemanticError::new(format!(
                "async task '{}' is missing a return statement for return type {:?}",
                task.name, task.return_type
            )));
        }

        Ok(())
    }

    fn analyze_agent_declaration(
        &mut self,
        agent: &AgentDeclaration,
        allow_top_level_declarations: bool,
    ) -> SemanticResult<()> {
        if !allow_top_level_declarations {
            return Err(SemanticError::new(format!(
                "agent declaration '{}' is only allowed at the top level",
                agent.name
            )));
        }

        for task in &agent.tasks {
            self.analyze_agent_task_declaration(&agent.name, task)?;
        }

        Ok(())
    }

    fn analyze_agent_task_declaration(
        &mut self,
        agent_name: &str,
        task: &AgentTaskDeclaration,
    ) -> SemanticResult<()> {
        self.ensure_declared_type_exists(
            &task.return_type,
            &format!("return type of task '{}.{}'", agent_name, task.name),
            true,
        )?;

        self.symbols.push_scope();

        for parameter in &task.parameters {
            self.ensure_declared_type_exists(
                &parameter.type_name,
                &format!(
                    "parameter '{}' in task '{}.{}'",
                    parameter.name, agent_name, task.name
                ),
                false,
            )?;

            self.symbols.insert(
                parameter.name.clone(),
                Symbol {
                    type_name: parameter.type_name.clone(),
                    is_mutable: false,
                },
            )?;
        }

        let mut saw_return = false;
        for statement in &task.body {
            if matches!(statement, Statement::Return(_)) {
                saw_return = true;
            }

            self.analyze_statement(statement, Some(&task.return_type), false)?;
        }

        self.symbols.pop_scope();

        if task.return_type != TypeName::Void && !saw_return {
            return Err(SemanticError::new(format!(
                "task '{}.{}' is missing a return statement for return type {:?}",
                agent_name, task.name, task.return_type
            )));
        }

        Ok(())
    }

    fn analyze_workflow_declaration(
        &mut self,
        workflow: &WorkflowDeclaration,
        allow_top_level_declarations: bool,
    ) -> SemanticResult<()> {
        if !allow_top_level_declarations {
            return Err(SemanticError::new(format!(
                "workflow declaration '{}' is only allowed at the top level",
                workflow.name
            )));
        }

        if workflow.steps.is_empty() {
            return Err(SemanticError::new(format!(
                "workflow '{}' must contain at least one step",
                workflow.name
            )));
        }

        let mut seen_step_names = HashSet::new();
        for step in &workflow.steps {
            if !seen_step_names.insert(step.name.clone()) {
                return Err(SemanticError::new(format!(
                    "duplicate step '{}' in workflow '{}'",
                    step.name, workflow.name
                )));
            }

            self.validate_workflow_step(workflow, step)?;
        }

        Ok(())
    }

    fn validate_workflow_step(
        &self,
        workflow: &WorkflowDeclaration,
        step: &loz_ast::WorkflowStep,
    ) -> SemanticResult<()> {
        let (parameter_types, return_type, unknown_target_message) = match &step.target {
            WorkflowTarget::FunctionOrTool(target_name) => {
                let symbol = self
                    .functions
                    .get(target_name)
                    .or_else(|| self.tools.get(target_name))
                    .ok_or_else(|| {
                        SemanticError::new(format!(
                            "workflow '{}' step '{}' refers to unknown function/tool '{}'",
                            workflow.name, step.name, target_name
                        ))
                    })?;
                (
                    &symbol.parameter_types,
                    &symbol.return_type,
                    format!(
                        "workflow '{}' step '{}' refers to unknown function/tool '{}'",
                        workflow.name, step.name, target_name
                    ),
                )
            }
            WorkflowTarget::AgentTask {
                agent_name,
                task_name,
            } => {
                let symbol = self
                    .agent_tasks
                    .get(&(agent_name.clone(), task_name.clone()))
                    .ok_or_else(|| {
                        SemanticError::new(format!(
                            "workflow '{}' step '{}' refers to unknown agent task '{}.{}'",
                            workflow.name, step.name, agent_name, task_name
                        ))
                    })?;
                (
                    &symbol.parameter_types,
                    &symbol.return_type,
                    format!(
                        "workflow '{}' step '{}' refers to unknown agent task '{}.{}'",
                        workflow.name, step.name, agent_name, task_name
                    ),
                )
            }
        };

        if !parameter_types.is_empty() {
            return Err(SemanticError::new(format!(
                "workflow step '{}' must refer to a zero-argument function/tool in this phase",
                step.name
            )));
        }

        if !self.is_supported_workflow_step_return_type(return_type) {
            return Err(SemanticError::new(format!(
                "workflow step '{}' must return Void, Text, i32, i64, f64, Bool, or Json in this phase",
                step.name
            )));
        }

        let _ = unknown_target_message;
        Ok(())
    }

    fn is_supported_workflow_step_return_type(&self, return_type: &TypeName) -> bool {
        matches!(
            return_type,
            TypeName::Void
                | TypeName::Text
                | TypeName::I32
                | TypeName::I64
                | TypeName::F64
                | TypeName::Bool
                | TypeName::Json
        )
    }

    fn analyze_impl_block(
        &mut self,
        impl_block: &ImplBlock,
        allow_top_level_declarations: bool,
    ) -> SemanticResult<()> {
        if !allow_top_level_declarations {
            return Err(SemanticError::new(format!(
                "impl block for '{}' is only allowed at the top level",
                impl_block.target_name
            )));
        }

        for method in &impl_block.methods {
            self.analyze_method_declaration(&impl_block.target_name, method)?;
        }

        Ok(())
    }

    fn analyze_method_declaration(
        &mut self,
        target_name: &str,
        method: &FunctionDeclaration,
    ) -> SemanticResult<()> {
        self.ensure_declared_type_exists(
            &method.return_type,
            &format!("return type of method '{}.{}'", target_name, method.name),
            true,
        )?;

        let self_parameter = method.parameters.first().ok_or_else(|| {
            SemanticError::new(format!(
                "method '{}.{}' must declare self as the first parameter",
                target_name, method.name
            ))
        })?;
        if self_parameter.name != "self" {
            return Err(SemanticError::new(format!(
                "method '{}.{}' must declare self as the first parameter",
                target_name, method.name
            )));
        }
        if self_parameter.type_name != TypeName::Named(target_name.to_string()) {
            return Err(SemanticError::new(format!(
                "method '{}.{}' self parameter must match impl target type '{}'",
                target_name, method.name, target_name
            )));
        }

        self.symbols.push_scope();
        self.symbols.insert(
            "self".to_string(),
            Symbol {
                type_name: TypeName::Named(target_name.to_string()),
                is_mutable: false,
            },
        )?;

        for parameter in method.parameters.iter().skip(1) {
            self.ensure_declared_type_exists(
                &parameter.type_name,
                &format!(
                    "parameter '{}' in method '{}.{}'",
                    parameter.name, target_name, method.name
                ),
                false,
            )?;

            self.symbols.insert(
                parameter.name.clone(),
                Symbol {
                    type_name: parameter.type_name.clone(),
                    is_mutable: false,
                },
            )?;
        }

        let mut saw_return = false;
        for statement in &method.body {
            if matches!(statement, Statement::Return(_)) {
                saw_return = true;
            }

            self.analyze_statement(statement, Some(&method.return_type), false)?;
        }

        self.symbols.pop_scope();

        if method.return_type != TypeName::Void && !saw_return {
            return Err(SemanticError::new(format!(
                "method '{}.{}' is missing a return statement for return type {:?}",
                target_name, method.name, method.return_type
            )));
        }

        Ok(())
    }

    fn analyze_module_declaration(
        &self,
        module_declaration: &ModuleDeclaration,
        allow_top_level_declarations: bool,
    ) -> SemanticResult<()> {
        if !allow_top_level_declarations {
            return Err(SemanticError::new(format!(
                "module declaration '{}' is only allowed at the top level",
                module_declaration.name
            )));
        }

        Ok(())
    }

    fn analyze_import_declaration(
        &self,
        import_declaration: &ImportDeclaration,
        allow_top_level_declarations: bool,
    ) -> SemanticResult<()> {
        if !allow_top_level_declarations {
            return Err(SemanticError::new(format!(
                "import declaration '{}' is only allowed at the top level",
                import_declaration.module_name
            )));
        }

        Ok(())
    }

    fn analyze_struct_declaration(
        &self,
        struct_declaration: &StructDeclaration,
        allow_struct_declarations: bool,
    ) -> SemanticResult<()> {
        if !allow_struct_declarations {
            return Err(SemanticError::new(format!(
                "struct declaration '{}' is only allowed at the top level",
                struct_declaration.name
            )));
        }

        let mut field_names = HashSet::new();
        for field in &struct_declaration.fields {
            if !field_names.insert(field.name.clone()) {
                return Err(SemanticError::new(format!(
                    "duplicate field name '{}' in struct '{}'",
                    field.name, struct_declaration.name
                )));
            }

            self.ensure_declared_type_exists(
                &field.type_name,
                &format!(
                    "field '{}' in struct '{}'",
                    field.name, struct_declaration.name
                ),
                false,
            )?;
        }

        Ok(())
    }

    fn analyze_schema_declaration(
        &self,
        schema_declaration: &SchemaDeclaration,
        allow_top_level_declarations: bool,
    ) -> SemanticResult<()> {
        if !allow_top_level_declarations {
            return Err(SemanticError::new(format!(
                "schema declaration '{}' is only allowed at the top level",
                schema_declaration.name
            )));
        }

        let mut field_names = HashSet::new();
        for field in &schema_declaration.fields {
            if !field_names.insert(field.name.clone()) {
                return Err(SemanticError::new(format!(
                    "duplicate field name '{}' in schema '{}'",
                    field.name, schema_declaration.name
                )));
            }

            self.ensure_supported_schema_field_type(
                &field.type_name,
                &format!(
                    "field '{}' in schema '{}'",
                    field.name, schema_declaration.name
                ),
            )?;
        }

        Ok(())
    }

    fn infer_expression_type(&self, expression: &Expression) -> SemanticResult<ExpressionType> {
        match &expression.kind {
            ExpressionKind::IntegerLiteral(_) => Ok(ExpressionType::IntegerLiteral),
            ExpressionKind::FloatLiteral(_) => Ok(ExpressionType::FloatLiteral),
            ExpressionKind::BooleanLiteral(_) => Ok(ExpressionType::Concrete(TypeName::Bool)),
            ExpressionKind::StringLiteral(_) => Ok(ExpressionType::Concrete(TypeName::Text)),
            ExpressionKind::Await(await_expression) => {
                self.infer_await_expression_type(await_expression)
            }
            ExpressionKind::Reference(reference_expression) => {
                let symbol = self
                    .symbols
                    .lookup(&reference_expression.target_name)
                    .cloned()
                    .ok_or_else(|| {
                        SemanticError::at_expression(
                            format!(
                                "cannot reference undeclared identifier '{}'",
                                reference_expression.target_name
                            ),
                            expression,
                        )
                    })?;

                if reference_expression.is_mutable && !symbol.is_mutable {
                    return Err(SemanticError::at_expression(
                        format!(
                            "cannot create mutable reference to immutable value '{}'",
                            reference_expression.target_name
                        ),
                        expression,
                    ));
                }

                Ok(ExpressionType::Concrete(TypeName::Reference {
                    inner: Box::new(symbol.type_name),
                    is_mutable: reference_expression.is_mutable,
                }))
            }
            ExpressionKind::Dereference(dereference_expression) => {
                let reference_type = self.infer_expression_type(&dereference_expression.value)?;
                let ExpressionType::Concrete(TypeName::Reference { inner, .. }) = reference_type
                else {
                    return Err(SemanticError::at_expression(
                        "cannot dereference a non-reference value",
                        expression,
                    ));
                };

                Ok(ExpressionType::Concrete(*inner))
            }
            ExpressionKind::Cast(cast_expression) => {
                self.ensure_declared_type_exists(
                    &cast_expression.target_type,
                    "cast target",
                    false,
                )?;
                let source_type = self.infer_expression_type(&cast_expression.value)?;
                self.ensure_legal_explicit_cast(
                    &source_type,
                    &cast_expression.target_type,
                    "cast expression",
                )?;
                Ok(ExpressionType::Concrete(
                    cast_expression.target_type.clone(),
                ))
            }
            ExpressionKind::Identifier(name) => self
                .symbols
                .lookup(name)
                .map(|symbol| ExpressionType::Concrete(symbol.type_name.clone()))
                .ok_or_else(|| {
                    SemanticError::at_expression(
                        format!("unknown identifier '{}'", name),
                        expression,
                    )
                }),
            ExpressionKind::ArrayLiteral(array_literal) => {
                let first_element = array_literal.elements.first().ok_or_else(|| {
                    SemanticError::at_expression(
                        "empty array literals are not supported in this phase",
                        expression,
                    )
                })?;
                let element_type = self.infer_expression_type(first_element)?;

                for element in array_literal.elements.iter().skip(1) {
                    let actual_type = self.infer_expression_type(element)?;
                    self.ensure_expression_types_compatible(
                        &element_type,
                        &actual_type,
                        "array literal",
                    )?;
                }

                Ok(ExpressionType::Array(
                    Box::new(element_type),
                    array_literal.elements.len(),
                ))
            }
            ExpressionKind::IndexAccess(index_access) => {
                let base_type = self
                    .symbols
                    .lookup(&index_access.base_name)
                    .map(|symbol| symbol.type_name.clone())
                    .ok_or_else(|| {
                        SemanticError::at_expression(
                            format!("unknown identifier '{}'", index_access.base_name),
                            expression,
                        )
                    })?;

                let index_type = self.infer_expression_type(&index_access.index)?;
                self.ensure_integer_type(&index_type, "array index")?;

                let TypeName::Array(element_type, _) = base_type else {
                    return Err(SemanticError::at_expression(
                        format!("cannot index non-array value '{}'", index_access.base_name),
                        expression,
                    ));
                };

                Ok(ExpressionType::Concrete(*element_type))
            }
            ExpressionKind::FieldAccess(field_access) => {
                let base_type = self
                    .symbols
                    .lookup(&field_access.base_name)
                    .map(|symbol| symbol.type_name.clone())
                    .ok_or_else(|| {
                        SemanticError::at_expression(
                            format!("unknown identifier '{}'", field_access.base_name),
                            expression,
                        )
                    })?;

                let TypeName::Named(struct_name) = base_type else {
                    return Err(SemanticError::at_expression(
                        format!(
                            "cannot access field '{}' on non-struct value '{}'",
                            field_access.field_name, field_access.base_name
                        ),
                        expression,
                    ));
                };

                let struct_declaration = self.structs.get(&struct_name).ok_or_else(|| {
                    SemanticError::at_expression(
                        format!("unknown struct type '{}' for field access", struct_name),
                        expression,
                    )
                })?;

                let field = struct_declaration
                    .fields
                    .iter()
                    .find(|field| field.name == field_access.field_name)
                    .ok_or_else(|| {
                        SemanticError::at_expression(
                            format!(
                                "struct '{}' has no field '{}'",
                                struct_name, field_access.field_name
                            ),
                            expression,
                        )
                    })?;

                Ok(ExpressionType::Concrete(field.type_name.clone()))
            }
            ExpressionKind::MethodCall(method_call) => {
                self.infer_method_call_type(method_call, expression)
            }
            ExpressionKind::Call(call) => {
                if call.callee == "Map" {
                    if !call.arguments.is_empty() {
                        return Err(SemanticError::at_expression(
                            format!(
                                "built-in constructor 'Map()' expects 0 arguments, found {}",
                                call.arguments.len()
                            ),
                            expression,
                        ));
                    }

                    return Ok(ExpressionType::EmptyMap);
                }

                if call.callee == "Set" {
                    if !call.arguments.is_empty() {
                        return Err(SemanticError::at_expression(
                            format!(
                                "built-in constructor 'Set()' expects 0 arguments, found {}",
                                call.arguments.len()
                            ),
                            expression,
                        ));
                    }

                    return Ok(ExpressionType::EmptySet);
                }

                if let Some(struct_declaration) = self.structs.get(&call.callee) {
                    if struct_declaration.fields.len() != call.arguments.len() {
                        return Err(SemanticError::at_expression(
                            format!(
                                "struct constructor '{}' expected {} arguments, found {}",
                                call.callee,
                                struct_declaration.fields.len(),
                                call.arguments.len()
                            ),
                            expression,
                        ));
                    }

                    for (argument, field) in
                        call.arguments.iter().zip(struct_declaration.fields.iter())
                    {
                        let argument_type = self.infer_expression_type(argument)?;
                        self.ensure_type_matches_at(
                            &field.type_name,
                            &argument_type,
                            "struct construction",
                            Some(argument),
                        )?;
                    }

                    return Ok(ExpressionType::Concrete(TypeName::Named(
                        struct_declaration.name.clone(),
                    )));
                }

                if self.async_tasks.contains_key(&call.callee) {
                    return Err(SemanticError::at_expression(
                        format!("async task '{}' must be awaited", call.callee),
                        expression,
                    ));
                }

                let function = if let Some(function) = self.functions.get(&call.callee) {
                    function
                } else if let Some(tool) = self.tools.get(&call.callee) {
                    tool
                } else {
                    return Err(SemanticError::at_expression(
                        format!("undeclared function or tool '{}'", call.callee),
                        expression,
                    ));
                };

                let accepts_any = function.parameter_types.len() == 1
                    && function.parameter_types[0] == TypeName::Named("Any".to_string())
                    && call.callee == "print";

                if !accepts_any && function.parameter_types.len() != call.arguments.len() {
                    return Err(SemanticError::at_expression(
                        format!(
                            "call '{}' expected {} arguments, found {}",
                            call.callee,
                            function.parameter_types.len(),
                            call.arguments.len()
                        ),
                        expression,
                    ));
                }

                if !accepts_any {
                    for (argument, parameter_type) in
                        call.arguments.iter().zip(function.parameter_types.iter())
                    {
                        let argument_type = self.infer_expression_type(argument)?;
                        self.ensure_type_matches_at(
                            parameter_type,
                            &argument_type,
                            "call",
                            Some(argument),
                        )?;
                    }
                } else if call.arguments.len() != 1 {
                    return Err(SemanticError::at_expression(
                        "built-in function 'print' expects exactly one argument",
                        expression,
                    ));
                } else {
                    self.infer_expression_type(&call.arguments[0])?;
                }

                Ok(ExpressionType::Concrete(function.return_type.clone()))
            }
            ExpressionKind::Binary(binary) => {
                let left_type = self.infer_expression_type(&binary.left)?;
                let right_type = self.infer_expression_type(&binary.right)?;

                match binary.operator {
                    BinaryOperator::Add
                    | BinaryOperator::Subtract
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide => self
                        .resolve_numeric_binary_type(&left_type, &right_type, "binary expression")
                        .map(ExpressionType::Concrete),
                    BinaryOperator::Greater
                    | BinaryOperator::Less
                    | BinaryOperator::GreaterEqual
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Equal
                    | BinaryOperator::NotEqual => {
                        self.validate_comparison_operands(&left_type, &right_type, binary)?;
                        Ok(ExpressionType::Concrete(TypeName::Bool))
                    }
                }
            }
        }
    }

    fn infer_await_expression_type(
        &self,
        await_expression: &AwaitExpression,
    ) -> SemanticResult<ExpressionType> {
        let ExpressionKind::Call(call) = &await_expression.expression.kind else {
            return Err(SemanticError::new(
                "await currently requires an async task call expression",
            ));
        };

        let task = self.async_tasks.get(&call.callee).ok_or_else(|| {
            if self.functions.contains_key(&call.callee) || self.tools.contains_key(&call.callee) {
                SemanticError::new(format!("cannot await non-async call '{}'", call.callee))
            } else {
                SemanticError::new(format!(
                    "await target '{}' must be an async task call",
                    call.callee
                ))
            }
        })?;

        if task.parameter_types.len() != call.arguments.len() {
            return Err(SemanticError::new(format!(
                "call '{}' expected {} arguments, found {}",
                call.callee,
                task.parameter_types.len(),
                call.arguments.len()
            )));
        }

        for (argument, parameter_type) in call.arguments.iter().zip(task.parameter_types.iter()) {
            let argument_type = self.infer_expression_type(argument)?;
            self.ensure_type_matches_at(
                parameter_type,
                &argument_type,
                "await call",
                Some(argument),
            )?;
        }

        Ok(ExpressionType::Concrete(task.return_type.clone()))
    }

    fn infer_method_call_type(
        &self,
        method_call: &MethodCallExpression,
        expression: &Expression,
    ) -> SemanticResult<ExpressionType> {
        if method_call.base_name == "io" {
            return self.infer_io_method_call_type(method_call);
        }
        if method_call.base_name == "json" {
            return self.infer_json_method_call_type(method_call);
        }
        if method_call.base_name == "schema" {
            return self.infer_schema_method_call_type(method_call);
        }
        if method_call.base_name == "python" {
            return self.infer_python_method_call_type(method_call);
        }
        if method_call.base_name == "llm" {
            return self.infer_llm_method_call_type(method_call);
        }

        if let Some(function) = self.module_functions.get(&(
            method_call.base_name.clone(),
            method_call.method_name.clone(),
        )) {
            if function.parameter_types.len() != method_call.arguments.len() {
                return Err(SemanticError::at_expression(
                    format!(
                        "call '{}.{}' expected {} arguments, found {}",
                        method_call.base_name,
                        method_call.method_name,
                        function.parameter_types.len(),
                        method_call.arguments.len()
                    ),
                    expression,
                ));
            }

            for (argument, parameter_type) in method_call
                .arguments
                .iter()
                .zip(function.parameter_types.iter())
            {
                let argument_type = self.infer_expression_type(argument)?;
                self.ensure_type_matches_at(
                    parameter_type,
                    &argument_type,
                    "module call",
                    Some(argument),
                )?;
            }

            return Ok(ExpressionType::Concrete(function.return_type.clone()));
        }

        if self.modules.contains(&method_call.base_name) {
            return Err(SemanticError::at_expression(
                format!(
                    "module '{}' has no function '{}'",
                    method_call.base_name, method_call.method_name
                ),
                expression,
            ));
        }

        let symbol = self
            .symbols
            .lookup(&method_call.base_name)
            .cloned()
            .ok_or_else(|| {
                SemanticError::new(format!("undeclared identifier '{}'", method_call.base_name))
            })?;

        match symbol.type_name {
            TypeName::Array(element_type, _) => match method_call.method_name.as_str() {
                "len" => {
                    if !method_call.arguments.is_empty() {
                        return Err(SemanticError::new(format!(
                            "array method 'len()' expects 0 arguments, found {}",
                            method_call.arguments.len()
                        )));
                    }

                    Ok(ExpressionType::Concrete(TypeName::U64))
                }
                "push" => {
                    if !symbol.is_mutable {
                        return Err(SemanticError::new(format!(
                            "cannot call push() on immutable array '{}'",
                            method_call.base_name
                        )));
                    }

                    if method_call.arguments.len() != 1 {
                        return Err(SemanticError::new(format!(
                            "array method 'push()' expects 1 argument, found {}",
                            method_call.arguments.len()
                        )));
                    }

                    let argument_type = self.infer_expression_type(&method_call.arguments[0])?;
                    self.ensure_type_matches(&element_type, &argument_type, "array push")?;
                    Ok(ExpressionType::Concrete(TypeName::Void))
                }
                "pop" => {
                    if !symbol.is_mutable {
                        return Err(SemanticError::new(format!(
                            "cannot call pop() on immutable array '{}'",
                            method_call.base_name
                        )));
                    }

                    if !method_call.arguments.is_empty() {
                        return Err(SemanticError::new(format!(
                            "array method 'pop()' expects 0 arguments, found {}",
                            method_call.arguments.len()
                        )));
                    }

                    Ok(ExpressionType::Concrete(*element_type))
                }
                other => Err(SemanticError::new(format!(
                    "unknown array method '{}()' on '{}'",
                    other, method_call.base_name
                ))),
            },
            TypeName::Named(struct_name) => {
                let method = self
                    .methods
                    .get(&(struct_name.clone(), method_call.method_name.clone()))
                    .ok_or_else(|| {
                        SemanticError::new(format!(
                            "struct '{}' has no method '{}()'",
                            struct_name, method_call.method_name
                        ))
                    })?;

                if method.parameter_types.len() != method_call.arguments.len() {
                    return Err(SemanticError::new(format!(
                        "method '{}.{}()' expected {} arguments, found {}",
                        struct_name,
                        method_call.method_name,
                        method.parameter_types.len(),
                        method_call.arguments.len()
                    )));
                }

                for (argument, parameter_type) in method_call
                    .arguments
                    .iter()
                    .zip(method.parameter_types.iter())
                {
                    let argument_type = self.infer_expression_type(argument)?;
                    self.ensure_type_matches(parameter_type, &argument_type, "method call")?;
                }

                Ok(ExpressionType::Concrete(method.return_type.clone()))
            }
            TypeName::Map(key_type, value_type) => match method_call.method_name.as_str() {
                "insert" => {
                    if !symbol.is_mutable {
                        return Err(SemanticError::new(format!(
                            "cannot call insert() on immutable map '{}'",
                            method_call.base_name
                        )));
                    }

                    if method_call.arguments.len() != 2 {
                        return Err(SemanticError::new(format!(
                            "map method 'insert()' expects 2 arguments, found {}",
                            method_call.arguments.len()
                        )));
                    }

                    self.ensure_supported_map_key_type(&key_type, "map key")?;
                    let key_argument_type =
                        self.infer_expression_type(&method_call.arguments[0])?;
                    self.ensure_type_matches(&key_type, &key_argument_type, "map insert key")?;
                    let value_argument_type =
                        self.infer_expression_type(&method_call.arguments[1])?;
                    self.ensure_type_matches(
                        &value_type,
                        &value_argument_type,
                        "map insert value",
                    )?;

                    Ok(ExpressionType::Concrete(TypeName::Void))
                }
                "get" => {
                    if method_call.arguments.len() != 1 {
                        return Err(SemanticError::new(format!(
                            "map method 'get()' expects 1 argument, found {}",
                            method_call.arguments.len()
                        )));
                    }

                    self.ensure_supported_map_key_type(&key_type, "map key")?;
                    let key_argument_type =
                        self.infer_expression_type(&method_call.arguments[0])?;
                    self.ensure_type_matches(&key_type, &key_argument_type, "map get key")?;

                    Ok(ExpressionType::Concrete(*value_type))
                }
                "contains" => {
                    if method_call.arguments.len() != 1 {
                        return Err(SemanticError::new(format!(
                            "map method 'contains()' expects 1 argument, found {}",
                            method_call.arguments.len()
                        )));
                    }

                    self.ensure_supported_map_key_type(&key_type, "map key")?;
                    let key_argument_type =
                        self.infer_expression_type(&method_call.arguments[0])?;
                    self.ensure_type_matches(&key_type, &key_argument_type, "map contains key")?;

                    Ok(ExpressionType::Concrete(TypeName::Bool))
                }
                other => Err(SemanticError::new(format!(
                    "unknown map method '{}()' on '{}'",
                    other, method_call.base_name
                ))),
            },
            TypeName::Set(element_type) => match method_call.method_name.as_str() {
                "add" => {
                    if !symbol.is_mutable {
                        return Err(SemanticError::new(format!(
                            "cannot call add() on immutable set '{}'",
                            method_call.base_name
                        )));
                    }

                    if method_call.arguments.len() != 1 {
                        return Err(SemanticError::new(format!(
                            "set method 'add()' expects 1 argument, found {}",
                            method_call.arguments.len()
                        )));
                    }

                    self.ensure_supported_set_element_type(&element_type, "set element")?;
                    let argument_type = self.infer_expression_type(&method_call.arguments[0])?;
                    self.ensure_type_matches(&element_type, &argument_type, "set add element")?;

                    Ok(ExpressionType::Concrete(TypeName::Void))
                }
                "contains" => {
                    if method_call.arguments.len() != 1 {
                        return Err(SemanticError::new(format!(
                            "set method 'contains()' expects 1 argument, found {}",
                            method_call.arguments.len()
                        )));
                    }

                    self.ensure_supported_set_element_type(&element_type, "set element")?;
                    let argument_type = self.infer_expression_type(&method_call.arguments[0])?;
                    self.ensure_type_matches(
                        &element_type,
                        &argument_type,
                        "set contains element",
                    )?;

                    Ok(ExpressionType::Concrete(TypeName::Bool))
                }
                other => Err(SemanticError::new(format!(
                    "unknown set method '{}()' on '{}'",
                    other, method_call.base_name
                ))),
            },
            other => {
                let message = match method_call.method_name.as_str() {
                    "len" | "push" | "pop" => format!(
                        "cannot call array method '{}()' on non-array value '{}'",
                        method_call.method_name, method_call.base_name
                    ),
                    "insert" | "get" => format!(
                        "cannot call map method '{}()' on non-map value '{}'",
                        method_call.method_name, method_call.base_name
                    ),
                    "add" => format!(
                        "cannot call set method '{}()' on non-set value '{}'",
                        method_call.method_name, method_call.base_name
                    ),
                    "contains" => format!(
                        "cannot call contains() on non-map/non-set value '{}'",
                        method_call.base_name
                    ),
                    _ => format!(
                        "cannot call method '{}()' on non-struct/non-array/non-map/non-set value '{}' of type {:?}",
                        method_call.method_name, method_call.base_name, other
                    ),
                };

                Err(SemanticError::new(message))
            }
        }
    }

    fn infer_io_method_call_type(
        &self,
        method_call: &MethodCallExpression,
    ) -> SemanticResult<ExpressionType> {
        if !method_call.arguments.is_empty() {
            return Err(SemanticError::new(format!(
                "io.{}() expects 0 arguments, found {}",
                method_call.method_name,
                method_call.arguments.len()
            )));
        }

        let return_type = match method_call.method_name.as_str() {
            "read_line" => TypeName::Text,
            "read_i8" => TypeName::I8,
            "read_i16" => TypeName::I16,
            "read_i32" => TypeName::I32,
            "read_i64" => TypeName::I64,
            "read_u8" => TypeName::U8,
            "read_u16" => TypeName::U16,
            "read_u32" => TypeName::U32,
            "read_u64" => TypeName::U64,
            "read_f32" => TypeName::F32,
            "read_f64" => TypeName::F64,
            "read_bool" => TypeName::Bool,
            other => {
                return Err(SemanticError::new(format!(
                    "unknown io method '{}()'",
                    other
                )));
            }
        };

        Ok(ExpressionType::Concrete(return_type))
    }

    fn infer_json_method_call_type(
        &self,
        method_call: &MethodCallExpression,
    ) -> SemanticResult<ExpressionType> {
        let (expected_argument_types, return_type) = match method_call.method_name.as_str() {
            "parse" => (vec![TypeName::Text], TypeName::Json),
            "stringify" => (vec![TypeName::Json], TypeName::Text),
            "get_text" => (vec![TypeName::Json, TypeName::Text], TypeName::Text),
            "get_i32" => (vec![TypeName::Json, TypeName::Text], TypeName::I32),
            "get_i64" => (vec![TypeName::Json, TypeName::Text], TypeName::I64),
            "get_f64" => (vec![TypeName::Json, TypeName::Text], TypeName::F64),
            "get_bool" => (vec![TypeName::Json, TypeName::Text], TypeName::Bool),
            "has" => (vec![TypeName::Json, TypeName::Text], TypeName::Bool),
            other => {
                return Err(SemanticError::new(format!(
                    "unknown json method '{}()'",
                    other
                )));
            }
        };

        if method_call.arguments.len() != expected_argument_types.len() {
            return Err(SemanticError::new(format!(
                "json.{}() expects {} arguments, found {}",
                method_call.method_name,
                expected_argument_types.len(),
                method_call.arguments.len()
            )));
        }

        for (argument, expected_type) in method_call
            .arguments
            .iter()
            .zip(expected_argument_types.iter())
        {
            let argument_type = self.infer_expression_type(argument)?;
            self.ensure_type_matches(expected_type, &argument_type, "json method call")?;
        }

        Ok(ExpressionType::Concrete(return_type))
    }

    fn infer_schema_method_call_type(
        &self,
        method_call: &MethodCallExpression,
    ) -> SemanticResult<ExpressionType> {
        let return_type = match method_call.method_name.as_str() {
            "validate" => TypeName::Bool,
            "require" => TypeName::Json,
            other => {
                return Err(SemanticError::new(format!(
                    "unknown schema method '{}()'",
                    other
                )));
            }
        };

        if method_call.arguments.len() != 2 {
            return Err(SemanticError::new(format!(
                "schema.{}() expects 2 arguments, found {}",
                method_call.method_name,
                method_call.arguments.len()
            )));
        }

        let ExpressionKind::StringLiteral(schema_name) = &method_call.arguments[0].kind else {
            return Err(SemanticError::new(format!(
                "schema.{}() requires a schema name string literal as the first argument",
                method_call.method_name
            )));
        };
        if !self.schemas.contains_key(schema_name) {
            return Err(SemanticError::new(format!(
                "unknown schema name '{}'",
                schema_name
            )));
        }

        let json_argument_type = self.infer_expression_type(&method_call.arguments[1])?;
        self.ensure_type_matches(&TypeName::Json, &json_argument_type, "schema method call")?;

        Ok(ExpressionType::Concrete(return_type))
    }

    fn infer_python_method_call_type(
        &self,
        method_call: &MethodCallExpression,
    ) -> SemanticResult<ExpressionType> {
        if method_call.method_name != "call" {
            return Err(SemanticError::new(format!(
                "unknown python method '{}()'",
                method_call.method_name
            )));
        }

        if method_call.arguments.len() != 2 {
            return Err(SemanticError::new(format!(
                "python.call() expects 2 arguments, found {}",
                method_call.arguments.len()
            )));
        }

        let path_type = self.infer_expression_type(&method_call.arguments[0])?;
        self.ensure_type_matches(&TypeName::Text, &path_type, "python method call")?;

        let input_type = self.infer_expression_type(&method_call.arguments[1])?;
        self.ensure_type_matches(&TypeName::Json, &input_type, "python method call")?;

        Ok(ExpressionType::Concrete(TypeName::Json))
    }

    fn infer_llm_method_call_type(
        &self,
        method_call: &MethodCallExpression,
    ) -> SemanticResult<ExpressionType> {
        if method_call.method_name != "ask" {
            return Err(SemanticError::new(format!(
                "unknown llm method '{}()'",
                method_call.method_name
            )));
        }

        if method_call.arguments.len() != 1 {
            return Err(SemanticError::new(format!(
                "llm.ask() expects 1 argument, found {}",
                method_call.arguments.len()
            )));
        }

        let prompt_type = self.infer_expression_type(&method_call.arguments[0])?;
        self.ensure_type_matches(&TypeName::Text, &prompt_type, "llm method call")?;

        Ok(ExpressionType::Concrete(TypeName::Text))
    }

    fn ensure_type_matches(
        &self,
        expected: &TypeName,
        actual: &ExpressionType,
        context: &str,
    ) -> SemanticResult<()> {
        self.ensure_type_matches_with_span(expected, actual, context, None)
    }

    fn ensure_type_matches_at(
        &self,
        expected: &TypeName,
        actual: &ExpressionType,
        context: &str,
        expression: Option<&Expression>,
    ) -> SemanticResult<()> {
        self.ensure_type_matches_with_span(
            expected,
            actual,
            context,
            expression.map(|expression| expression.span.clone()),
        )
    }

    fn ensure_type_matches_with_span(
        &self,
        expected: &TypeName,
        actual: &ExpressionType,
        context: &str,
        span: Option<Span>,
    ) -> SemanticResult<()> {
        match (expected, actual) {
            (
                TypeName::Array(expected_element, expected_length),
                ExpressionType::Array(actual_element, actual_length),
            ) => {
                self.ensure_type_matches(expected_element, actual_element, context)?;

                if let Some(expected_length) = expected_length {
                    if *expected_length != *actual_length {
                        return Err(self.type_mismatch_error(context, expected, actual, span));
                    }
                }

                Ok(())
            }
            (TypeName::Map(key_type, value_type), ExpressionType::EmptyMap) => {
                self.ensure_supported_map_key_type(key_type, "map key")?;
                self.ensure_declared_type_exists(value_type, "map value", false)?;
                Ok(())
            }
            (TypeName::Set(element_type), ExpressionType::EmptySet) => {
                self.ensure_supported_set_element_type(element_type, "set element")?;
                Ok(())
            }
            _ if matches!(actual, ExpressionType::Concrete(actual_type) if expected == actual_type) => {
                Ok(())
            }
            _ if expected.is_integer() && matches!(actual, ExpressionType::IntegerLiteral) => {
                Ok(())
            }
            _ if expected.is_float() && matches!(actual, ExpressionType::FloatLiteral) => Ok(()),
            _ => Err(self.type_mismatch_error(context, expected, actual, span)),
        }
    }

    fn type_mismatch_error(
        &self,
        context: &str,
        expected: &TypeName,
        actual: &ExpressionType,
        span: Option<Span>,
    ) -> SemanticError {
        let mut error = SemanticError::new(format!(
            "type mismatch in {}: expected {}, found {}",
            context,
            self.type_name_label(expected),
            self.expression_type_label(actual)
        ));
        if let Some(span) = span {
            error = error.with_span(span);
        }
        error
    }

    fn type_name_label(&self, type_name: &TypeName) -> String {
        match type_name {
            TypeName::I8 => "i8".to_string(),
            TypeName::I16 => "i16".to_string(),
            TypeName::I32 => "i32".to_string(),
            TypeName::I64 => "i64".to_string(),
            TypeName::U8 => "u8".to_string(),
            TypeName::U16 => "u16".to_string(),
            TypeName::U32 => "u32".to_string(),
            TypeName::U64 => "u64".to_string(),
            TypeName::F32 => "f32".to_string(),
            TypeName::F64 => "f64".to_string(),
            TypeName::Bool => "Bool".to_string(),
            TypeName::Text => "Text".to_string(),
            TypeName::Json => "Json".to_string(),
            TypeName::Char => "Char".to_string(),
            TypeName::Void => "Void".to_string(),
            TypeName::Reference { inner, is_mutable } => {
                let prefix = if *is_mutable { "mut ref " } else { "ref " };
                format!("{prefix}{}", self.type_name_label(inner))
            }
            TypeName::Array(inner, Some(length)) => {
                format!("Array<{}; {}>", self.type_name_label(inner), length)
            }
            TypeName::Array(inner, None) => format!("Array<{}>", self.type_name_label(inner)),
            TypeName::Map(key, value) => {
                format!(
                    "Map<{}, {}>",
                    self.type_name_label(key),
                    self.type_name_label(value)
                )
            }
            TypeName::Set(inner) => format!("Set<{}>", self.type_name_label(inner)),
            TypeName::Named(name) => name.clone(),
        }
    }

    fn expression_type_label(&self, expression_type: &ExpressionType) -> String {
        match expression_type {
            ExpressionType::Concrete(type_name) => self.type_name_label(type_name),
            ExpressionType::IntegerLiteral => "i64".to_string(),
            ExpressionType::FloatLiteral => "f64".to_string(),
            ExpressionType::Array(element, length) => {
                format!("Array<{}; {}>", self.expression_type_label(element), length)
            }
            ExpressionType::EmptyMap => "Map<?, ?>".to_string(),
            ExpressionType::EmptySet => "Set<?>".to_string(),
        }
    }

    fn ensure_expression_types_compatible(
        &self,
        left: &ExpressionType,
        right: &ExpressionType,
        context: &str,
    ) -> SemanticResult<()> {
        match (left, right) {
            (ExpressionType::Concrete(left), ExpressionType::Concrete(right)) if left == right => {
                Ok(())
            }
            (ExpressionType::IntegerLiteral, ExpressionType::IntegerLiteral)
            | (ExpressionType::FloatLiteral, ExpressionType::FloatLiteral) => Ok(()),
            (ExpressionType::Concrete(type_name), ExpressionType::IntegerLiteral)
            | (ExpressionType::IntegerLiteral, ExpressionType::Concrete(type_name))
                if type_name.is_integer() =>
            {
                Ok(())
            }
            (ExpressionType::Concrete(type_name), ExpressionType::FloatLiteral)
            | (ExpressionType::FloatLiteral, ExpressionType::Concrete(type_name))
                if type_name.is_float() =>
            {
                Ok(())
            }
            (
                ExpressionType::Array(left_element, left_length),
                ExpressionType::Array(right_element, right_length),
            ) if left_length == right_length => {
                self.ensure_expression_types_compatible(left_element, right_element, context)
            }
            _ => Err(SemanticError::new(format!(
                "type mismatch in {}: left is {:?}, right is {:?}",
                context, left, right
            ))),
        }
    }

    fn ensure_integer_type(&self, actual: &ExpressionType, context: &str) -> SemanticResult<()> {
        match actual {
            ExpressionType::IntegerLiteral => Ok(()),
            ExpressionType::Concrete(type_name) if type_name.is_integer() => Ok(()),
            _ => Err(SemanticError::new(format!(
                "{} must be an integer type, found {:?}",
                context, actual
            ))),
        }
    }

    fn resolve_numeric_binary_type(
        &self,
        left: &ExpressionType,
        right: &ExpressionType,
        context: &str,
    ) -> SemanticResult<TypeName> {
        match (left, right) {
            (ExpressionType::Concrete(left), ExpressionType::Concrete(right))
                if left == right && left.is_numeric() =>
            {
                Ok(left.clone())
            }
            (ExpressionType::Concrete(type_name), ExpressionType::IntegerLiteral)
            | (ExpressionType::IntegerLiteral, ExpressionType::Concrete(type_name))
                if type_name.is_integer() =>
            {
                Ok(type_name.clone())
            }
            (ExpressionType::Concrete(type_name), ExpressionType::FloatLiteral)
            | (ExpressionType::FloatLiteral, ExpressionType::Concrete(type_name))
                if type_name.is_float() =>
            {
                Ok(type_name.clone())
            }
            (ExpressionType::IntegerLiteral, ExpressionType::IntegerLiteral) => Ok(TypeName::I64),
            (ExpressionType::FloatLiteral, ExpressionType::FloatLiteral) => Ok(TypeName::F64),
            _ => Err(SemanticError::new(format!(
                "numeric operation type mismatch in {}: left is {:?}, right is {:?}",
                context, left, right
            ))),
        }
    }

    fn validate_comparison_operands(
        &self,
        left: &ExpressionType,
        right: &ExpressionType,
        binary: &loz_ast::BinaryExpression,
    ) -> SemanticResult<()> {
        match binary.operator {
            BinaryOperator::Equal | BinaryOperator::NotEqual => {
                if self
                    .ensure_expression_types_compatible(left, right, "comparison")
                    .is_ok()
                {
                    match (left, right) {
                        (ExpressionType::Concrete(left), ExpressionType::Concrete(right))
                            if left == right
                                && (left.is_numeric() || matches!(left, TypeName::Bool)) =>
                        {
                            Ok(())
                        }
                        (ExpressionType::IntegerLiteral, ExpressionType::IntegerLiteral)
                        | (ExpressionType::FloatLiteral, ExpressionType::FloatLiteral) => Ok(()),
                        (ExpressionType::Concrete(type_name), ExpressionType::IntegerLiteral)
                        | (ExpressionType::IntegerLiteral, ExpressionType::Concrete(type_name))
                            if type_name.is_integer() =>
                        {
                            Ok(())
                        }
                        (ExpressionType::Concrete(type_name), ExpressionType::FloatLiteral)
                        | (ExpressionType::FloatLiteral, ExpressionType::Concrete(type_name))
                            if type_name.is_float() =>
                        {
                            Ok(())
                        }
                        _ => Err(SemanticError::new(format!(
                            "comparison operator {:?} is not supported for {:?} and {:?}",
                            binary.operator, left, right
                        ))),
                    }
                } else {
                    Err(SemanticError::new(format!(
                        "comparison type mismatch: left is {:?}, right is {:?}",
                        left, right
                    )))
                }
            }
            _ => self
                .resolve_numeric_binary_type(left, right, "comparison")
                .map(|_| ()),
        }
    }

    fn ensure_declared_type_exists(
        &self,
        type_name: &TypeName,
        context: &str,
        allow_void: bool,
    ) -> SemanticResult<()> {
        match type_name {
            TypeName::I8
            | TypeName::I16
            | TypeName::I32
            | TypeName::I64
            | TypeName::U8
            | TypeName::U16
            | TypeName::U32
            | TypeName::U64
            | TypeName::F32
            | TypeName::F64
            | TypeName::Bool
            | TypeName::Text
            | TypeName::Json
            | TypeName::Char => Ok(()),
            TypeName::Void if allow_void => Ok(()),
            TypeName::Void => Err(SemanticError::new(format!(
                "{context} cannot use type Void"
            ))),
            TypeName::Reference { inner, .. } => {
                self.ensure_declared_type_exists(inner, context, false)
            }
            TypeName::Array(element_type, _) => {
                self.ensure_declared_type_exists(element_type, context, false)
            }
            TypeName::Map(key_type, value_type) => {
                self.ensure_supported_map_key_type(key_type, context)?;
                self.ensure_declared_type_exists(value_type, context, false)
            }
            TypeName::Set(element_type) => {
                self.ensure_supported_set_element_type(element_type, context)
            }
            TypeName::Named(name) if self.structs.contains_key(name) || name == "Any" => Ok(()),
            TypeName::Named(name) => Err(SemanticError::new(format!(
                "{context} references unknown type '{}'",
                name
            ))),
        }
    }

    fn ensure_supported_schema_field_type(
        &self,
        type_name: &TypeName,
        context: &str,
    ) -> SemanticResult<()> {
        match type_name {
            TypeName::Text
            | TypeName::Bool
            | TypeName::I32
            | TypeName::I64
            | TypeName::F64
            | TypeName::Json => Ok(()),
            other => Err(SemanticError::new(format!(
                "{context} uses unsupported schema field type {:?}",
                other
            ))),
        }
    }

    fn ensure_supported_map_key_type(
        &self,
        type_name: &TypeName,
        context: &str,
    ) -> SemanticResult<()> {
        self.ensure_supported_hash_collection_type(type_name, context)
    }

    fn ensure_supported_set_element_type(
        &self,
        type_name: &TypeName,
        context: &str,
    ) -> SemanticResult<()> {
        self.ensure_supported_hash_collection_type(type_name, context)
    }

    fn ensure_supported_hash_collection_type(
        &self,
        type_name: &TypeName,
        context: &str,
    ) -> SemanticResult<()> {
        match type_name {
            TypeName::Text | TypeName::Bool => Ok(()),
            type_name if type_name.is_integer() => Ok(()),
            TypeName::Named(name) if !self.structs.contains_key(name) && name != "Any" => Err(
                SemanticError::new(format!("{context} references unknown type '{}'", name)),
            ),
            _ => Err(SemanticError::new(format!(
                "{context} must be a supported hash collection type, found {:?}",
                type_name
            ))),
        }
    }

    fn ensure_legal_explicit_cast(
        &self,
        source: &ExpressionType,
        target: &TypeName,
        context: &str,
    ) -> SemanticResult<()> {
        if target == &TypeName::Bool {
            return match source {
                ExpressionType::Concrete(source_type) if source_type.is_integer() => Ok(()),
                ExpressionType::IntegerLiteral => Ok(()),
                _ => Err(SemanticError::new(format!(
                    "illegal cast in {}: cannot cast {:?} to {:?}",
                    context, source, target
                ))),
            };
        }

        if target.is_integer() {
            return match source {
                ExpressionType::Concrete(source_type)
                    if source_type.is_integer()
                        || source_type.is_float()
                        || source_type == &TypeName::Bool =>
                {
                    Ok(())
                }
                ExpressionType::IntegerLiteral | ExpressionType::FloatLiteral => Ok(()),
                _ => Err(SemanticError::new(format!(
                    "illegal cast in {}: cannot cast {:?} to {:?}",
                    context, source, target
                ))),
            };
        }

        if target.is_float() {
            return match source {
                ExpressionType::Concrete(source_type)
                    if source_type.is_integer() || source_type.is_float() =>
                {
                    Ok(())
                }
                ExpressionType::IntegerLiteral | ExpressionType::FloatLiteral => Ok(()),
                _ => Err(SemanticError::new(format!(
                    "illegal cast in {}: cannot cast {:?} to {:?}",
                    context, source, target
                ))),
            };
        }

        Err(SemanticError::new(format!(
            "illegal cast target in {}: {:?}",
            context, target
        )))
    }

    fn expression_type_to_type_name(
        &self,
        expression_type: &ExpressionType,
        context: &str,
    ) -> SemanticResult<TypeName> {
        match expression_type {
            ExpressionType::Concrete(type_name) => Ok(type_name.clone()),
            ExpressionType::IntegerLiteral => Ok(TypeName::I64),
            ExpressionType::FloatLiteral => Ok(TypeName::F64),
            ExpressionType::Array(element_type, length) => Ok(TypeName::Array(
                Box::new(self.expression_type_to_type_name(element_type, context)?),
                Some(*length),
            )),
            ExpressionType::EmptyMap => Err(SemanticError::new(format!(
                "cannot infer map type in {} from empty Map() constructor",
                context
            ))),
            ExpressionType::EmptySet => Err(SemanticError::new(format!(
                "cannot infer set type in {} from empty Set() constructor",
                context
            ))),
        }
        .and_then(|type_name| {
            self.ensure_declared_type_exists(&type_name, context, false)?;
            Ok(type_name)
        })
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn analyze(program: &Program) -> SemanticResult<()> {
    SemanticAnalyzer::new().analyze(program)
}

#[cfg(test)]
mod tests {
    use loz_lexer::tokenize;
    use loz_lexer::tokenize_with_file_path;
    use loz_parser::parse_program;

    use super::{SemanticError, analyze};

    fn analyze_source(source: &str) -> Result<(), String> {
        let program = parse_program(tokenize(source).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        analyze(&program).map_err(|error| error.to_string())
    }

    fn analyze_source_error(source: &str) -> SemanticError {
        let program =
            parse_program(tokenize_with_file_path(source, "src/main.loz").unwrap()).unwrap();
        analyze(&program).unwrap_err()
    }

    #[test]
    fn passes_valid_program() {
        let source = r#"const x: Int = 10;
func add(a: Int, b: Int) -> Int {
    return a + b;
}
func main() -> Void {
    print(add(x, 20));
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_multiple_numeric_types() {
        let source = r#"func main() -> i32 {
    const x: i32 = 10;
    const y: i32 = 20;
    const pi: f64 = 3.14;
    const ok: Bool = true;
    print(pi);
    print(ok);
    return x + y;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn reports_unknown_identifier_with_source_span() {
        let source = "func main() -> i32 {\n    print(username);\n    return 0;\n}\n";
        let error = analyze_source_error(source);
        let rendered = error.diagnostic.render_with_source(Some(source));

        assert!(rendered.contains("src/main.loz:2:11"));
        assert!(rendered.contains("unknown identifier 'username'"));
        assert!(rendered.contains("print(username);"));
        assert!(rendered.contains("^^^^^^^^"));
    }

    #[test]
    fn reports_type_mismatch_with_source_span() {
        let source = "const x: i32 = \"bad\";\nfunc main() -> i32 {\n    return 0;\n}\n";
        let error = analyze_source_error(source);
        let rendered = error.diagnostic.render_with_source(Some(source));

        assert!(rendered.contains("src/main.loz:1:16"));
        assert!(
            rendered.contains("type mismatch in variable declaration: expected i32, found Text")
        );
        assert!(rendered.contains("\"bad\""));
        assert!(rendered.contains("^^^^^"));
    }

    #[test]
    fn rejects_mixed_numeric_binary_types_without_casts() {
        let source = r#"func main() -> i32 {
    const x: i32 = 10;
    const y: i64 = 20;
    return x + y;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("numeric operation type mismatch"));
    }

    #[test]
    fn accepts_explicit_numeric_and_bool_casts() {
        let source = r#"func main() -> i32 {
    const x: i32 = 10;
    const y: f64 = as<f64>(x);
    const z: Bool = as<Bool>(1);
    const w: u8 = as<u8>(z);
    print(y);
    print(w);
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn rejects_illegal_struct_cast() {
        let source = r#"struct Point {
    x: i32,
    y: i32
}

func main() -> i32 {
    const p: Point = Point(10, 20);
    const x: i32 = as<i32>(p);
    return x;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("illegal cast"));
    }

    #[test]
    fn accepts_references_and_dereference_assignment() {
        let source = r#"func main() -> i32 {
    mut x: i32 = 10;
    const rx = ref x;
    const ry = mut ref x;
    print(*rx);
    *ry = 50;
    return x;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn rejects_mutable_reference_to_immutable_value() {
        let source = r#"func main() -> i32 {
    const x: i32 = 10;
    const rx = mut ref x;
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("mutable reference"));
    }

    #[test]
    fn rejects_assignment_through_immutable_reference() {
        let source = r#"func main() -> i32 {
    mut x: i32 = 10;
    const rx = ref x;
    *rx = 50;
    return x;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("immutable reference"));
    }

    #[test]
    fn rejects_variable_type_mismatch() {
        let source = r#"const x: Int = "hello";"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch"));
    }

    #[test]
    fn rejects_const_reassignment() {
        let source = r#"const x: Int = 10;
x = 20;
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("cannot reassign immutable value"));
    }

    #[test]
    fn accepts_module_and_import_declarations() {
        let source = r#"module math;

func add(a: Int, b: Int) -> Int {
    return a + b;
}

module main;
import math;

func main() -> Int {
    return add(10, 20);
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn rejects_duplicate_module_names() {
        let source = r#"module math;
module math;
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate module name"));
    }

    #[test]
    fn rejects_duplicate_imports() {
        let source = r#"module math;

func add(a: Int, b: Int) -> Int {
    return a + b;
}

module main;
import math;
import math;
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate import"));
    }

    #[test]
    fn rejects_missing_imported_module() {
        let source = r#"module main;
import math;
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("does not exist"));
    }

    #[test]
    fn accepts_mutable_struct_field_assignment() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    mut p: Point = Point(10, 20);
    p.x = 50;
    return p.x;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn rejects_undeclared_identifier() {
        let source = r#"func add() -> Int {
    return unknownVar;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("unknown identifier"));
    }

    #[test]
    fn rejects_missing_return() {
        let source = r#"func add() -> Int {
}"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("missing a return statement"));
    }

    #[test]
    fn rejects_duplicate_variable_names_in_same_scope() {
        let source = r#"const x: Int = 10;
const x: Int = 20;
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate variable name"));
    }

    #[test]
    fn rejects_unknown_function_calls() {
        let source = r#"func main() -> Void {
    missing(1);
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("undeclared function"));
    }

    #[test]
    fn accepts_if_condition_with_int_comparison() {
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

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_while_condition_with_int_comparison() {
        let source = r#"func main() -> Int {
    mut x: Int = 0;
    while x < 10 {
        x = x + 1;
    }
    return x;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_for_loop_over_array_with_inferred_loop_variable() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20, 30];
    for x in nums {
        print(x);
    }
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_break_and_continue_inside_loops() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20, 30];
    for x in nums {
        if x == 20 {
            continue;
        }

        if x == 30 {
            break;
        }
    }

    mut y: Int = 0;
    while y < 10 {
        y = y + 1;
        break;
    }

    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_struct_declarations_with_known_field_types() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

struct Line {
    start: Point,
    end: Point,
}

func main() -> Int {
    print("Struct phase started");
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_struct_constructor_with_matching_arguments() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    const p: Point = Point(10, 20);
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_struct_field_access_with_known_field() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    const p: Point = Point(10, 20);
    return p.x;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_struct_parameters_and_returns() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func make_point() -> Point {
    return Point(10, 20);
}

func sum(p: Point) -> Int {
    return p.x + p.y;
}

func main() -> Int {
    const p: Point = make_point();
    return sum(p);
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_impl_method_and_struct_method_call() {
        let source = r#"struct Point {
    x: i32,
    y: i32
}

impl Point {
    func sum(self) -> i32 {
        return self.x + self.y;
    }
}

func main() -> i32 {
    const p: Point = Point(10, 20);
    return p.sum();
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_array_literal_and_index_access() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20, 30];
    return nums[1];
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_array_of_struct_values() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    const points: Array<Point> = [Point(10, 20), Point(30, 40)];
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_mutable_array_element_assignment() {
        let source = r#"func main() -> Int {
    mut nums: Array<Int> = [10, 20, 30];
    nums[1] = 99;
    return nums[1];
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_array_len_push_and_pop() {
        let source = r#"func main() -> Int {
    mut nums: Array<Int> = [10, 20];
    nums.push(30);
    const size: u64 = nums.len();
    const last: Int = nums.pop();
    print(size);
    return last;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_map_construction_and_methods() {
        let source = r#"func main() -> i32 {
    mut scores: Map<Text, i32> = Map();
    scores.insert("ahmed", 100);
    print(scores.contains("ahmed"));
    return scores.get("ahmed");
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_set_construction_and_methods() {
        let source = r#"func main() -> i32 {
    mut ids: Set<i32> = Set();
    ids.add(10);
    print(ids.contains(10));
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_all_io_read_methods() {
        let source = r#"func main() -> i32 {
    const name: Text = io.read_line();
    const tiny_signed: i8 = io.read_i8();
    const small_signed: i16 = io.read_i16();
    const age: i32 = io.read_i32();
    const big_signed: i64 = io.read_i64();
    const tiny_unsigned: u8 = io.read_u8();
    const small_unsigned: u16 = io.read_u16();
    const medium_unsigned: u32 = io.read_u32();
    const big_unsigned: u64 = io.read_u64();
    const ratio32: f32 = io.read_f32();
    const score: f64 = io.read_f64();
    const active: Bool = io.read_bool();
    print(name);
    print(tiny_signed);
    print(small_signed);
    print(age);
    print(big_signed);
    print(tiny_unsigned);
    print(small_unsigned);
    print(medium_unsigned);
    print(big_unsigned);
    print(ratio32);
    print(score);
    print(active);
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_json_parse_and_getters() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\",\"active\":true,\"score\":98.5}");
    print(json.get_text(user, "name"));
    print(json.get_i32(user, "id"));
    print(json.get_i64(user, "id"));
    print(json.get_f64(user, "score"));
    print(json.get_bool(user, "active"));
    print(json.has(user, "email"));
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_schema_validate_and_require() {
        let source = r#"schema User {
    id: i32,
    name: Text,
    active: Bool
}

func main() -> i32 {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\",\"active\":true}");
    print(schema.validate("User", user));
    const checked: Json = schema.require("User", user);
    print(json.get_text(checked, "name"));
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_python_call_with_text_and_json() {
        let source = r#"func main() -> i32 {
    const payload: Json = json.parse("{\"text\":\"hello\"}");
    const result: Json = python.call("tools.analyze_text", payload);
    print(result);
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_llm_ask_with_text() {
        let source = r#"func main() -> i32 {
    const answer: Text = llm.ask("hello");
    print(answer);
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_agent_task_with_llm_ask() {
        let source = r#"agent SupportAgent {
    model: "mock";
    tools: [];

    task answer(question: Text) -> Text {
        return llm.ask(question);
    }
}

func main() -> i32 {
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_valid_workflow_with_function_tool_and_agent_steps() {
        let source = r#"func prepare() -> Text {
    return "prepared";
}

tool get_data() -> Json {
    return json.parse("{\"name\":\"Ahmed\"}");
}

agent SupportAgent {
    model: "mock";
    tools: [];

    task answer() -> Text {
        return "ok";
    }
}

workflow Onboarding {
    step prepare;
    step get_data;
    step SupportAgent.answer;
}

func main() -> i32 {
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_valid_tool_declaration() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

func main() -> i32 {
    const user: Json = get_user(1);
    print(json.get_text(user, "name"));
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn accepts_tool_using_json_and_schema() {
        let source = r#"schema User {
    id: i32,
    name: Text
}

tool get_user(id: i32) -> Json {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
    return schema.require("User", user);
}

func main() -> i32 {
    const user: Json = get_user(1);
    print(schema.validate("User", user));
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn rejects_duplicate_tool_names() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1}");
}

tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":2}");
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate tool name"));
    }

    #[test]
    fn rejects_tool_function_name_conflict() {
        let source = r#"func get_user(id: i32) -> Json {
    return json.parse("{\"id\":1}");
}

tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":2}");
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate tool name") || error.contains("duplicate function name"));
    }

    #[test]
    fn rejects_unknown_tool_call() {
        let source = r#"func main() -> i32 {
    const user: Json = get_user(1);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("undeclared function or tool"));
    }

    #[test]
    fn rejects_tool_call_with_wrong_argument_count() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1}");
}

func main() -> i32 {
    const user: Json = get_user();
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("call 'get_user' expected 1 arguments, found 0"));
    }

    #[test]
    fn rejects_tool_call_with_wrong_argument_type() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1}");
}

func main() -> i32 {
    const user: Json = get_user("bad");
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in call"));
    }

    #[test]
    fn rejects_tool_body_return_type_mismatch() {
        let source = r#"tool get_user(id: i32) -> Json {
    return 1;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in return statement"));
    }

    #[test]
    fn rejects_nested_tool_declaration() {
        let source = r#"func main() -> i32 {
    tool get_user(id: i32) -> Json {
        return json.parse("{\"id\":1}");
    }
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("tool declaration 'get_user' is only allowed at the top level"));
    }

    #[test]
    fn rejects_duplicate_task_names_in_agent() {
        let source = r#"agent SupportAgent {
    model: "mock";
    tools: [];

    task answer(question: Text) -> Text {
        return question;
    }

    task answer(question: Text) -> Text {
        return llm.ask(question);
    }
}

func main() -> i32 {
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate task name 'answer' in agent 'SupportAgent'"));
    }

    #[test]
    fn rejects_duplicate_workflow_names() {
        let source = r#"workflow Onboarding {
    step prepare;
}

workflow Onboarding {
    step prepare;
}

func prepare() -> Text {
    return "prepared";
}

func main() -> i32 {
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate workflow name 'Onboarding'"));
    }

    #[test]
    fn rejects_duplicate_workflow_step_names() {
        let source = r#"func get_data() -> Json {
    return json.parse("{\"name\":\"Ahmed\"}");
}

workflow Onboarding {
    step get_data;
    step get_data;
}

func main() -> i32 {
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate step 'get_data' in workflow 'Onboarding'"));
    }

    #[test]
    fn rejects_unknown_workflow_step_target() {
        let source = r#"workflow Onboarding {
    step validate_data;
}

func main() -> i32 {
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains(
            "workflow 'Onboarding' step 'validate_data' refers to unknown function/tool 'validate_data'"
        ));
    }

    #[test]
    fn rejects_workflow_step_target_with_parameters() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1}");
}

workflow Onboarding {
    step get_user;
}

func main() -> i32 {
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains(
            "workflow step 'get_user' must refer to a zero-argument function/tool in this phase"
        ));
    }

    #[test]
    fn rejects_duplicate_struct_names() {
        let source = r#"struct Point {
    x: Int,
}

struct Point {
    y: Int,
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate struct name"));
    }

    #[test]
    fn rejects_duplicate_field_names_in_struct() {
        let source = r#"struct Point {
    x: Int,
    x: Int,
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate field name"));
    }

    #[test]
    fn rejects_duplicate_schema_names() {
        let source = r#"schema User {
    id: i32
}

schema User {
    name: Text
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate schema name"));
    }

    #[test]
    fn rejects_duplicate_field_names_in_schema() {
        let source = r#"schema User {
    id: i32,
    id: i32
}"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate field name"));
    }

    #[test]
    fn rejects_unknown_schema_field_type() {
        let source = r#"schema User {
    id: i32,
    profile: Profile
}"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("unsupported schema field type"));
    }

    #[test]
    fn rejects_unknown_schema_name_in_schema_validate() {
        let source = r#"schema User {
    id: i32
}

func main() -> i32 {
    const user: Json = json.parse("{\"id\":1}");
    print(schema.validate("Missing", user));
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("unknown schema name"));
    }

    #[test]
    fn rejects_python_call_with_wrong_argument_count() {
        let source = r#"func main() -> i32 {
    const payload: Json = json.parse("{\"text\":\"hello\"}");
    return python.call("tools.analyze_text");
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("python.call() expects 2 arguments"));
    }

    #[test]
    fn rejects_python_call_with_wrong_first_argument_type() {
        let source = r#"func main() -> i32 {
    const payload: Json = json.parse("{\"text\":\"hello\"}");
    const result: Json = python.call(10, payload);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in python method call"));
    }

    #[test]
    fn rejects_python_call_with_wrong_second_argument_type() {
        let source = r#"func main() -> i32 {
    const result: Json = python.call("tools.analyze_text", "bad");
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in python method call"));
    }

    #[test]
    fn rejects_unknown_python_method() {
        let source = r#"func main() -> i32 {
    const payload: Json = json.parse("{\"text\":\"hello\"}");
    const result: Json = python.invoke("tools.analyze_text", payload);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("unknown python method"));
    }

    #[test]
    fn rejects_llm_ask_with_wrong_argument_count() {
        let source = r#"func main() -> i32 {
    const answer: Text = llm.ask();
    print(answer);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("llm.ask() expects 1 argument"));
    }

    #[test]
    fn rejects_llm_ask_with_wrong_argument_type() {
        let source = r#"func main() -> i32 {
    const answer: Text = llm.ask(10);
    print(answer);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in llm method call"));
    }

    #[test]
    fn rejects_unknown_llm_method() {
        let source = r#"func main() -> i32 {
    const answer: Text = llm.reply("hello");
    print(answer);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("unknown llm method"));
    }

    #[test]
    fn rejects_schema_method_with_wrong_argument_type() {
        let source = r#"schema User {
    id: i32
}

func main() -> i32 {
    print(schema.validate("User", "bad"));
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("schema method call"));
    }

    #[test]
    fn rejects_schema_method_with_wrong_argument_count() {
        let source = r#"schema User {
    id: i32
}

func main() -> i32 {
    print(schema.validate("User"));
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("expects 2 arguments"));
    }

    #[test]
    fn rejects_struct_constructor_with_wrong_argument_count() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    const p: Point = Point(10);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("expected 2 arguments, found 1"));
    }

    #[test]
    fn rejects_struct_constructor_with_wrong_argument_type() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    const p: Point = Point(10, "bad");
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in struct construction"));
    }

    #[test]
    fn rejects_unknown_struct_field_access() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    const p: Point = Point(10, 20);
    return p.z;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("has no field"));
    }

    #[test]
    fn rejects_struct_field_assignment_on_immutable_base() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    const p: Point = Point(10, 20);
    p.x = 50;
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("is immutable"));
    }

    #[test]
    fn rejects_struct_field_assignment_with_wrong_type() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    mut p: Point = Point(10, 20);
    p.x = "bad";
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in field assignment"));
    }

    #[test]
    fn rejects_unknown_struct_field_assignment() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    mut p: Point = Point(10, 20);
    p.z = 50;
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("has no field"));
    }

    #[test]
    fn rejects_field_access_on_non_struct_value() {
        let source = r#"func main() -> Int {
    const x: Int = 10;
    return x.value;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("cannot access field"));
    }

    #[test]
    fn rejects_unknown_struct_field_type() {
        let source = r#"struct Point {
    x: MissingType,
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("references unknown type"));
    }

    #[test]
    fn rejects_unknown_struct_parameter_type() {
        let source = r#"func sum(p: MissingPoint) -> Int {
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("parameter 'p' in function 'sum' references unknown type"));
    }

    #[test]
    fn rejects_unknown_struct_return_type() {
        let source = r#"func make_point() -> MissingPoint {
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("return type of function 'make_point' references unknown type"));
    }

    #[test]
    fn rejects_impl_target_for_unknown_struct() {
        let source = r#"impl MissingPoint {
    func sum(self) -> Int {
        return 0;
    }
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("impl target"));
    }

    #[test]
    fn rejects_duplicate_method_names_in_same_impl() {
        let source = r#"struct Point {
    x: Int
}

impl Point {
    func sum(self) -> Int {
        return self.x;
    }

    func sum(self) -> Int {
        return self.x;
    }
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate method name"));
    }

    #[test]
    fn rejects_unknown_struct_method_call() {
        let source = r#"struct Point {
    x: Int
}

impl Point {
    func sum(self) -> Int {
        return self.x;
    }
}

func main() -> Int {
    const p: Point = Point(10);
    return p.missing();
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("has no method"));
    }

    #[test]
    fn rejects_struct_method_call_with_wrong_argument_type() {
        let source = r#"struct Point {
    x: Int
}

impl Point {
    func add(self, delta: Int) -> Int {
        return self.x + delta;
    }
}

func main() -> Int {
    const p: Point = Point(10);
    return p.add("bad");
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in method call"));
    }

    #[test]
    fn rejects_array_literal_with_mismatched_element_types() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, "bad"];
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in array literal"));
    }

    #[test]
    fn rejects_array_index_with_non_int_expression() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20, 30];
    return nums["bad"];
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("array index must be an integer type"));
    }

    #[test]
    fn rejects_index_access_on_non_array_value() {
        let source = r#"func main() -> Int {
    const x: Int = 10;
    return x[0];
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("cannot index non-array value"));
    }

    #[test]
    fn rejects_array_element_assignment_on_immutable_base() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20, 30];
    nums[1] = 99;
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("is immutable"));
    }

    #[test]
    fn rejects_array_element_assignment_with_wrong_type() {
        let source = r#"func main() -> Int {
    mut nums: Array<Int> = [10, 20, 30];
    nums[1] = "bad";
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in array element assignment"));
    }

    #[test]
    fn rejects_array_element_assignment_on_non_array_value() {
        let source = r#"func main() -> Int {
    mut x: Int = 10;
    x[0] = 99;
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("non-array value"));
    }

    #[test]
    fn rejects_array_push_on_immutable_array() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20];
    nums.push(30);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("immutable array"));
    }

    #[test]
    fn rejects_array_pop_on_immutable_array() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20];
    return nums.pop();
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("immutable array"));
    }

    #[test]
    fn rejects_array_push_with_wrong_element_type() {
        let source = r#"func main() -> Int {
    mut nums: Array<Int> = [10, 20];
    nums.push("bad");
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in array push"));
    }

    #[test]
    fn rejects_array_methods_on_non_array_value() {
        let source = r#"func main() -> u64 {
    const x: Int = 10;
    return x.len();
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("non-array value"));
    }

    #[test]
    fn rejects_map_insert_with_wrong_key_type() {
        let source = r#"func main() -> i32 {
    mut scores: Map<Text, i32> = Map();
    scores.insert(10, 100);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("map insert key"));
    }

    #[test]
    fn rejects_map_insert_with_wrong_value_type() {
        let source = r#"func main() -> i32 {
    mut scores: Map<Text, i32> = Map();
    scores.insert("ahmed", false);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("map insert value"));
    }

    #[test]
    fn rejects_map_with_unsupported_key_type() {
        let source = r#"func main() -> i32 {
    mut lookup: Map<f64, i32> = Map();
    lookup.insert(1.5, 10);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("supported hash collection type"));
    }

    #[test]
    fn rejects_untyped_empty_map_construction() {
        let source = r#"func main() -> i32 {
    const scores = Map();
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("cannot infer map type"));
    }

    #[test]
    fn rejects_set_add_on_immutable_set() {
        let source = r#"func main() -> i32 {
    const ids: Set<i32> = Set();
    ids.add(10);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("immutable set"));
    }

    #[test]
    fn rejects_set_add_with_wrong_element_type() {
        let source = r#"func main() -> i32 {
    mut ids: Set<i32> = Set();
    ids.add("bad");
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("set add element"));
    }

    #[test]
    fn rejects_set_with_unsupported_element_type() {
        let source = r#"func main() -> i32 {
    mut ids: Set<f64> = Set();
    ids.add(1.5);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("supported hash collection type"));
    }

    #[test]
    fn rejects_untyped_empty_set_construction() {
        let source = r#"func main() -> i32 {
    const ids = Set();
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("cannot infer set type"));
    }

    #[test]
    fn rejects_io_method_with_wrong_argument_count() {
        let source = r#"func main() -> i32 {
    return io.read_i32(10);
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("expects 0 arguments"));
    }

    #[test]
    fn rejects_unknown_io_method() {
        let source = r#"func main() -> i32 {
    return io.read_number();
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("unknown io method"));
    }

    #[test]
    fn rejects_json_method_with_wrong_argument_count() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{\"id\":1}");
    return json.get_i32(user);
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("expects 2 arguments"));
    }

    #[test]
    fn rejects_json_method_with_wrong_argument_type() {
        let source = r#"func main() -> i32 {
    return json.parse(10);
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in json method call"));
    }

    #[test]
    fn rejects_unknown_json_method() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{\"id\":1}");
    return json.get_number(user, "id");
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("unknown json method"));
    }

    #[test]
    fn rejects_json_arithmetic() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{\"id\":1}");
    return user + 1;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("numeric operation type mismatch"));
    }

    #[test]
    fn rejects_for_loop_over_non_array_value() {
        let source = r#"func main() -> Int {
    const x: Int = 10;
    for item in x {
        print(item);
    }
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("for loop source must be an Array value"));
    }

    #[test]
    fn rejects_break_outside_loop() {
        let source = r#"func main() -> Int {
    break;
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("break statement is only allowed inside a loop"));
    }

    #[test]
    fn rejects_continue_outside_loop() {
        let source = r#"func main() -> Int {
    continue;
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("continue statement is only allowed inside a loop"));
    }

    #[test]
    fn accepts_valid_async_task_and_await() {
        let source = r#"async task get_name() -> Text {
    return "Ahmed";
}

func main() -> i32 {
    const name: Text = await get_name();
    return 0;
}
"#;

        assert!(analyze_source(source).is_ok());
    }

    #[test]
    fn rejects_duplicate_async_task_names() {
        let source = r#"async task fetch_user() -> Text {
    return "Ahmed";
}

async task fetch_user() -> Text {
    return "Ali";
}

func main() -> i32 {
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("duplicate async task name 'fetch_user'"));
    }

    #[test]
    fn rejects_async_task_return_type_mismatch() {
        let source = r#"async task fetch_user() -> Json {
    return "bad";
}

func main() -> i32 {
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in return statement"));
    }

    #[test]
    fn rejects_async_task_call_without_await() {
        let source = r#"async task fetch_user(id: i32) -> Json {
    return json.parse("{\"id\":1}");
}

func main() -> i32 {
    const user: Json = fetch_user(1);
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("async task 'fetch_user' must be awaited"));
    }

    #[test]
    fn rejects_await_on_normal_function() {
        let source = r#"func get_value() -> i32 {
    return 10;
}

func main() -> i32 {
    const x: i32 = await get_value();
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("cannot await non-async call 'get_value'"));
    }

    #[test]
    fn rejects_async_task_with_wrong_argument_count() {
        let source = r#"async task fetch_user(id: i32) -> Json {
    return json.parse("{\"id\":1}");
}

func main() -> i32 {
    const user: Json = await fetch_user();
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("call 'fetch_user' expected 1 arguments, found 0"));
    }

    #[test]
    fn rejects_async_task_with_wrong_argument_type() {
        let source = r#"async task fetch_user(id: i32) -> Json {
    return json.parse("{\"id\":1}");
}

func main() -> i32 {
    const user: Json = await fetch_user("bad");
    return 0;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("type mismatch in await call"));
    }
}
