use std::collections::HashMap;

use inkwell::AddressSpace;
use inkwell::FloatPredicate;
use inkwell::IntPredicate;
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::{ArrayType, BasicType, BasicTypeEnum, StructType};
use inkwell::values::{
    ArrayValue, BasicValueEnum, FunctionValue, GlobalValue, IntValue, PointerValue, StructValue,
};
use loz_ast::{
    AgentDeclaration, AgentTaskDeclaration, AssignmentTarget, AsyncTaskDeclaration,
    AwaitExpression, DereferenceExpression, Expression, ExpressionKind, ForStatement,
    FunctionDeclaration, IfStatement, ImplBlock, MethodCallExpression, Program, SchemaDeclaration,
    Span, Statement, StructDeclaration, ToolDeclaration, TypeName, VariableDeclaration,
    WhileStatement, WorkflowDeclaration, WorkflowTarget,
};

use super::{CodegenError, CodegenResult};

pub struct LlvmIrGenerator<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    globals: HashMap<String, GlobalValue<'ctx>>,
    global_types: HashMap<String, TypeName>,
    functions: HashMap<String, FunctionDeclaration>,
    module_functions: HashMap<(String, String), FunctionDeclaration>,
    async_tasks: HashMap<String, FunctionDeclaration>,
    tools: HashMap<String, ToolDeclaration>,
    methods: HashMap<(String, String), FunctionDeclaration>,
    structs: HashMap<String, StructDeclaration>,
    schemas: HashMap<String, SchemaDeclaration>,
}

#[derive(Clone)]
enum FunctionBinding<'ctx> {
    Parameter {
        value: BasicValueEnum<'ctx>,
        type_name: TypeName,
    },
    Local {
        pointer: PointerValue<'ctx>,
        is_mutable: bool,
        type_name: TypeName,
    },
}

#[derive(Clone, Copy)]
struct LoopTargets<'ctx> {
    break_block: BasicBlock<'ctx>,
    continue_block: BasicBlock<'ctx>,
}

fn synthetic_identifier(name: String) -> Expression {
    Expression::new(ExpressionKind::Identifier(name), Span::default())
}

impl<'ctx> LlvmIrGenerator<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self {
            context,
            module: context.create_module(module_name),
            builder: context.create_builder(),
            globals: HashMap::new(),
            global_types: HashMap::new(),
            functions: HashMap::new(),
            module_functions: HashMap::new(),
            async_tasks: HashMap::new(),
            tools: HashMap::new(),
            methods: HashMap::new(),
            structs: HashMap::new(),
            schemas: HashMap::new(),
        }
    }

    pub fn generate_program(&mut self, program: &Program) -> CodegenResult<()> {
        self.declare_printf();
        self.declare_puts();
        self.declare_scanf();
        self.declare_strcmp();
        self.declare_fgets();
        self.declare_strcspn();
        self.declare_stdin();
        self.declare_text_runtime_globals();
        self.declare_json_runtime();
        self.declare_schema_runtime();
        self.declare_python_runtime();
        self.declare_llm_runtime();

        for statement in &program.statements {
            if let Statement::StructDeclaration(struct_declaration) = statement {
                self.register_struct(struct_declaration)?;
            }
            if let Statement::SchemaDeclaration(schema_declaration) = statement {
                self.register_schema(schema_declaration)?;
            }
        }

        let mut current_module = None;
        for statement in &program.statements {
            match statement {
                Statement::ModuleDeclaration(module_declaration) => {
                    current_module = Some(module_declaration.name.as_str());
                }
                Statement::ImportDeclaration(_) => {
                    current_module = None;
                }
                Statement::VariableDeclaration(declaration) => self.declare_global(declaration)?,
                Statement::FunctionDeclaration(function) => {
                    self.declare_function(function, current_module)?
                }
                Statement::AsyncTaskDeclaration(task) => self.declare_async_task(task)?,
                Statement::ToolDeclaration(tool) => self.declare_tool(tool)?,
                Statement::AgentDeclaration(agent) => self.declare_agent(agent)?,
                Statement::WorkflowDeclaration(workflow) => self.declare_workflow(workflow)?,
                Statement::StructDeclaration(_) => {}
                Statement::SchemaDeclaration(_) => {}
                Statement::ImplBlock(impl_block) => self.declare_impl_block(impl_block)?,
                _ => {
                    return Err(CodegenError::new(
                        "LLVM backend only supports top-level const primitive globals, struct declarations, schema declarations, function declarations, tool declarations, agent declarations, workflow declarations, and impl blocks",
                    ));
                }
            }
        }

        for statement in &program.statements {
            match statement {
                Statement::FunctionDeclaration(function) => self.define_function_body(function)?,
                Statement::AsyncTaskDeclaration(task) => self.define_async_task_body(task)?,
                Statement::ToolDeclaration(tool) => self.define_tool_body(tool)?,
                Statement::AgentDeclaration(agent) => self.define_agent_task_bodies(agent)?,
                Statement::WorkflowDeclaration(workflow) => self.define_workflow_body(workflow)?,
                Statement::ImplBlock(impl_block) => self.define_impl_block_bodies(impl_block)?,
                _ => {}
            }
        }

        if self.module.get_function("main").is_none() {
            return Err(CodegenError::new("LLVM backend requires a 'main' function"));
        }

        Ok(())
    }

    fn declare_impl_block(&mut self, impl_block: &ImplBlock) -> CodegenResult<()> {
        for method in &impl_block.methods {
            let lowered_method = self.lower_method_function(&impl_block.target_name, method);
            self.declare_function(&lowered_method, None)?;
            self.methods.insert(
                (impl_block.target_name.clone(), method.name.clone()),
                lowered_method,
            );
        }

        Ok(())
    }

    fn define_impl_block_bodies(&self, impl_block: &ImplBlock) -> CodegenResult<()> {
        for method in &impl_block.methods {
            let lowered_method = self.lower_method_function(&impl_block.target_name, method);
            self.define_function_body(&lowered_method)?;
        }

        Ok(())
    }

    fn lower_method_function(
        &self,
        target_name: &str,
        method: &FunctionDeclaration,
    ) -> FunctionDeclaration {
        let mut lowered = method.clone();
        lowered.name = self.method_symbol_name(target_name, &method.name);
        lowered
    }

    fn method_symbol_name(&self, target_name: &str, method_name: &str) -> String {
        format!("__loz_method__{target_name}__{method_name}")
    }

    fn tool_symbol_name(&self, tool_name: &str) -> String {
        format!("__loz_tool__{tool_name}")
    }

    fn async_task_symbol_name(&self, task_name: &str) -> String {
        format!("__loz_async__{task_name}")
    }

    fn agent_task_symbol_name(&self, agent_name: &str, task_name: &str) -> String {
        format!("__loz_agent__{agent_name}__{task_name}")
    }

    fn workflow_symbol_name(&self, workflow_name: &str) -> String {
        format!("__loz_workflow__{workflow_name}")
    }

    fn register_struct(&mut self, struct_declaration: &StructDeclaration) -> CodegenResult<()> {
        if self
            .structs
            .insert(struct_declaration.name.clone(), struct_declaration.clone())
            .is_some()
        {
            return Err(CodegenError::new(format!(
                "duplicate struct '{}' during LLVM registration",
                struct_declaration.name
            )));
        }

        Ok(())
    }

    fn register_schema(&mut self, schema_declaration: &SchemaDeclaration) -> CodegenResult<()> {
        if self
            .schemas
            .insert(schema_declaration.name.clone(), schema_declaration.clone())
            .is_some()
        {
            return Err(CodegenError::new(format!(
                "duplicate schema '{}' during LLVM registration",
                schema_declaration.name
            )));
        }

        Ok(())
    }

    fn declare_printf(&self) {
        if self.module.get_function("printf").is_some() {
            return;
        }

        let pointer_type = self.context.ptr_type(AddressSpace::default());
        let printf_type = self
            .context
            .i32_type()
            .fn_type(&[pointer_type.into()], true);
        self.module.add_function("printf", printf_type, None);
    }

    fn declare_puts(&self) {
        if self.module.get_function("puts").is_some() {
            return;
        }

        let pointer_type = self.context.ptr_type(AddressSpace::default());
        let puts_type = self
            .context
            .i32_type()
            .fn_type(&[pointer_type.into()], false);
        self.module.add_function("puts", puts_type, None);
    }

    fn declare_scanf(&self) {
        if self.module.get_function("scanf").is_some() {
            return;
        }

        let pointer_type = self.context.ptr_type(AddressSpace::default());
        let scanf_type = self
            .context
            .i32_type()
            .fn_type(&[pointer_type.into()], true);
        self.module.add_function("scanf", scanf_type, None);
    }

    fn declare_strcmp(&self) {
        if self.module.get_function("strcmp").is_some() {
            return;
        }

        let pointer_type = self.context.ptr_type(AddressSpace::default());
        let strcmp_type = self
            .context
            .i32_type()
            .fn_type(&[pointer_type.into(), pointer_type.into()], false);
        self.module.add_function("strcmp", strcmp_type, None);
    }

    fn declare_fgets(&self) {
        if self.module.get_function("fgets").is_some() {
            return;
        }

        let pointer_type = self.context.ptr_type(AddressSpace::default());
        let fgets_type = pointer_type.fn_type(
            &[
                pointer_type.into(),
                self.context.i32_type().into(),
                pointer_type.into(),
            ],
            false,
        );
        self.module.add_function("fgets", fgets_type, None);
    }

    fn declare_strcspn(&self) {
        if self.module.get_function("strcspn").is_some() {
            return;
        }

        let pointer_type = self.context.ptr_type(AddressSpace::default());
        let strcspn_type = self
            .context
            .i64_type()
            .fn_type(&[pointer_type.into(), pointer_type.into()], false);
        self.module.add_function("strcspn", strcspn_type, None);
    }

    fn declare_stdin(&self) {
        if self.module.get_global("stdin").is_some() {
            return;
        }

        let stdin = self.module.add_global(
            self.context.ptr_type(AddressSpace::default()),
            None,
            "stdin",
        );
        stdin.set_linkage(inkwell::module::Linkage::External);
    }

    fn declare_text_runtime_globals(&self) {
        if self.module.get_global("loz_stdin_buffer").is_none() {
            let buffer_type = self.context.i8_type().array_type(1024);
            let buffer = self
                .module
                .add_global(buffer_type, None, "loz_stdin_buffer");
            buffer.set_linkage(inkwell::module::Linkage::Internal);
            buffer.set_initializer(&buffer_type.const_zero());
        }
    }

    fn declare_json_runtime(&self) {
        let pointer_type = self.context.ptr_type(AddressSpace::default());

        if self.module.get_function("loz_json_parse").is_none() {
            let function_type = pointer_type.fn_type(&[pointer_type.into()], false);
            self.module
                .add_function("loz_json_parse", function_type, None);
        }

        if self.module.get_function("loz_json_stringify").is_none() {
            let function_type = pointer_type.fn_type(&[pointer_type.into()], false);
            self.module
                .add_function("loz_json_stringify", function_type, None);
        }

        if self.module.get_function("loz_json_has").is_none() {
            let function_type = self
                .context
                .bool_type()
                .fn_type(&[pointer_type.into(), pointer_type.into()], false);
            self.module
                .add_function("loz_json_has", function_type, None);
        }

        if self.module.get_function("loz_json_get_text").is_none() {
            let function_type =
                pointer_type.fn_type(&[pointer_type.into(), pointer_type.into()], false);
            self.module
                .add_function("loz_json_get_text", function_type, None);
        }

        if self.module.get_function("loz_json_get_i32").is_none() {
            let function_type = self
                .context
                .i32_type()
                .fn_type(&[pointer_type.into(), pointer_type.into()], false);
            self.module
                .add_function("loz_json_get_i32", function_type, None);
        }

        if self.module.get_function("loz_json_get_i64").is_none() {
            let function_type = self
                .context
                .i64_type()
                .fn_type(&[pointer_type.into(), pointer_type.into()], false);
            self.module
                .add_function("loz_json_get_i64", function_type, None);
        }

        if self.module.get_function("loz_json_get_f64").is_none() {
            let function_type = self
                .context
                .f64_type()
                .fn_type(&[pointer_type.into(), pointer_type.into()], false);
            self.module
                .add_function("loz_json_get_f64", function_type, None);
        }

        if self.module.get_function("loz_json_get_bool").is_none() {
            let function_type = self
                .context
                .bool_type()
                .fn_type(&[pointer_type.into(), pointer_type.into()], false);
            self.module
                .add_function("loz_json_get_bool", function_type, None);
        }
    }

    fn declare_schema_runtime(&self) {
        let pointer_type = self.context.ptr_type(AddressSpace::default());

        if self.module.get_function("loz_schema_validate").is_none() {
            let function_type = self
                .context
                .bool_type()
                .fn_type(&[pointer_type.into(), pointer_type.into()], false);
            self.module
                .add_function("loz_schema_validate", function_type, None);
        }

        if self.module.get_function("loz_schema_require").is_none() {
            let function_type =
                pointer_type.fn_type(&[pointer_type.into(), pointer_type.into()], false);
            self.module
                .add_function("loz_schema_require", function_type, None);
        }
    }

    fn declare_python_runtime(&self) {
        let pointer_type = self.context.ptr_type(AddressSpace::default());

        if self.module.get_function("loz_python_call").is_none() {
            let function_type =
                pointer_type.fn_type(&[pointer_type.into(), pointer_type.into()], false);
            self.module
                .add_function("loz_python_call", function_type, None);
        }
    }

    fn declare_llm_runtime(&self) {
        let pointer_type = self.context.ptr_type(AddressSpace::default());

        if self.module.get_function("loz_llm_ask").is_none() {
            let function_type = pointer_type.fn_type(&[pointer_type.into()], false);
            self.module.add_function("loz_llm_ask", function_type, None);
        }
    }

    fn codegen_schema_descriptor_pointer(
        &self,
        schema_name: &str,
    ) -> CodegenResult<PointerValue<'ctx>> {
        let schema = self.schemas.get(schema_name).ok_or_else(|| {
            CodegenError::new(format!(
                "unknown schema '{}' during LLVM descriptor emission",
                schema_name
            ))
        })?;
        let global_name = format!("schema_{}_descriptor", schema_name);

        if let Some(global) = self.module.get_global(&global_name) {
            return Ok(global.as_pointer_value());
        }

        let descriptor = self.schema_descriptor_text(schema)?;
        let descriptor_bytes = descriptor.as_bytes();
        let array_type = self
            .context
            .i8_type()
            .array_type((descriptor_bytes.len() + 1) as u32);
        let global = self.module.add_global(array_type, None, &global_name);
        global.set_linkage(inkwell::module::Linkage::Private);
        global.set_constant(true);
        global.set_initializer(&self.context.const_string(descriptor_bytes, true));
        Ok(global.as_pointer_value())
    }

    fn schema_descriptor_text(&self, schema: &SchemaDeclaration) -> CodegenResult<String> {
        let mut descriptor = format!("{}|", schema.name);
        for (index, field) in schema.fields.iter().enumerate() {
            if index > 0 {
                descriptor.push(';');
            }
            descriptor.push_str(&field.name);
            descriptor.push(':');
            descriptor.push_str(self.schema_field_type_name(&field.type_name)?);
        }
        Ok(descriptor)
    }

    fn schema_field_type_name(&self, type_name: &TypeName) -> CodegenResult<&'static str> {
        match type_name {
            TypeName::Text => Ok("Text"),
            TypeName::Bool => Ok("Bool"),
            TypeName::I32 => Ok("i32"),
            TypeName::I64 => Ok("i64"),
            TypeName::F64 => Ok("f64"),
            TypeName::Json => Ok("Json"),
            other => Err(CodegenError::new(format!(
                "unsupported schema field type {:?} in LLVM descriptor emission",
                other
            ))),
        }
    }

    fn declare_global(&mut self, declaration: &VariableDeclaration) -> CodegenResult<()> {
        if declaration.is_mutable {
            return Err(CodegenError::new(format!(
                "LLVM backend does not support mutable global '{}' yet",
                declaration.name
            )));
        }

        let Some(declared_type) = &declaration.type_name else {
            return Err(CodegenError::new(format!(
                "LLVM backend requires an explicit type for global '{}'",
                declaration.name
            )));
        };

        if !self.is_primitive_type(declared_type) {
            return Err(CodegenError::new(format!(
                "LLVM backend only supports primitive global '{}' currently",
                declaration.name
            )));
        }

        if self.globals.contains_key(&declaration.name)
            || self.module.get_function(&declaration.name).is_some()
        {
            return Err(CodegenError::new(format!(
                "duplicate global name '{}' during LLVM declaration",
                declaration.name
            )));
        }

        let global_type = self.llvm_basic_type(declared_type)?;
        let initializer =
            self.evaluate_const_primitive_expression(declared_type, &declaration.value)?;

        let global = self.module.add_global(global_type, None, &declaration.name);
        global.set_initializer(&initializer);
        global.set_constant(true);
        self.globals.insert(declaration.name.clone(), global);
        self.global_types
            .insert(declaration.name.clone(), declared_type.clone());

        Ok(())
    }

    fn declare_function(
        &mut self,
        function: &FunctionDeclaration,
        current_module: Option<&str>,
    ) -> CodegenResult<()> {
        if self.module.get_function(&function.name).is_some()
            || self.globals.contains_key(&function.name)
            || self.functions.contains_key(&function.name)
            || self.tools.contains_key(&function.name)
        {
            return Err(CodegenError::new(format!(
                "duplicate function '{}' during LLVM declaration",
                function.name
            )));
        }

        let parameter_types = function
            .parameters
            .iter()
            .map(|parameter| self.llvm_basic_type(&parameter.type_name).map(Into::into))
            .collect::<CodegenResult<Vec<_>>>()?;

        let function_type = self.llvm_function_type(&function.return_type, &parameter_types)?;
        self.module
            .add_function(&function.name, function_type, None);
        if let Some(module_name) = current_module {
            self.module_functions.insert(
                (module_name.to_string(), function.name.clone()),
                function.clone(),
            );
        }
        self.functions
            .insert(function.name.clone(), function.clone());
        Ok(())
    }

    fn declare_tool(&mut self, tool: &ToolDeclaration) -> CodegenResult<()> {
        let symbol_name = self.tool_symbol_name(&tool.name);
        if self.module.get_function(&symbol_name).is_some()
            || self.globals.contains_key(&tool.name)
            || self.functions.contains_key(&tool.name)
            || self.tools.contains_key(&tool.name)
        {
            return Err(CodegenError::new(format!(
                "duplicate tool '{}' during LLVM declaration",
                tool.name
            )));
        }

        let parameter_types = tool
            .parameters
            .iter()
            .map(|parameter| self.llvm_basic_type(&parameter.type_name).map(Into::into))
            .collect::<CodegenResult<Vec<_>>>()?;

        let function_type = self.llvm_function_type(&tool.return_type, &parameter_types)?;
        self.module.add_function(&symbol_name, function_type, None);
        self.tools.insert(tool.name.clone(), tool.clone());
        Ok(())
    }

    fn declare_async_task(&mut self, task: &AsyncTaskDeclaration) -> CodegenResult<()> {
        let symbol_name = self.async_task_symbol_name(&task.name);
        if self.module.get_function(&symbol_name).is_some()
            || self.globals.contains_key(&task.name)
            || self.functions.contains_key(&task.name)
            || self.async_tasks.contains_key(&task.name)
            || self.tools.contains_key(&task.name)
        {
            return Err(CodegenError::new(format!(
                "duplicate async task '{}' during LLVM declaration",
                task.name
            )));
        }

        let parameter_types = task
            .parameters
            .iter()
            .map(|parameter| self.llvm_basic_type(&parameter.type_name).map(Into::into))
            .collect::<CodegenResult<Vec<_>>>()?;

        let function_type = self.llvm_function_type(&task.return_type, &parameter_types)?;
        self.module.add_function(&symbol_name, function_type, None);
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
        Ok(())
    }

    fn declare_agent(&mut self, agent: &AgentDeclaration) -> CodegenResult<()> {
        for task in &agent.tasks {
            self.declare_agent_task(&agent.name, task)?;
        }

        Ok(())
    }

    fn declare_workflow(&mut self, workflow: &WorkflowDeclaration) -> CodegenResult<()> {
        let symbol_name = self.workflow_symbol_name(&workflow.name);
        if self.module.get_function(&symbol_name).is_some() {
            return Err(CodegenError::new(format!(
                "duplicate workflow '{}' during LLVM declaration",
                workflow.name
            )));
        }

        let function_type = self.context.void_type().fn_type(&[], false);
        self.module.add_function(&symbol_name, function_type, None);
        Ok(())
    }

    fn declare_agent_task(
        &mut self,
        agent_name: &str,
        task: &AgentTaskDeclaration,
    ) -> CodegenResult<()> {
        let symbol_name = self.agent_task_symbol_name(agent_name, &task.name);
        if self.module.get_function(&symbol_name).is_some() {
            return Err(CodegenError::new(format!(
                "duplicate agent task '{}.{}' during LLVM declaration",
                agent_name, task.name
            )));
        }

        let parameter_types = task
            .parameters
            .iter()
            .map(|parameter| self.llvm_basic_type(&parameter.type_name).map(Into::into))
            .collect::<CodegenResult<Vec<_>>>()?;

        let function_type = self.llvm_function_type(&task.return_type, &parameter_types)?;
        self.module.add_function(&symbol_name, function_type, None);
        Ok(())
    }

    fn define_function_body(&self, function: &FunctionDeclaration) -> CodegenResult<()> {
        let function_value = self.module.get_function(&function.name).ok_or_else(|| {
            CodegenError::new(format!(
                "function '{}' was not declared before body generation",
                function.name
            ))
        })?;

        if !function_value.get_basic_blocks().is_empty() {
            return Err(CodegenError::new(format!(
                "function '{}' body was generated more than once",
                function.name
            )));
        }

        let entry = self.context.append_basic_block(function_value, "entry");
        self.builder.position_at_end(entry);

        let mut bindings = self.build_function_scope(function, function_value)?;
        let mut returned = false;

        for statement in &function.body {
            if self.codegen_function_statement(
                statement,
                function_value,
                &mut bindings,
                &function.return_type,
                None,
            )? {
                returned = true;
                break;
            }
        }

        if !returned {
            return Err(CodegenError::new(format!(
                "LLVM backend requires a return statement in function '{}'",
                function.name
            )));
        }

        Ok(())
    }

    fn define_tool_body(&self, tool: &ToolDeclaration) -> CodegenResult<()> {
        let lowered = self.lower_tool_function(tool);
        self.define_function_body(&lowered)
    }

    fn define_async_task_body(&self, task: &AsyncTaskDeclaration) -> CodegenResult<()> {
        let lowered = self.lower_async_task_function(task);
        self.define_function_body(&lowered)
    }

    fn define_agent_task_bodies(&self, agent: &AgentDeclaration) -> CodegenResult<()> {
        for task in &agent.tasks {
            let lowered = self.lower_agent_task_function(&agent.name, task);
            self.define_function_body(&lowered)?;
        }

        Ok(())
    }

    fn define_workflow_body(&self, workflow: &WorkflowDeclaration) -> CodegenResult<()> {
        let symbol_name = self.workflow_symbol_name(&workflow.name);
        let function_value = self.module.get_function(&symbol_name).ok_or_else(|| {
            CodegenError::new(format!(
                "workflow '{}' was not declared before body generation",
                workflow.name
            ))
        })?;

        if !function_value.get_basic_blocks().is_empty() {
            return Err(CodegenError::new(format!(
                "workflow '{}' body was generated more than once",
                workflow.name
            )));
        }

        let entry = self.context.append_basic_block(function_value, "entry");
        self.builder.position_at_end(entry);

        for step in &workflow.steps {
            self.codegen_workflow_step(step)?;
        }

        self.builder.build_return(None).map_err(|error| {
            CodegenError::new(format!(
                "failed to return from workflow '{}': {error}",
                workflow.name
            ))
        })?;

        Ok(())
    }

    fn lower_tool_function(&self, tool: &ToolDeclaration) -> FunctionDeclaration {
        FunctionDeclaration {
            name: self.tool_symbol_name(&tool.name),
            parameters: tool.parameters.clone(),
            return_type: tool.return_type.clone(),
            body: tool.body.clone(),
            span: tool.span.clone(),
        }
    }

    fn lower_async_task_function(&self, task: &AsyncTaskDeclaration) -> FunctionDeclaration {
        FunctionDeclaration {
            name: self.async_task_symbol_name(&task.name),
            parameters: task.parameters.clone(),
            return_type: task.return_type.clone(),
            body: task.body.clone(),
            span: task.span.clone(),
        }
    }

    fn lower_agent_task_function(
        &self,
        agent_name: &str,
        task: &AgentTaskDeclaration,
    ) -> FunctionDeclaration {
        FunctionDeclaration {
            name: self.agent_task_symbol_name(agent_name, &task.name),
            parameters: task.parameters.clone(),
            return_type: task.return_type.clone(),
            body: task.body.clone(),
            span: task.span.clone(),
        }
    }

    fn codegen_workflow_step(&self, step: &loz_ast::WorkflowStep) -> CodegenResult<()> {
        let target_function = match &step.target {
            WorkflowTarget::FunctionOrTool(target_name) => {
                if self.functions.contains_key(target_name) {
                    self.module.get_function(target_name).ok_or_else(|| {
                        CodegenError::new(format!(
                            "workflow step '{}' target function '{}' is missing from LLVM module",
                            step.name, target_name
                        ))
                    })?
                } else if self.tools.contains_key(target_name) {
                    let symbol_name = self.tool_symbol_name(target_name);
                    self.module.get_function(&symbol_name).ok_or_else(|| {
                        CodegenError::new(format!(
                            "workflow step '{}' target tool '{}' is missing from LLVM module",
                            step.name, target_name
                        ))
                    })?
                } else {
                    return Err(CodegenError::new(format!(
                        "workflow step '{}' refers to unknown function/tool '{}'",
                        step.name, target_name
                    )));
                }
            }
            WorkflowTarget::AgentTask {
                agent_name,
                task_name,
            } => {
                let symbol_name = self.agent_task_symbol_name(agent_name, task_name);
                self.module.get_function(&symbol_name).ok_or_else(|| {
                    CodegenError::new(format!(
                        "workflow step '{}' target agent task '{}.{}' is missing from LLVM module",
                        step.name, agent_name, task_name
                    ))
                })?
            }
        };

        self.builder
            .build_call(
                target_function,
                &[],
                &format!("workflow_step_{}", step.name),
            )
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to build workflow step call '{}': {error}",
                    step.name
                ))
            })?;

        Ok(())
    }

    fn build_function_scope(
        &self,
        function: &FunctionDeclaration,
        function_value: FunctionValue<'ctx>,
    ) -> CodegenResult<HashMap<String, FunctionBinding<'ctx>>> {
        let mut bindings = HashMap::new();

        for (index, parameter) in function.parameters.iter().enumerate() {
            let value = function_value.get_nth_param(index as u32).ok_or_else(|| {
                CodegenError::new(format!(
                    "missing LLVM parameter {} for function '{}'",
                    index, function.name
                ))
            })?;

            bindings.insert(
                parameter.name.clone(),
                FunctionBinding::Parameter {
                    value,
                    type_name: parameter.type_name.clone(),
                },
            );
        }

        Ok(bindings)
    }

    fn codegen_function_statement(
        &self,
        statement: &Statement,
        current_function: FunctionValue<'ctx>,
        bindings: &mut HashMap<String, FunctionBinding<'ctx>>,
        current_return_type: &TypeName,
        loop_targets: Option<LoopTargets<'ctx>>,
    ) -> CodegenResult<bool> {
        match statement {
            Statement::VariableDeclaration(declaration) => {
                self.codegen_local_variable_declaration(declaration, current_function, bindings)?;
                Ok(false)
            }
            Statement::Assignment(assignment) => {
                self.codegen_assignment_statement(assignment, current_function, bindings)?;
                Ok(false)
            }
            Statement::If(if_statement) => self.codegen_if_statement(
                if_statement,
                current_function,
                bindings,
                current_return_type,
                loop_targets,
            ),
            Statement::While(while_statement) => {
                self.codegen_while_statement(
                    while_statement,
                    current_function,
                    bindings,
                    current_return_type,
                    loop_targets,
                )?;
                Ok(false)
            }
            Statement::For(for_statement) => {
                self.codegen_for_statement(
                    for_statement,
                    current_function,
                    bindings,
                    current_return_type,
                    loop_targets,
                )?;
                Ok(false)
            }
            Statement::Break(_) => {
                let Some(loop_targets) = loop_targets else {
                    return Err(CodegenError::new(
                        "break statement is only valid inside a loop in LLVM backend",
                    ));
                };

                self.builder
                    .build_unconditional_branch(loop_targets.break_block)
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build break branch: {error}"))
                    })?;
                Ok(true)
            }
            Statement::Continue(_) => {
                let Some(loop_targets) = loop_targets else {
                    return Err(CodegenError::new(
                        "continue statement is only valid inside a loop in LLVM backend",
                    ));
                };

                self.builder
                    .build_unconditional_branch(loop_targets.continue_block)
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build continue branch: {error}"))
                    })?;
                Ok(true)
            }
            Statement::Return(return_statement) => {
                let value = self.codegen_value_expression(
                    current_return_type,
                    &return_statement.value,
                    current_function,
                    bindings,
                )?;
                self.builder.build_return(Some(&value)).map_err(|error| {
                    CodegenError::new(format!("failed to build return: {error}"))
                })?;
                Ok(true)
            }
            Statement::Expression(expression) => {
                self.codegen_expression_statement(expression, current_function, bindings)?;
                Ok(false)
            }
            Statement::ModuleDeclaration(_) => Err(CodegenError::new(
                "module declarations are only supported at the top level in LLVM backend",
            )),
            Statement::ImportDeclaration(_) => Err(CodegenError::new(
                "import declarations are only supported at the top level in LLVM backend",
            )),
            Statement::StructDeclaration(_) => Err(CodegenError::new(
                "struct declarations are only supported at the top level in LLVM backend",
            )),
            Statement::SchemaDeclaration(_) => Err(CodegenError::new(
                "schema declarations are only supported at the top level in LLVM backend",
            )),
            Statement::FunctionDeclaration(_) => Err(CodegenError::new(
                "nested function declarations are not supported in LLVM backend",
            )),
            Statement::AsyncTaskDeclaration(_) => Err(CodegenError::new(
                "async task declarations are only supported at the top level in LLVM backend",
            )),
            Statement::ToolDeclaration(_) => Err(CodegenError::new(
                "tool declarations are only supported at the top level in LLVM backend",
            )),
            Statement::AgentDeclaration(_) => Err(CodegenError::new(
                "agent declarations are only supported at the top level in LLVM backend",
            )),
            Statement::WorkflowDeclaration(_) => Err(CodegenError::new(
                "workflow declarations are only supported at the top level in LLVM backend",
            )),
            Statement::ImplBlock(_) => Err(CodegenError::new(
                "impl blocks are only supported at the top level in LLVM backend",
            )),
        }
    }

    fn codegen_for_statement(
        &self,
        for_statement: &ForStatement,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
        current_return_type: &TypeName,
        _outer_loop_targets: Option<LoopTargets<'ctx>>,
    ) -> CodegenResult<()> {
        let iterable_type =
            self.infer_expression_type(&for_statement.iterable, bindings, current_function)?;
        let TypeName::Array(element_type, Some(length)) = &iterable_type else {
            return Err(CodegenError::new(format!(
                "for loop source must be a fixed Array value in LLVM backend, found {:?}",
                iterable_type
            )));
        };

        let array_type = self.llvm_array_type(&iterable_type)?;
        let iterable_pointer = self.create_entry_alloca(
            current_function,
            &format!("for_iter_{}", for_statement.variable_name),
            array_type.into(),
        )?;
        let iterable_value = self.codegen_value_expression(
            &iterable_type,
            &for_statement.iterable,
            current_function,
            bindings,
        )?;
        self.builder
            .build_store(iterable_pointer, iterable_value)
            .map_err(|error| {
                CodegenError::new(format!("failed to materialize for-loop iterable: {error}"))
            })?;

        let index_type = self.context.i64_type();
        let index_pointer = self.create_entry_alloca(
            current_function,
            &format!("for_idx_{}", for_statement.variable_name),
            index_type.into(),
        )?;
        self.builder
            .build_store(index_pointer, index_type.const_zero())
            .map_err(|error| {
                CodegenError::new(format!("failed to initialize for-loop index: {error}"))
            })?;

        let loop_variable_pointer = self.create_entry_alloca(
            current_function,
            &for_statement.variable_name,
            self.llvm_basic_type(element_type)?,
        )?;

        let condition_block = self.context.append_basic_block(current_function, "forcond");
        let body_block = self.context.append_basic_block(current_function, "forbody");
        let step_block = self.context.append_basic_block(current_function, "forstep");
        let exit_block = self.context.append_basic_block(current_function, "forexit");

        self.builder
            .build_unconditional_branch(condition_block)
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to branch to for-loop condition block: {error}"
                ))
            })?;

        self.builder.position_at_end(condition_block);
        let current_index = self
            .builder
            .build_load(index_type, index_pointer, "for_idx")
            .map_err(|error| CodegenError::new(format!("failed to load for-loop index: {error}")))?
            .into_int_value();
        let length_value = index_type.const_int(*length as u64, false);
        let condition = self
            .builder
            .build_int_compare(
                IntPredicate::ULT,
                current_index,
                length_value,
                "for_has_next",
            )
            .map_err(|error| {
                CodegenError::new(format!("failed to compare for-loop index: {error}"))
            })?;
        self.builder
            .build_conditional_branch(condition, body_block, exit_block)
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to build for-loop conditional branch: {error}"
                ))
            })?;

        self.builder.position_at_end(body_block);
        let iteration_index = self
            .builder
            .build_load(index_type, index_pointer, "for_idx_iter")
            .map_err(|error| {
                CodegenError::new(format!("failed to reload for-loop index: {error}"))
            })?
            .into_int_value();
        let zero = index_type.const_zero();
        let element_pointer = unsafe {
            self.builder.build_in_bounds_gep(
                array_type,
                iterable_pointer,
                &[zero, iteration_index],
                &format!("for_elem_ptr_{}", for_statement.variable_name),
            )
        }
        .map_err(|error| {
            CodegenError::new(format!("failed to address for-loop array element: {error}"))
        })?;
        let element_value = self
            .builder
            .build_load(
                self.llvm_basic_type(element_type)?,
                element_pointer,
                &format!("for_elem_{}", for_statement.variable_name),
            )
            .map_err(|error| {
                CodegenError::new(format!("failed to load for-loop array element: {error}"))
            })?;
        self.builder
            .build_store(loop_variable_pointer, element_value)
            .map_err(|error| {
                CodegenError::new(format!("failed to update for-loop variable: {error}"))
            })?;

        let mut body_bindings = bindings.clone();
        body_bindings.insert(
            for_statement.variable_name.clone(),
            FunctionBinding::Local {
                pointer: loop_variable_pointer,
                is_mutable: for_statement.is_mutable,
                type_name: (**element_type).clone(),
            },
        );

        let loop_targets = LoopTargets {
            break_block: exit_block,
            continue_block: step_block,
        };
        let body_terminated = self.codegen_branch_statements(
            &for_statement.body,
            current_function,
            &mut body_bindings,
            current_return_type,
            Some(loop_targets),
        )?;
        if !body_terminated {
            self.builder
                .build_unconditional_branch(step_block)
                .map_err(|error| {
                    CodegenError::new(format!("failed to branch to for-loop step block: {error}"))
                })?;
        }

        self.builder.position_at_end(step_block);
        let step_index = self
            .builder
            .build_load(index_type, index_pointer, "for_idx_step")
            .map_err(|error| {
                CodegenError::new(format!("failed to load for-loop step index: {error}"))
            })?
            .into_int_value();
        let next_index = self
            .builder
            .build_int_add(step_index, index_type.const_int(1, false), "for_idx_next")
            .map_err(|error| {
                CodegenError::new(format!("failed to increment for-loop index: {error}"))
            })?;
        self.builder
            .build_store(index_pointer, next_index)
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to store incremented for-loop index: {error}"
                ))
            })?;
        self.builder
            .build_unconditional_branch(condition_block)
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to branch back to for-loop condition block: {error}"
                ))
            })?;

        self.builder.position_at_end(exit_block);
        Ok(())
    }

    fn codegen_assignment_statement(
        &self,
        assignment: &loz_ast::AssignmentStatement,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<()> {
        match &assignment.target {
            AssignmentTarget::Identifier(name) => {
                let binding = bindings.get(name).cloned().ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown local identifier '{}' during LLVM assignment",
                        name
                    ))
                })?;

                let FunctionBinding::Local {
                    pointer,
                    is_mutable,
                    type_name,
                } = binding
                else {
                    return Err(CodegenError::new(format!(
                        "LLVM backend only supports assignment to local variables, not '{}'",
                        name
                    )));
                };

                if !is_mutable {
                    return Err(CodegenError::new(format!(
                        "cannot assign to immutable local '{}'",
                        name
                    )));
                }

                let value = self.codegen_value_expression(
                    &type_name,
                    &assignment.value,
                    current_function,
                    bindings,
                )?;
                self.builder.build_store(pointer, value).map_err(|error| {
                    CodegenError::new(format!("failed to store local '{}': {error}", name))
                })?;
                Ok(())
            }
            AssignmentTarget::Dereference(dereference) => {
                let (dereference_type, _, pointer) =
                    self.codegen_dereference_expression(dereference, current_function, bindings)?;
                let reference_type =
                    self.infer_expression_type(&dereference.value, bindings, current_function)?;
                let TypeName::Reference { is_mutable, .. } = reference_type else {
                    return Err(CodegenError::new(
                        "cannot assign through non-reference value in LLVM backend",
                    ));
                };

                if !is_mutable {
                    return Err(CodegenError::new(
                        "cannot assign through immutable reference in LLVM backend",
                    ));
                }

                let value = self.codegen_value_expression(
                    &dereference_type,
                    &assignment.value,
                    current_function,
                    bindings,
                )?;
                self.builder.build_store(pointer, value).map_err(|error| {
                    CodegenError::new(format!(
                        "failed to store through dereference assignment: {error}"
                    ))
                })?;
                Ok(())
            }
            AssignmentTarget::FieldAccess(field_access) => self.codegen_field_assignment_statement(
                &field_access.base_name,
                &field_access.field_name,
                &assignment.value,
                current_function,
                bindings,
            ),
            AssignmentTarget::IndexAccess(index_access) => self
                .codegen_array_element_assignment_statement(
                    &index_access.base_name,
                    &index_access.index,
                    &assignment.value,
                    current_function,
                    bindings,
                ),
        }
    }

    fn codegen_field_assignment_statement(
        &self,
        base_name: &str,
        field_name: &str,
        value_expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<()> {
        let binding = bindings.get(base_name).cloned().ok_or_else(|| {
            CodegenError::new(format!(
                "unknown local identifier '{}' during LLVM field assignment",
                base_name
            ))
        })?;

        let FunctionBinding::Local {
            pointer,
            is_mutable,
            type_name: TypeName::Named(struct_name),
        } = binding
        else {
            return Err(CodegenError::new(format!(
                "LLVM backend only supports field assignment on local struct variables, not '{}'",
                base_name
            )));
        };

        if !is_mutable {
            return Err(CodegenError::new(format!(
                "cannot assign to immutable local struct '{}'",
                base_name
            )));
        }

        let struct_declaration = self.structs.get(&struct_name).ok_or_else(|| {
            CodegenError::new(format!(
                "unknown struct '{}' during LLVM field assignment",
                struct_name
            ))
        })?;

        let (field_index, field) = struct_declaration
            .fields
            .iter()
            .enumerate()
            .find(|(_, field)| field.name == field_name)
            .ok_or_else(|| {
                CodegenError::new(format!(
                    "struct '{}' has no field '{}' during LLVM field assignment",
                    struct_name, field_name
                ))
            })?;

        let struct_type = self.llvm_struct_type(&TypeName::Named(struct_name.clone()))?;
        let loaded_struct = self
            .builder
            .build_load(struct_type, pointer, &format!("load_{base_name}"))
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to load struct local '{}' for field assignment: {error}",
                    base_name
                ))
            })?
            .into_struct_value();

        let field_value = self.codegen_value_expression(
            &field.type_name,
            value_expression,
            current_function,
            bindings,
        )?;

        let updated_struct = self
            .builder
            .build_insert_value(
                loaded_struct,
                field_value,
                field_index as u32,
                &format!("insert_{base_name}_{field_name}"),
            )
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to update field '{}.{}': {error}",
                    base_name, field_name
                ))
            })?
            .into_struct_value();

        self.builder
            .build_store(pointer, updated_struct)
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to store updated struct '{}': {error}",
                    base_name
                ))
            })?;

        Ok(())
    }

    fn codegen_array_element_assignment_statement(
        &self,
        base_name: &str,
        index_expression: &Expression,
        value_expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<()> {
        let binding = bindings.get(base_name).cloned().ok_or_else(|| {
            CodegenError::new(format!(
                "unknown local identifier '{}' during LLVM array element assignment",
                base_name
            ))
        })?;

        let FunctionBinding::Local {
            pointer,
            is_mutable,
            type_name: TypeName::Array(element_type, Some(length)),
        } = binding
        else {
            return Err(CodegenError::new(format!(
                "LLVM backend only supports array element assignment on local fixed arrays, not '{}'",
                base_name
            )));
        };

        if !is_mutable {
            return Err(CodegenError::new(format!(
                "cannot assign to immutable local array '{}'",
                base_name
            )));
        }

        let index =
            self.const_index_from_expression(index_expression, current_function, bindings)?;
        if index >= length {
            return Err(CodegenError::new(format!(
                "array index {} is out of bounds for '{}'",
                index, base_name
            )));
        }

        let array_type =
            self.llvm_array_type(&TypeName::Array(element_type.clone(), Some(length)))?;
        let loaded_array = self
            .builder
            .build_load(array_type, pointer, &format!("load_{base_name}"))
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to load array local '{}' for element assignment: {error}",
                    base_name
                ))
            })?
            .into_array_value();

        let element_value = self.codegen_value_expression(
            &element_type,
            value_expression,
            current_function,
            bindings,
        )?;

        let updated_array = self
            .builder
            .build_insert_value(
                loaded_array,
                element_value,
                index as u32,
                &format!("insert_{base_name}_{index}"),
            )
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to update array element '{}[{}]': {error}",
                    base_name, index
                ))
            })?
            .into_array_value();

        self.builder
            .build_store(pointer, updated_array)
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to store updated array '{}': {error}",
                    base_name
                ))
            })?;

        Ok(())
    }

    fn codegen_expression_statement(
        &self,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<()> {
        if let ExpressionKind::Call(call) = &expression.kind {
            if call.callee == "print" {
                if call.arguments.len() != 1 {
                    return Err(CodegenError::new(
                        "print() expects exactly one argument in LLVM backend",
                    ));
                }

                match &call.arguments[0].kind {
                    ExpressionKind::StringLiteral(value) => {
                        let puts = self.module.get_function("puts").ok_or_else(|| {
                            CodegenError::new("puts declaration is missing from LLVM module")
                        })?;
                        let string_literal = self
                            .builder
                            .build_global_string_ptr(value, "puts_text")
                            .map_err(|error| {
                                CodegenError::new(format!(
                                    "failed to build string literal for print(Text): {error}"
                                ))
                            })?;

                        self.builder
                            .build_call(
                                puts,
                                &[string_literal.as_pointer_value().into()],
                                "puts_call",
                            )
                            .map_err(|error| {
                                CodegenError::new(format!("failed to build puts call: {error}"))
                            })?;
                    }
                    _ => {
                        self.codegen_print_expression(
                            &call.arguments[0],
                            current_function,
                            bindings,
                        )?;
                    }
                }

                return Ok(());
            }

            self.codegen_call_expression(call, current_function, bindings)?;
            return Ok(());
        }

        if let ExpressionKind::MethodCall(method_call) = &expression.kind {
            if method_call.base_name == "io"
                || method_call.base_name == "json"
                || method_call.base_name == "schema"
            {
                let method_type =
                    self.infer_method_call_type(method_call, bindings, current_function)?;
                self.codegen_method_call_value(
                    &method_type,
                    method_call,
                    current_function,
                    bindings,
                )?;
                return Ok(());
            }

            match method_call.method_name.as_str() {
                "len" => {
                    self.codegen_array_len_expression(method_call, bindings)?;
                    return Ok(());
                }
                "push" => {
                    return Err(CodegenError::new(
                        "LLVM backend does not support Array.push() yet; use the interpreter path for dynamic array mutation",
                    ));
                }
                "pop" => {
                    return Err(CodegenError::new(
                        "LLVM backend does not support Array.pop() yet; use the interpreter path for dynamic array mutation",
                    ));
                }
                other => {
                    return Err(CodegenError::new(format!(
                        "unknown method '{}()' in LLVM backend",
                        other
                    )));
                }
            }
        }

        let expression_type = self.infer_expression_type(expression, bindings, current_function)?;
        if self.is_primitive_type(&expression_type) {
            self.codegen_primitive_expression(
                &expression_type,
                expression,
                current_function,
                bindings,
            )?;
        }
        Ok(())
    }

    fn codegen_if_statement(
        &self,
        if_statement: &IfStatement,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
        current_return_type: &TypeName,
        loop_targets: Option<LoopTargets<'ctx>>,
    ) -> CodegenResult<bool> {
        if let ExpressionKind::BooleanLiteral(condition) = &if_statement.condition.kind {
            if *condition {
                let mut then_bindings = bindings.clone();
                return self.codegen_branch_statements(
                    &if_statement.then_branch,
                    current_function,
                    &mut then_bindings,
                    current_return_type,
                    loop_targets,
                );
            }

            if let Some(else_branch) = &if_statement.else_branch {
                let mut else_bindings = bindings.clone();
                return self.codegen_branch_statements(
                    else_branch,
                    current_function,
                    &mut else_bindings,
                    current_return_type,
                    loop_targets,
                );
            }

            return Ok(false);
        }

        let condition =
            self.codegen_condition_expression(&if_statement.condition, current_function, bindings)?;

        let then_block = self.context.append_basic_block(current_function, "then");
        let else_block = self.context.append_basic_block(current_function, "else");
        let merge_block = self.context.append_basic_block(current_function, "ifcont");

        self.builder
            .build_conditional_branch(condition, then_block, else_block)
            .map_err(|error| {
                CodegenError::new(format!("failed to build conditional branch: {error}"))
            })?;

        self.builder.position_at_end(then_block);
        let mut then_bindings = bindings.clone();
        let then_terminated = self.codegen_branch_statements(
            &if_statement.then_branch,
            current_function,
            &mut then_bindings,
            current_return_type,
            loop_targets,
        )?;
        if !then_terminated {
            self.builder
                .build_unconditional_branch(merge_block)
                .map_err(|error| {
                    CodegenError::new(format!("failed to branch from then block: {error}"))
                })?;
        }

        self.builder.position_at_end(else_block);
        let mut else_bindings = bindings.clone();
        let else_terminated = if let Some(else_branch) = &if_statement.else_branch {
            self.codegen_branch_statements(
                else_branch,
                current_function,
                &mut else_bindings,
                current_return_type,
                loop_targets,
            )?
        } else {
            false
        };
        if !else_terminated {
            self.builder
                .build_unconditional_branch(merge_block)
                .map_err(|error| {
                    CodegenError::new(format!("failed to branch from else block: {error}"))
                })?;
        }

        if then_terminated && else_terminated {
            Ok(true)
        } else {
            self.builder.position_at_end(merge_block);
            Ok(false)
        }
    }

    fn codegen_while_statement(
        &self,
        while_statement: &WhileStatement,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
        current_return_type: &TypeName,
        _outer_loop_targets: Option<LoopTargets<'ctx>>,
    ) -> CodegenResult<()> {
        let condition_block = self
            .context
            .append_basic_block(current_function, "loopcond");
        let body_block = self
            .context
            .append_basic_block(current_function, "loopbody");
        let exit_block = self
            .context
            .append_basic_block(current_function, "loopexit");

        self.builder
            .build_unconditional_branch(condition_block)
            .map_err(|error| {
                CodegenError::new(format!("failed to branch to loop condition block: {error}"))
            })?;

        self.builder.position_at_end(condition_block);
        let condition = self.codegen_condition_expression(
            &while_statement.condition,
            current_function,
            bindings,
        )?;
        self.builder
            .build_conditional_branch(condition, body_block, exit_block)
            .map_err(|error| {
                CodegenError::new(format!("failed to build while conditional branch: {error}"))
            })?;

        self.builder.position_at_end(body_block);
        let mut body_bindings = bindings.clone();
        let loop_targets = LoopTargets {
            break_block: exit_block,
            continue_block: condition_block,
        };
        let body_terminated = self.codegen_branch_statements(
            &while_statement.body,
            current_function,
            &mut body_bindings,
            current_return_type,
            Some(loop_targets),
        )?;
        if !body_terminated {
            self.builder
                .build_unconditional_branch(condition_block)
                .map_err(|error| {
                    CodegenError::new(format!(
                        "failed to branch back to loop condition block: {error}"
                    ))
                })?;
        }

        self.builder.position_at_end(exit_block);
        Ok(())
    }

    fn codegen_branch_statements(
        &self,
        statements: &[Statement],
        current_function: FunctionValue<'ctx>,
        bindings: &mut HashMap<String, FunctionBinding<'ctx>>,
        current_return_type: &TypeName,
        loop_targets: Option<LoopTargets<'ctx>>,
    ) -> CodegenResult<bool> {
        for statement in statements {
            if self.codegen_function_statement(
                statement,
                current_function,
                bindings,
                current_return_type,
                loop_targets,
            )? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn codegen_local_variable_declaration(
        &self,
        declaration: &VariableDeclaration,
        current_function: FunctionValue<'ctx>,
        bindings: &mut HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<()> {
        if bindings.contains_key(&declaration.name) {
            return Err(CodegenError::new(format!(
                "duplicate local identifier '{}' during LLVM codegen",
                declaration.name
            )));
        }

        let concrete_type =
            self.resolve_declaration_type(declaration, current_function, bindings)?;
        let alloca_type = self.llvm_basic_type(&concrete_type)?;
        let pointer = self.create_entry_alloca(current_function, &declaration.name, alloca_type)?;
        let value = self.codegen_value_expression(
            &concrete_type,
            &declaration.value,
            current_function,
            bindings,
        )?;
        self.builder.build_store(pointer, value).map_err(|error| {
            CodegenError::new(format!(
                "failed to initialize local '{}': {error}",
                declaration.name
            ))
        })?;

        bindings.insert(
            declaration.name.clone(),
            FunctionBinding::Local {
                pointer,
                is_mutable: declaration.is_mutable,
                type_name: concrete_type,
            },
        );

        Ok(())
    }

    fn create_entry_alloca(
        &self,
        function: FunctionValue<'ctx>,
        name: &str,
        value_type: BasicTypeEnum<'ctx>,
    ) -> CodegenResult<PointerValue<'ctx>> {
        let entry = function
            .get_first_basic_block()
            .ok_or_else(|| CodegenError::new("function is missing an entry block for alloca"))?;

        let builder = self.context.create_builder();
        if let Some(first_instruction) = entry.get_first_instruction() {
            builder.position_before(&first_instruction);
        } else {
            builder.position_at_end(entry);
        }

        builder.build_alloca(value_type, name).map_err(|error| {
            CodegenError::new(format!("failed to allocate local '{name}': {error}"))
        })
    }

    fn llvm_function_type(
        &self,
        return_type: &TypeName,
        parameter_types: &[inkwell::types::BasicMetadataTypeEnum<'ctx>],
    ) -> CodegenResult<inkwell::types::FunctionType<'ctx>> {
        match return_type {
            TypeName::Void => Ok(self.context.void_type().fn_type(parameter_types, false)),
            _ => Ok(self
                .llvm_basic_type(return_type)?
                .fn_type(parameter_types, false)),
        }
    }

    fn is_primitive_type(&self, type_name: &TypeName) -> bool {
        matches!(type_name, TypeName::Bool | TypeName::Text | TypeName::Char)
            || type_name.is_numeric()
    }

    fn is_reference_type(&self, type_name: &TypeName) -> bool {
        matches!(type_name, TypeName::Reference { .. })
    }

    fn llvm_basic_type(&self, type_name: &TypeName) -> CodegenResult<BasicTypeEnum<'ctx>> {
        self.llvm_basic_type_with_stack(type_name, &mut Vec::new())
    }

    fn llvm_basic_type_with_stack(
        &self,
        type_name: &TypeName,
        stack: &mut Vec<String>,
    ) -> CodegenResult<BasicTypeEnum<'ctx>> {
        match type_name {
            TypeName::I8 | TypeName::U8 => Ok(self.context.i8_type().into()),
            TypeName::I16 | TypeName::U16 => Ok(self.context.i16_type().into()),
            TypeName::I32 | TypeName::U32 => Ok(self.context.i32_type().into()),
            TypeName::I64 | TypeName::U64 => Ok(self.context.i64_type().into()),
            TypeName::F32 => Ok(self.context.f32_type().into()),
            TypeName::F64 => Ok(self.context.f64_type().into()),
            TypeName::Bool => Ok(self.context.bool_type().into()),
            TypeName::Text => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            TypeName::Json => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            TypeName::Reference { .. } => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            TypeName::Array(element_type, Some(length)) => {
                let element_type = self.llvm_basic_type_with_stack(element_type, stack)?;
                Ok(element_type.array_type(*length as u32).into())
            }
            TypeName::Array(_, None) => Err(CodegenError::new(
                "LLVM backend requires a concrete array length before lowering Array values",
            )),
            TypeName::Named(name) => {
                let struct_declaration = self.structs.get(name).ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown struct type '{}' during LLVM codegen",
                        name
                    ))
                })?;

                if stack.iter().any(|entry| entry == name) {
                    return Err(CodegenError::new(format!(
                        "recursive struct type '{}' is not supported in LLVM backend",
                        name
                    )));
                }

                stack.push(name.clone());
                let field_types = struct_declaration
                    .fields
                    .iter()
                    .map(|field| self.llvm_basic_type_with_stack(&field.type_name, stack))
                    .collect::<CodegenResult<Vec<_>>>()?;
                stack.pop();

                Ok(self.context.struct_type(&field_types, false).into())
            }
            _ => Err(CodegenError::new(format!(
                "LLVM backend only supports primitive, fixed Array, and struct value types currently, found {:?}",
                type_name
            ))),
        }
    }

    fn llvm_struct_type(&self, type_name: &TypeName) -> CodegenResult<StructType<'ctx>> {
        match self.llvm_basic_type(type_name)? {
            BasicTypeEnum::StructType(struct_type) => Ok(struct_type),
            _ => Err(CodegenError::new(format!(
                "type {:?} is not a struct type in LLVM backend",
                type_name
            ))),
        }
    }

    fn llvm_array_type(&self, type_name: &TypeName) -> CodegenResult<ArrayType<'ctx>> {
        match self.llvm_basic_type(type_name)? {
            BasicTypeEnum::ArrayType(array_type) => Ok(array_type),
            _ => Err(CodegenError::new(format!(
                "type {:?} is not an array type in LLVM backend",
                type_name
            ))),
        }
    }

    fn codegen_value_expression(
        &self,
        expected_type: &TypeName,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        if let ExpressionKind::Await(await_expression) = &expression.kind {
            let awaited_type =
                self.infer_expression_type(expression, bindings, current_function)?;
            if &awaited_type != expected_type {
                return Err(CodegenError::new(format!(
                    "await expression produced type {:?}, expected {:?}",
                    awaited_type, expected_type
                )));
            }

            return self.codegen_await_expression(await_expression, current_function, bindings);
        }

        if let ExpressionKind::FieldAccess(field_access) = &expression.kind {
            let (field_type, field_value) = self.codegen_field_access_expression(
                &field_access.base_name,
                &field_access.field_name,
                current_function,
                bindings,
            )?;

            if &field_type != expected_type {
                return Err(CodegenError::new(format!(
                    "field access '{}.{}' produced type {:?}, expected {:?}",
                    field_access.base_name, field_access.field_name, field_type, expected_type
                )));
            }

            return Ok(field_value);
        }

        if let ExpressionKind::IndexAccess(index_access) = &expression.kind {
            let (element_type, element_value) = self.codegen_index_access_expression(
                &index_access.base_name,
                &index_access.index,
                current_function,
                bindings,
            )?;

            if &element_type != expected_type {
                return Err(CodegenError::new(format!(
                    "index access '{}[...]' produced type {:?}, expected {:?}",
                    index_access.base_name, element_type, expected_type
                )));
            }

            return Ok(element_value);
        }

        if let ExpressionKind::Dereference(dereference) = &expression.kind {
            let (dereference_type, dereference_value, _) =
                self.codegen_dereference_expression(dereference, current_function, bindings)?;

            if &dereference_type != expected_type {
                return Err(CodegenError::new(format!(
                    "dereference expression produced type {:?}, expected {:?}",
                    dereference_type, expected_type
                )));
            }

            return Ok(dereference_value);
        }

        if let ExpressionKind::MethodCall(method_call) = &expression.kind {
            let method_type =
                self.infer_method_call_type(method_call, bindings, current_function)?;
            if &method_type != expected_type {
                return Err(CodegenError::new(format!(
                    "method call '{}.{}()' produced type {:?}, expected {:?}",
                    method_call.base_name, method_call.method_name, method_type, expected_type
                )));
            }

            return self.codegen_method_call_value(
                expected_type,
                method_call,
                current_function,
                bindings,
            );
        }

        match expected_type {
            type_name if self.is_primitive_type(type_name) => {
                self.codegen_primitive_expression(type_name, expression, current_function, bindings)
            }
            type_name if self.is_reference_type(type_name) => self
                .codegen_reference_expression(type_name, expression, bindings)
                .map(Into::into),
            TypeName::Array(_, _) => self
                .codegen_array_expression(expected_type, expression, current_function, bindings)
                .map(Into::into),
            TypeName::Named(_) => self
                .codegen_struct_expression(expected_type, expression, current_function, bindings)
                .map(Into::into),
            TypeName::Json => self
                .codegen_json_expression(expression, current_function, bindings)
                .map(Into::into),
            _ => Err(CodegenError::new(format!(
                "LLVM backend only supports primitive, fixed Array, struct, and Json value expressions currently, found {:?}",
                expected_type
            ))),
        }
    }

    fn codegen_index_access_expression(
        &self,
        base_name: &str,
        index_expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<(TypeName, BasicValueEnum<'ctx>)> {
        let binding = bindings.get(base_name).cloned().ok_or_else(|| {
            CodegenError::new(format!(
                "unknown identifier '{}' during LLVM array index access",
                base_name
            ))
        })?;

        let (element_type, array_length, array_value) = match binding {
            FunctionBinding::Local {
                pointer,
                type_name: TypeName::Array(element_type, Some(length)),
                ..
            } => {
                let array_type =
                    self.llvm_array_type(&TypeName::Array(element_type.clone(), Some(length)))?;
                let array_value = self
                    .builder
                    .build_load(array_type, pointer, &format!("load_{base_name}"))
                    .map_err(|error| {
                        CodegenError::new(format!(
                            "failed to load array local '{}' for index access: {error}",
                            base_name
                        ))
                    })?
                    .into_array_value();
                (element_type, length, array_value)
            }
            FunctionBinding::Parameter {
                value,
                type_name: TypeName::Array(element_type, Some(length)),
            } => (element_type, length, value.into_array_value()),
            _ => {
                return Err(CodegenError::new(format!(
                    "index access is only supported on fixed array values, not '{}'",
                    base_name
                )));
            }
        };

        let index =
            self.const_index_from_expression(index_expression, current_function, bindings)?;
        if index >= array_length {
            return Err(CodegenError::new(format!(
                "array index {} is out of bounds for '{}'",
                index, base_name
            )));
        }
        let element_value = self
            .builder
            .build_extract_value(
                array_value,
                index as u32,
                &format!("extract_{base_name}_{index}"),
            )
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to extract array element '{}[{}]': {error}",
                    base_name, index
                ))
            })?;

        Ok((*element_type, element_value))
    }

    fn codegen_dereference_expression(
        &self,
        dereference: &DereferenceExpression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<(TypeName, BasicValueEnum<'ctx>, PointerValue<'ctx>)> {
        let reference_type =
            self.infer_expression_type(&dereference.value, bindings, current_function)?;
        let TypeName::Reference { inner, .. } = &reference_type else {
            return Err(CodegenError::new(
                "cannot dereference a non-reference value in LLVM backend",
            ));
        };

        let pointer =
            self.codegen_reference_expression(&reference_type, &dereference.value, bindings)?;
        let value = self
            .builder
            .build_load(self.llvm_basic_type(inner)?, pointer, "deref_load")
            .map_err(|error| {
                CodegenError::new(format!("failed to load dereferenced value: {error}"))
            })?;

        Ok((*inner.clone(), value, pointer))
    }

    fn infer_method_call_type(
        &self,
        method_call: &MethodCallExpression,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
        current_function: FunctionValue<'ctx>,
    ) -> CodegenResult<TypeName> {
        if method_call.base_name == "io" {
            return self.infer_io_method_call_type(method_call);
        }
        if method_call.base_name == "json" {
            return self.infer_json_method_call_type(method_call);
        }
        if method_call.base_name == "schema" {
            return self.infer_schema_method_call_type(method_call, bindings, current_function);
        }
        if method_call.base_name == "python" {
            return self.infer_python_method_call_type(method_call, bindings, current_function);
        }
        if method_call.base_name == "llm" {
            return self.infer_llm_method_call_type(method_call, bindings, current_function);
        }

        if let Some(function) = self.module_functions.get(&(
            method_call.base_name.clone(),
            method_call.method_name.clone(),
        )) {
            return Ok(function.return_type.clone());
        }

        let base_type = self.infer_expression_type(
            &synthetic_identifier(method_call.base_name.clone()),
            bindings,
            current_function,
        )?;

        match base_type {
            TypeName::Array(element_type, _) => match method_call.method_name.as_str() {
                "len" => Ok(TypeName::U64),
                "push" => Ok(TypeName::Void),
                "pop" => Ok(*element_type),
                other => Err(CodegenError::new(format!(
                    "unknown array method '{}()' in LLVM backend",
                    other
                ))),
            },
            TypeName::Named(struct_name) => self
                .methods
                .get(&(struct_name.clone(), method_call.method_name.clone()))
                .map(|method| method.return_type.clone())
                .ok_or_else(|| {
                    CodegenError::new(format!(
                        "struct '{}' has no method '{}()' in LLVM backend",
                        struct_name, method_call.method_name
                    ))
                }),
            other => Err(CodegenError::new(format!(
                "cannot call method '{}()' on LLVM value of type {:?}",
                method_call.method_name, other
            ))),
        }
    }

    fn infer_io_method_call_type(
        &self,
        method_call: &MethodCallExpression,
    ) -> CodegenResult<TypeName> {
        if !method_call.arguments.is_empty() {
            return Err(CodegenError::new(format!(
                "io.{}() expects 0 arguments in LLVM backend, found {}",
                method_call.method_name,
                method_call.arguments.len()
            )));
        }

        self.io_method_return_type(&method_call.method_name)
    }

    fn infer_json_method_call_type(
        &self,
        method_call: &MethodCallExpression,
    ) -> CodegenResult<TypeName> {
        let (expected_arguments, return_type) = match method_call.method_name.as_str() {
            "parse" => (1, TypeName::Json),
            "stringify" => (1, TypeName::Text),
            "get_text" => (2, TypeName::Text),
            "get_i32" => (2, TypeName::I32),
            "get_i64" => (2, TypeName::I64),
            "get_f64" => (2, TypeName::F64),
            "get_bool" => (2, TypeName::Bool),
            "has" => (2, TypeName::Bool),
            other => {
                return Err(CodegenError::new(format!(
                    "unknown json method '{}()' in LLVM backend",
                    other
                )));
            }
        };

        if method_call.arguments.len() != expected_arguments {
            return Err(CodegenError::new(format!(
                "json.{}() expects {} arguments in LLVM backend, found {}",
                method_call.method_name,
                expected_arguments,
                method_call.arguments.len()
            )));
        }

        Ok(return_type)
    }

    fn infer_schema_method_call_type(
        &self,
        method_call: &MethodCallExpression,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
        current_function: FunctionValue<'ctx>,
    ) -> CodegenResult<TypeName> {
        let return_type = match method_call.method_name.as_str() {
            "validate" => TypeName::Bool,
            "require" => TypeName::Json,
            other => {
                return Err(CodegenError::new(format!(
                    "unknown schema method '{}()' in LLVM backend",
                    other
                )));
            }
        };

        if method_call.arguments.len() != 2 {
            return Err(CodegenError::new(format!(
                "schema.{}() expects 2 arguments in LLVM backend, found {}",
                method_call.method_name,
                method_call.arguments.len()
            )));
        }

        let ExpressionKind::StringLiteral(schema_name) = &method_call.arguments[0].kind else {
            return Err(CodegenError::new(format!(
                "schema.{}() requires a schema name string literal as the first argument",
                method_call.method_name
            )));
        };
        if !self.schemas.contains_key(schema_name) {
            return Err(CodegenError::new(format!(
                "unknown schema name '{}' in LLVM backend",
                schema_name
            )));
        }

        let json_argument_type =
            self.infer_expression_type(&method_call.arguments[1], bindings, current_function)?;
        if json_argument_type != TypeName::Json {
            return Err(CodegenError::new(format!(
                "schema.{}() expected Json as the second argument, found {:?}",
                method_call.method_name, json_argument_type
            )));
        }

        Ok(return_type)
    }

    fn infer_python_method_call_type(
        &self,
        method_call: &MethodCallExpression,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
        current_function: FunctionValue<'ctx>,
    ) -> CodegenResult<TypeName> {
        if method_call.method_name != "call" {
            return Err(CodegenError::new(format!(
                "unknown python method '{}()' in LLVM backend",
                method_call.method_name
            )));
        }

        if method_call.arguments.len() != 2 {
            return Err(CodegenError::new(format!(
                "python.call() expects 2 arguments in LLVM backend, found {}",
                method_call.arguments.len()
            )));
        }

        let path_type =
            self.infer_expression_type(&method_call.arguments[0], bindings, current_function)?;
        if path_type != TypeName::Text {
            return Err(CodegenError::new(format!(
                "python.call() expected Text as the first argument, found {:?}",
                path_type
            )));
        }

        let input_type =
            self.infer_expression_type(&method_call.arguments[1], bindings, current_function)?;
        if input_type != TypeName::Json {
            return Err(CodegenError::new(format!(
                "python.call() expected Json as the second argument, found {:?}",
                input_type
            )));
        }

        Ok(TypeName::Json)
    }

    fn infer_llm_method_call_type(
        &self,
        method_call: &MethodCallExpression,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
        current_function: FunctionValue<'ctx>,
    ) -> CodegenResult<TypeName> {
        if method_call.method_name != "ask" {
            return Err(CodegenError::new(format!(
                "unknown llm method '{}()' in LLVM backend",
                method_call.method_name
            )));
        }

        if method_call.arguments.len() != 1 {
            return Err(CodegenError::new(format!(
                "llm.ask() expects 1 argument in LLVM backend, found {}",
                method_call.arguments.len()
            )));
        }

        let prompt_type =
            self.infer_expression_type(&method_call.arguments[0], bindings, current_function)?;
        if prompt_type != TypeName::Text {
            return Err(CodegenError::new(format!(
                "llm.ask() expected Text as the first argument, found {:?}",
                prompt_type
            )));
        }

        Ok(TypeName::Text)
    }

    fn io_method_return_type(&self, method_name: &str) -> CodegenResult<TypeName> {
        match method_name {
            "read_line" => Ok(TypeName::Text),
            "read_i8" => Ok(TypeName::I8),
            "read_i16" => Ok(TypeName::I16),
            "read_i32" => Ok(TypeName::I32),
            "read_i64" => Ok(TypeName::I64),
            "read_u8" => Ok(TypeName::U8),
            "read_u16" => Ok(TypeName::U16),
            "read_u32" => Ok(TypeName::U32),
            "read_u64" => Ok(TypeName::U64),
            "read_f32" => Ok(TypeName::F32),
            "read_f64" => Ok(TypeName::F64),
            "read_bool" => Ok(TypeName::Bool),
            other => Err(CodegenError::new(format!(
                "unknown io method '{}()' in LLVM backend",
                other
            ))),
        }
    }

    fn codegen_method_call_value(
        &self,
        expected_type: &TypeName,
        method_call: &MethodCallExpression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        if method_call.base_name == "io" {
            return self.codegen_io_method_call_value(expected_type, method_call, current_function);
        }
        if method_call.base_name == "json" {
            return self.codegen_json_method_call_value(
                expected_type,
                method_call,
                current_function,
                bindings,
            );
        }
        if method_call.base_name == "schema" {
            return self.codegen_schema_method_call_value(
                expected_type,
                method_call,
                current_function,
                bindings,
            );
        }
        if method_call.base_name == "python" {
            return self.codegen_python_method_call_value(
                expected_type,
                method_call,
                current_function,
                bindings,
            );
        }
        if method_call.base_name == "llm" {
            return self.codegen_llm_method_call_value(
                expected_type,
                method_call,
                current_function,
                bindings,
            );
        }

        if let Some(function) = self.module_functions.get(&(
            method_call.base_name.clone(),
            method_call.method_name.clone(),
        )) {
            if function.parameters.len() != method_call.arguments.len() {
                return Err(CodegenError::new(format!(
                    "call '{}.{}' expected {} arguments, found {}",
                    method_call.base_name,
                    method_call.method_name,
                    function.parameters.len(),
                    method_call.arguments.len()
                )));
            }

            let callee = self.module.get_function(&function.name).ok_or_else(|| {
                CodegenError::new(format!(
                    "missing LLVM declaration for module function '{}.{}'",
                    method_call.base_name, method_call.method_name
                ))
            })?;

            let mut arguments = Vec::with_capacity(function.parameters.len());
            for (parameter, argument) in
                function.parameters.iter().zip(method_call.arguments.iter())
            {
                arguments.push(
                    self.codegen_value_expression(
                        &parameter.type_name,
                        argument,
                        current_function,
                        bindings,
                    )?
                    .into(),
                );
            }

            let call_site = self
                .builder
                .build_call(callee, &arguments, "modulecall")
                .map_err(|error| {
                    CodegenError::new(format!(
                        "failed to build call to module function '{}.{}': {error}",
                        method_call.base_name, method_call.method_name
                    ))
                })?;

            return call_site.try_as_basic_value().basic().ok_or_else(|| {
                CodegenError::new(format!(
                    "module function '{}.{}' did not produce a return value",
                    method_call.base_name, method_call.method_name
                ))
            });
        }

        let base_type = self.infer_expression_type(
            &synthetic_identifier(method_call.base_name.clone()),
            bindings,
            current_function,
        )?;

        match base_type {
            TypeName::Array(_, _) => match method_call.method_name.as_str() {
                "len" => {
                    let value = self.codegen_array_len_expression(method_call, bindings)?;
                    if !expected_type.is_integer() {
                        return Err(CodegenError::new(format!(
                            "array len() expected integer type in LLVM backend, found {:?}",
                            expected_type
                        )));
                    }

                    self.cast_primitive_value(&TypeName::U64, expected_type, value.into())
                }
                "push" => Err(CodegenError::new(
                    "LLVM backend does not support Array.push() yet; use the interpreter path for dynamic array mutation",
                )),
                "pop" => Err(CodegenError::new(
                    "LLVM backend does not support Array.pop() yet; use the interpreter path for dynamic array mutation",
                )),
                other => Err(CodegenError::new(format!(
                    "unknown array method '{}()' in LLVM backend",
                    other
                ))),
            },
            TypeName::Named(struct_name) => self.codegen_struct_method_call_value(
                &struct_name,
                expected_type,
                method_call,
                current_function,
                bindings,
            ),
            other => Err(CodegenError::new(format!(
                "cannot lower method '{}()' on LLVM value of type {:?}",
                method_call.method_name, other
            ))),
        }
    }

    fn codegen_array_len_expression(
        &self,
        method_call: &MethodCallExpression,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<IntValue<'ctx>> {
        if !method_call.arguments.is_empty() {
            return Err(CodegenError::new(format!(
                "array method 'len()' expects 0 arguments, found {}",
                method_call.arguments.len()
            )));
        }

        let length = match bindings.get(&method_call.base_name).cloned() {
            Some(FunctionBinding::Local {
                type_name: TypeName::Array(_, Some(length)),
                ..
            })
            | Some(FunctionBinding::Parameter {
                type_name: TypeName::Array(_, Some(length)),
                ..
            }) => length,
            Some(FunctionBinding::Local { type_name, .. })
            | Some(FunctionBinding::Parameter { type_name, .. }) => {
                return Err(CodegenError::new(format!(
                    "array '{}' has LLVM type {:?}; len() requires a fixed array length",
                    method_call.base_name, type_name
                )));
            }
            None => {
                return Err(CodegenError::new(format!(
                    "unknown identifier '{}' during LLVM array len()",
                    method_call.base_name
                )));
            }
        };

        Ok(self.context.i64_type().const_int(length as u64, false))
    }

    fn codegen_io_method_call_value(
        &self,
        expected_type: &TypeName,
        method_call: &MethodCallExpression,
        current_function: FunctionValue<'ctx>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        let method_type = self.io_method_return_type(&method_call.method_name)?;
        if &method_type != expected_type {
            return Err(CodegenError::new(format!(
                "io.{}() has type {:?}, expected {:?}",
                method_call.method_name, method_type, expected_type
            )));
        }

        match method_call.method_name.as_str() {
            "read_line" => self.codegen_read_line_value(),
            "read_i8" | "read_i16" | "read_i32" | "read_i64" | "read_u8" | "read_u16"
            | "read_u32" | "read_u64" | "read_f32" | "read_f64" => {
                self.codegen_scanf_read_value(expected_type, current_function)
            }
            "read_bool" => self.codegen_bool_read_value(current_function),
            other => Err(CodegenError::new(format!(
                "unknown io method '{}()' in LLVM backend",
                other
            ))),
        }
    }

    fn codegen_json_method_call_value(
        &self,
        expected_type: &TypeName,
        method_call: &MethodCallExpression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        let method_type = self.infer_json_method_call_type(method_call)?;
        if &method_type != expected_type {
            return Err(CodegenError::new(format!(
                "json.{}() has type {:?}, expected {:?}",
                method_call.method_name, method_type, expected_type
            )));
        }

        let (function_name, arguments): (&str, Vec<inkwell::values::BasicMetadataValueEnum<'ctx>>) =
            match method_call.method_name.as_str() {
                "parse" => (
                    "loz_json_parse",
                    vec![
                        self.codegen_text_expression(
                            &method_call.arguments[0],
                            current_function,
                            bindings,
                        )?
                        .into(),
                    ],
                ),
                "stringify" => (
                    "loz_json_stringify",
                    vec![
                        self.codegen_json_expression(
                            &method_call.arguments[0],
                            current_function,
                            bindings,
                        )?
                        .into(),
                    ],
                ),
                "has" | "get_text" | "get_i32" | "get_i64" | "get_f64" | "get_bool" => {
                    let json = self.codegen_json_expression(
                        &method_call.arguments[0],
                        current_function,
                        bindings,
                    )?;
                    let key = self.codegen_text_expression(
                        &method_call.arguments[1],
                        current_function,
                        bindings,
                    )?;
                    let function_name = match method_call.method_name.as_str() {
                        "has" => "loz_json_has",
                        "get_text" => "loz_json_get_text",
                        "get_i32" => "loz_json_get_i32",
                        "get_i64" => "loz_json_get_i64",
                        "get_f64" => "loz_json_get_f64",
                        "get_bool" => "loz_json_get_bool",
                        _ => unreachable!(),
                    };
                    (function_name, vec![json.into(), key.into()])
                }
                other => {
                    return Err(CodegenError::new(format!(
                        "unknown json method '{}()' in LLVM backend",
                        other
                    )));
                }
            };

        let function = self.module.get_function(function_name).ok_or_else(|| {
            CodegenError::new(format!(
                "{function_name} declaration is missing from LLVM module"
            ))
        })?;

        self.builder
            .build_call(function, &arguments, "json_call")
            .map_err(|error| {
                CodegenError::new(format!("failed to build {function_name} call: {error}"))
            })?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| {
                CodegenError::new(format!("{function_name} did not produce a return value"))
            })
    }

    fn codegen_schema_method_call_value(
        &self,
        expected_type: &TypeName,
        method_call: &MethodCallExpression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        let method_type =
            self.infer_schema_method_call_type(method_call, bindings, current_function)?;
        if &method_type != expected_type {
            return Err(CodegenError::new(format!(
                "schema.{}() has type {:?}, expected {:?}",
                method_call.method_name, method_type, expected_type
            )));
        }

        let ExpressionKind::StringLiteral(schema_name) = &method_call.arguments[0].kind else {
            return Err(CodegenError::new(format!(
                "schema.{}() requires a schema name string literal as the first argument",
                method_call.method_name
            )));
        };

        let schema_descriptor = self.codegen_schema_descriptor_pointer(schema_name)?;
        let json_value =
            self.codegen_json_expression(&method_call.arguments[1], current_function, bindings)?;
        let function_name = match method_call.method_name.as_str() {
            "validate" => "loz_schema_validate",
            "require" => "loz_schema_require",
            other => {
                return Err(CodegenError::new(format!(
                    "unknown schema method '{}()' in LLVM backend",
                    other
                )));
            }
        };
        let function = self.module.get_function(function_name).ok_or_else(|| {
            CodegenError::new(format!(
                "{function_name} declaration is missing from LLVM module"
            ))
        })?;

        self.builder
            .build_call(
                function,
                &[schema_descriptor.into(), json_value.into()],
                "schema_call",
            )
            .map_err(|error| {
                CodegenError::new(format!("failed to build {function_name} call: {error}"))
            })?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| {
                CodegenError::new(format!("{function_name} did not produce a return value"))
            })
    }

    fn codegen_python_method_call_value(
        &self,
        expected_type: &TypeName,
        method_call: &MethodCallExpression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        let method_type =
            self.infer_python_method_call_type(method_call, bindings, current_function)?;
        if &method_type != expected_type {
            return Err(CodegenError::new(format!(
                "python.{}() has type {:?}, expected {:?}",
                method_call.method_name, method_type, expected_type
            )));
        }

        let function_path =
            self.codegen_text_expression(&method_call.arguments[0], current_function, bindings)?;
        let input_json =
            self.codegen_json_expression(&method_call.arguments[1], current_function, bindings)?;
        let function = self.module.get_function("loz_python_call").ok_or_else(|| {
            CodegenError::new("loz_python_call declaration is missing from LLVM module")
        })?;

        self.builder
            .build_call(
                function,
                &[function_path.into(), input_json.into()],
                "python_call",
            )
            .map_err(|error| {
                CodegenError::new(format!("failed to build loz_python_call call: {error}"))
            })?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| CodegenError::new("loz_python_call did not produce a return value"))
    }

    fn codegen_llm_method_call_value(
        &self,
        expected_type: &TypeName,
        method_call: &MethodCallExpression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        let method_type =
            self.infer_llm_method_call_type(method_call, bindings, current_function)?;
        if &method_type != expected_type {
            return Err(CodegenError::new(format!(
                "llm.{}() has type {:?}, expected {:?}",
                method_call.method_name, method_type, expected_type
            )));
        }

        let prompt =
            self.codegen_text_expression(&method_call.arguments[0], current_function, bindings)?;
        let function = self.module.get_function("loz_llm_ask").ok_or_else(|| {
            CodegenError::new("loz_llm_ask declaration is missing from LLVM module")
        })?;

        self.builder
            .build_call(function, &[prompt.into()], "llm_call")
            .map_err(|error| {
                CodegenError::new(format!("failed to build loz_llm_ask call: {error}"))
            })?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| CodegenError::new("loz_llm_ask did not produce a return value"))
    }

    fn codegen_scanf_read_value(
        &self,
        expected_type: &TypeName,
        current_function: FunctionValue<'ctx>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        let llvm_type = self.llvm_basic_type(expected_type)?;
        let pointer = self.create_entry_alloca(current_function, "io_read_value", llvm_type)?;
        self.builder
            .build_store(pointer, self.zero_value(expected_type)?)
            .map_err(|error| {
                CodegenError::new(format!("failed to initialize io read storage: {error}"))
            })?;

        let scanf = self
            .module
            .get_function("scanf")
            .ok_or_else(|| CodegenError::new("scanf declaration is missing from LLVM module"))?;
        let format_string = self
            .builder
            .build_global_string_ptr(self.io_scanf_format(expected_type)?, "scanf_fmt")
            .map_err(|error| {
                CodegenError::new(format!("failed to build scanf format string: {error}"))
            })?;

        self.builder
            .build_call(
                scanf,
                &[format_string.as_pointer_value().into(), pointer.into()],
                "scanf_read",
            )
            .map_err(|error| CodegenError::new(format!("failed to build scanf call: {error}")))?;

        self.builder
            .build_load(llvm_type, pointer, "scanf_loaded")
            .map_err(|error| CodegenError::new(format!("failed to load scanf value: {error}")))
    }

    fn codegen_bool_read_value(
        &self,
        current_function: FunctionValue<'ctx>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        let buffer_type = self.context.i8_type().array_type(16);
        let buffer_pointer =
            self.create_entry_alloca(current_function, "io_read_bool_buffer", buffer_type.into())?;
        self.builder
            .build_store(buffer_pointer, buffer_type.const_zero())
            .map_err(|error| {
                CodegenError::new(format!("failed to initialize bool read buffer: {error}"))
            })?;

        let zero = self.context.i32_type().const_zero();
        let buffer_start = unsafe {
            self.builder.build_in_bounds_gep(
                buffer_type,
                buffer_pointer,
                &[zero, zero],
                "io_read_bool_ptr",
            )
        }
        .map_err(|error| {
            CodegenError::new(format!("failed to address bool read buffer: {error}"))
        })?;

        let scanf = self
            .module
            .get_function("scanf")
            .ok_or_else(|| CodegenError::new("scanf declaration is missing from LLVM module"))?;
        let format_string = self
            .builder
            .build_global_string_ptr("%15s", "scanf_fmt_bool")
            .map_err(|error| {
                CodegenError::new(format!("failed to build bool scanf format string: {error}"))
            })?;

        self.builder
            .build_call(
                scanf,
                &[format_string.as_pointer_value().into(), buffer_start.into()],
                "scanf_bool",
            )
            .map_err(|error| {
                CodegenError::new(format!("failed to build bool scanf call: {error}"))
            })?;

        let is_true = self.codegen_strcmp_equals(buffer_start, "true", "bool_true_cmp")?;
        let is_one = self.codegen_strcmp_equals(buffer_start, "1", "bool_one_cmp")?;
        let is_yes = self.codegen_strcmp_equals(buffer_start, "yes", "bool_yes_cmp")?;
        let true_or_one = self
            .builder
            .build_or(is_true, is_one, "bool_true_or_one")
            .map_err(|error| {
                CodegenError::new(format!("failed to combine bool true inputs: {error}"))
            })?;
        let result = self
            .builder
            .build_or(true_or_one, is_yes, "bool_true_result")
            .map_err(|error| {
                CodegenError::new(format!("failed to combine bool yes input: {error}"))
            })?;

        Ok(result.into())
    }

    // Alpha Text runtime: a fixed global stdin buffer is reused on every read_line() call.
    fn codegen_read_line_value(&self) -> CodegenResult<BasicValueEnum<'ctx>> {
        let buffer = self
            .module
            .get_global("loz_stdin_buffer")
            .ok_or_else(|| CodegenError::new("loz_stdin_buffer is missing from LLVM module"))?;
        let buffer_type = self.context.i8_type().array_type(1024);
        let zero = self.context.i32_type().const_zero();
        let buffer_start = unsafe {
            self.builder.build_in_bounds_gep(
                buffer_type,
                buffer.as_pointer_value(),
                &[zero, zero],
                "io_read_line_ptr",
            )
        }
        .map_err(|error| {
            CodegenError::new(format!("failed to address stdin Text buffer: {error}"))
        })?;

        let fgets = self
            .module
            .get_function("fgets")
            .ok_or_else(|| CodegenError::new("fgets declaration is missing from LLVM module"))?;
        let stdin = self.module.get_global("stdin").ok_or_else(|| {
            CodegenError::new("stdin global declaration is missing from LLVM module")
        })?;
        let stdin_value = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                stdin.as_pointer_value(),
                "stdin_value",
            )
            .map_err(|error| CodegenError::new(format!("failed to load stdin pointer: {error}")))?;

        self.builder
            .build_call(
                fgets,
                &[
                    buffer_start.into(),
                    self.context.i32_type().const_int(1024, false).into(),
                    stdin_value.into(),
                ],
                "fgets_read_line",
            )
            .map_err(|error| CodegenError::new(format!("failed to build fgets call: {error}")))?;

        let strcspn = self
            .module
            .get_function("strcspn")
            .ok_or_else(|| CodegenError::new("strcspn declaration is missing from LLVM module"))?;
        let newline = self
            .builder
            .build_global_string_ptr("\n", "stdin_newline")
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to build newline literal for read_line: {error}"
                ))
            })?;
        let newline_index = self
            .builder
            .build_call(
                strcspn,
                &[buffer_start.into(), newline.as_pointer_value().into()],
                "read_line_newline_idx",
            )
            .map_err(|error| CodegenError::new(format!("failed to build strcspn call: {error}")))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| CodegenError::new("strcspn did not produce a return value"))?
            .into_int_value();
        let newline_pointer = unsafe {
            self.builder.build_in_bounds_gep(
                self.context.i8_type(),
                buffer_start,
                &[newline_index],
                "read_line_newline_ptr",
            )
        }
        .map_err(|error| {
            CodegenError::new(format!("failed to address read_line newline slot: {error}"))
        })?;
        self.builder
            .build_store(newline_pointer, self.context.i8_type().const_zero())
            .map_err(|error| {
                CodegenError::new(format!("failed to strip read_line newline: {error}"))
            })?;

        Ok(buffer_start.into())
    }

    fn codegen_strcmp_equals(
        &self,
        left: PointerValue<'ctx>,
        right_literal: &str,
        name: &str,
    ) -> CodegenResult<IntValue<'ctx>> {
        let strcmp = self
            .module
            .get_function("strcmp")
            .ok_or_else(|| CodegenError::new("strcmp declaration is missing from LLVM module"))?;
        let right = self
            .builder
            .build_global_string_ptr(right_literal, name)
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to build strcmp literal '{right_literal}': {error}"
                ))
            })?;
        let comparison = self
            .builder
            .build_call(
                strcmp,
                &[left.into(), right.as_pointer_value().into()],
                &format!("{name}_call"),
            )
            .map_err(|error| CodegenError::new(format!("failed to build strcmp call: {error}")))?;
        let value = comparison
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| CodegenError::new("strcmp did not produce a return value"))?
            .into_int_value();

        self.builder
            .build_int_compare(
                IntPredicate::EQ,
                value,
                self.context.i32_type().const_zero(),
                &format!("{name}_eq"),
            )
            .map_err(|error| CodegenError::new(format!("failed to compare strcmp result: {error}")))
    }

    fn io_scanf_format(&self, type_name: &TypeName) -> CodegenResult<&'static str> {
        match type_name {
            TypeName::I8 => Ok("%hhd"),
            TypeName::I16 => Ok("%hd"),
            TypeName::I32 => Ok("%d"),
            TypeName::I64 => Ok("%lld"),
            TypeName::U8 => Ok("%hhu"),
            TypeName::U16 => Ok("%hu"),
            TypeName::U32 => Ok("%u"),
            TypeName::U64 => Ok("%llu"),
            TypeName::F32 => Ok("%f"),
            TypeName::F64 => Ok("%lf"),
            other => Err(CodegenError::new(format!(
                "type {:?} is not supported by io scanf lowering",
                other
            ))),
        }
    }

    fn zero_value(&self, type_name: &TypeName) -> CodegenResult<BasicValueEnum<'ctx>> {
        match type_name {
            type_name if type_name.is_integer() => Ok(self
                .llvm_basic_type(type_name)?
                .into_int_type()
                .const_zero()
                .into()),
            type_name if type_name.is_float() => Ok(self
                .llvm_basic_type(type_name)?
                .into_float_type()
                .const_zero()
                .into()),
            TypeName::Bool => Ok(self.context.bool_type().const_zero().into()),
            other => Err(CodegenError::new(format!(
                "cannot create zero value for {:?} in LLVM backend",
                other
            ))),
        }
    }

    fn codegen_struct_method_call_value(
        &self,
        struct_name: &str,
        _expected_type: &TypeName,
        method_call: &MethodCallExpression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        let method = self
            .methods
            .get(&(struct_name.to_string(), method_call.method_name.clone()))
            .ok_or_else(|| {
                CodegenError::new(format!(
                    "struct '{}' has no method '{}()' in LLVM backend",
                    struct_name, method_call.method_name
                ))
            })?;
        let callee_name = self.method_symbol_name(struct_name, &method_call.method_name);
        let callee = self.module.get_function(&callee_name).ok_or_else(|| {
            CodegenError::new(format!(
                "missing LLVM declaration for method '{}.{}()'",
                struct_name, method_call.method_name
            ))
        })?;

        let mut arguments = Vec::with_capacity(method_call.arguments.len() + 1);
        let receiver_type = TypeName::Named(struct_name.to_string());
        let receiver = self.codegen_value_expression(
            &receiver_type,
            &synthetic_identifier(method_call.base_name.clone()),
            current_function,
            bindings,
        )?;
        arguments.push(receiver.into());

        for (parameter, argument) in method
            .parameters
            .iter()
            .skip(1)
            .zip(method_call.arguments.iter())
        {
            arguments.push(
                self.codegen_value_expression(
                    &parameter.type_name,
                    argument,
                    current_function,
                    bindings,
                )?
                .into(),
            );
        }

        let call_site = self
            .builder
            .build_call(callee, &arguments, "methodcall")
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to build call to method '{}.{}()': {error}",
                    struct_name, method_call.method_name
                ))
            })?;

        call_site.try_as_basic_value().basic().ok_or_else(|| {
            CodegenError::new(format!(
                "method '{}.{}()' did not produce a return value",
                struct_name, method_call.method_name
            ))
        })
    }

    fn codegen_reference_expression(
        &self,
        expected_type: &TypeName,
        expression: &Expression,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<PointerValue<'ctx>> {
        let TypeName::Reference { .. } = expected_type else {
            return Err(CodegenError::new(format!(
                "expected reference type in LLVM backend, found {:?}",
                expected_type
            )));
        };

        match &expression.kind {
            ExpressionKind::Reference(reference_expression) => {
                if let Some(binding) = bindings.get(&reference_expression.target_name).cloned() {
                    match binding {
                        FunctionBinding::Local { pointer, .. } => Ok(pointer),
                        FunctionBinding::Parameter { .. } => Err(CodegenError::new(format!(
                            "references to parameters are not supported in LLVM backend for '{}'",
                            reference_expression.target_name
                        ))),
                    }
                } else if let Some(global) = self.globals.get(&reference_expression.target_name) {
                    Ok(global.as_pointer_value())
                } else {
                    Err(CodegenError::new(format!(
                        "unknown identifier '{}' during LLVM reference creation",
                        reference_expression.target_name
                    )))
                }
            }
            ExpressionKind::Identifier(name) => {
                let binding = bindings.get(name).cloned().ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown reference identifier '{}' during LLVM codegen",
                        name
                    ))
                })?;

                match binding {
                    FunctionBinding::Local {
                        pointer, type_name, ..
                    } if type_name == *expected_type => self
                        .builder
                        .build_load(
                            self.context.ptr_type(AddressSpace::default()),
                            pointer,
                            &format!("load_{name}_ref"),
                        )
                        .map(|value| value.into_pointer_value())
                        .map_err(|error| {
                            CodegenError::new(format!(
                                "failed to load reference local '{name}': {error}"
                            ))
                        }),
                    FunctionBinding::Local { type_name, .. } => Err(CodegenError::new(format!(
                        "local '{}' has type {:?}, expected reference type {:?}",
                        name, type_name, expected_type
                    ))),
                    FunctionBinding::Parameter { type_name, .. } => {
                        Err(CodegenError::new(format!(
                            "parameter '{}' has type {:?}, expected reference type {:?}",
                            name, type_name, expected_type
                        )))
                    }
                }
            }
            _ => Err(CodegenError::new(format!(
                "LLVM backend only supports direct ref expressions and reference identifiers, found {:?}",
                expression
            ))),
        }
    }

    fn codegen_field_access_expression(
        &self,
        base_name: &str,
        field_name: &str,
        _current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<(TypeName, BasicValueEnum<'ctx>)> {
        let binding = bindings.get(base_name).cloned().ok_or_else(|| {
            CodegenError::new(format!(
                "unknown identifier '{}' during LLVM field access",
                base_name
            ))
        })?;

        let (struct_name, struct_value) = match binding {
            FunctionBinding::Local {
                pointer,
                type_name: TypeName::Named(struct_name),
                ..
            } => {
                let struct_type = self.llvm_struct_type(&TypeName::Named(struct_name.clone()))?;
                let struct_value = self
                    .builder
                    .build_load(struct_type, pointer, &format!("load_{base_name}"))
                    .map_err(|error| {
                        CodegenError::new(format!(
                            "failed to load struct local '{}' for field access: {error}",
                            base_name
                        ))
                    })?
                    .into_struct_value();
                (struct_name, struct_value)
            }
            FunctionBinding::Parameter {
                value,
                type_name: TypeName::Named(struct_name),
            } => (struct_name, value.into_struct_value()),
            _ => {
                return Err(CodegenError::new(format!(
                    "field access is only supported on struct values, not '{}'",
                    base_name
                )));
            }
        };

        let struct_declaration = self.structs.get(&struct_name).ok_or_else(|| {
            CodegenError::new(format!(
                "unknown struct '{}' during LLVM field access",
                struct_name
            ))
        })?;

        let (field_index, field) = struct_declaration
            .fields
            .iter()
            .enumerate()
            .find(|(_, field)| field.name == field_name)
            .ok_or_else(|| {
                CodegenError::new(format!(
                    "struct '{}' has no field '{}' during LLVM field access",
                    struct_name, field_name
                ))
            })?;

        let field_value = self
            .builder
            .build_extract_value(
                struct_value,
                field_index as u32,
                &format!("extract_{base_name}_{field_name}"),
            )
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to extract field '{}.{}': {error}",
                    base_name, field_name
                ))
            })?;

        Ok((field.type_name.clone(), field_value))
    }

    fn codegen_struct_expression(
        &self,
        expected_type: &TypeName,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<StructValue<'ctx>> {
        let struct_type = self.llvm_struct_type(expected_type)?;
        let TypeName::Named(struct_name) = expected_type else {
            return Err(CodegenError::new(format!(
                "expected struct type for LLVM construction, found {:?}",
                expected_type
            )));
        };

        match &expression.kind {
            ExpressionKind::Identifier(name) => {
                let binding = bindings.get(name).cloned().ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown local identifier '{}' during LLVM struct load",
                        name
                    ))
                })?;

                match binding {
                    FunctionBinding::Local {
                        pointer, type_name, ..
                    } if type_name == *expected_type => self
                        .builder
                        .build_load(struct_type, pointer, &format!("load_{name}"))
                        .map_err(|error| {
                            CodegenError::new(format!(
                                "failed to load struct local '{name}': {error}"
                            ))
                        })
                        .map(|value| value.into_struct_value()),
                    FunctionBinding::Local { type_name, .. } => Err(CodegenError::new(format!(
                        "local '{}' has type {:?}, expected struct type {:?}",
                        name, type_name, expected_type
                    ))),
                    FunctionBinding::Parameter { value, type_name }
                        if type_name == *expected_type =>
                    {
                        Ok(value.into_struct_value())
                    }
                    FunctionBinding::Parameter { type_name, .. } => {
                        Err(CodegenError::new(format!(
                            "parameter '{}' has type {:?}, expected struct type {:?}",
                            name, type_name, expected_type
                        )))
                    }
                }
            }
            ExpressionKind::Call(call) if call.callee == *struct_name => {
                let struct_declaration = self.structs.get(struct_name).ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown struct constructor '{}' during LLVM codegen",
                        struct_name
                    ))
                })?;

                if struct_declaration.fields.len() != call.arguments.len() {
                    return Err(CodegenError::new(format!(
                        "struct constructor '{}' expected {} arguments, found {}",
                        struct_name,
                        struct_declaration.fields.len(),
                        call.arguments.len()
                    )));
                }

                let mut value = struct_type.get_undef();
                for (index, (field, argument)) in struct_declaration
                    .fields
                    .iter()
                    .zip(call.arguments.iter())
                    .enumerate()
                {
                    let field_value = self.codegen_value_expression(
                        &field.type_name,
                        argument,
                        current_function,
                        bindings,
                    )?;
                    value = self
                        .builder
                        .build_insert_value(
                            value,
                            field_value,
                            index as u32,
                            &format!("insert_{struct_name}_{index}"),
                        )
                        .map_err(|error| {
                            CodegenError::new(format!(
                                "failed to build struct constructor '{}': {error}",
                                struct_name
                            ))
                        })?
                        .into_struct_value();
                }

                Ok(value)
            }
            ExpressionKind::Call(call) => self
                .codegen_call_expression(call, current_function, bindings)
                .map(|value| value.into_struct_value()),
            _ => Err(CodegenError::new(format!(
                "LLVM backend only supports direct '{}' constructor calls and struct identifiers for struct values",
                struct_name
            ))),
        }
    }

    fn codegen_array_expression(
        &self,
        expected_type: &TypeName,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<ArrayValue<'ctx>> {
        let array_type = self.llvm_array_type(expected_type)?;
        let TypeName::Array(element_type, Some(expected_length)) = expected_type else {
            return Err(CodegenError::new(format!(
                "expected fixed array type for LLVM array construction, found {:?}",
                expected_type
            )));
        };

        match &expression.kind {
            ExpressionKind::Identifier(name) => {
                let binding = bindings.get(name).cloned().ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown local identifier '{}' during LLVM array load",
                        name
                    ))
                })?;

                match binding {
                    FunctionBinding::Local {
                        pointer, type_name, ..
                    } if type_name == *expected_type => self
                        .builder
                        .build_load(array_type, pointer, &format!("load_{name}"))
                        .map_err(|error| {
                            CodegenError::new(format!(
                                "failed to load array local '{name}': {error}"
                            ))
                        })
                        .map(|value| value.into_array_value()),
                    FunctionBinding::Local { type_name, .. } => Err(CodegenError::new(format!(
                        "local '{}' has type {:?}, expected array type {:?}",
                        name, type_name, expected_type
                    ))),
                    FunctionBinding::Parameter { value, type_name }
                        if type_name == *expected_type =>
                    {
                        Ok(value.into_array_value())
                    }
                    FunctionBinding::Parameter { type_name, .. } => {
                        Err(CodegenError::new(format!(
                            "parameter '{}' has type {:?}, expected array type {:?}",
                            name, type_name, expected_type
                        )))
                    }
                }
            }
            ExpressionKind::ArrayLiteral(array_literal) => {
                if array_literal.elements.len() != *expected_length {
                    return Err(CodegenError::new(format!(
                        "array literal expected {} elements, found {}",
                        expected_length,
                        array_literal.elements.len()
                    )));
                }

                let mut value = array_type.get_undef();
                for (index, element) in array_literal.elements.iter().enumerate() {
                    let element_value = self.codegen_value_expression(
                        element_type,
                        element,
                        current_function,
                        bindings,
                    )?;
                    value = self
                        .builder
                        .build_insert_value(
                            value,
                            element_value,
                            index as u32,
                            &format!("insert_array_{index}"),
                        )
                        .map_err(|error| {
                            CodegenError::new(format!(
                                "failed to build array literal element {}: {error}",
                                index
                            ))
                        })?
                        .into_array_value();
                }

                Ok(value)
            }
            _ => Err(CodegenError::new(format!(
                "LLVM backend only supports fixed array literals and identifiers for array values, found {:?}",
                expression
            ))),
        }
    }

    fn codegen_primitive_expression(
        &self,
        expected_type: &TypeName,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        match expected_type {
            type_name if type_name.is_integer() => self
                .codegen_integer_expression(type_name, expression, current_function, bindings)
                .map(Into::into),
            type_name if type_name.is_float() => self
                .codegen_float_expression(type_name, expression, current_function, bindings)
                .map(Into::into),
            TypeName::Bool => self
                .codegen_bool_expression(expression, current_function, bindings)
                .map(Into::into),
            TypeName::Text => self
                .codegen_text_expression(expression, current_function, bindings)
                .map(Into::into),
            _ => Err(CodegenError::new(format!(
                "primitive LLVM lowering does not support type {:?}",
                expected_type
            ))),
        }
    }

    fn codegen_json_expression(
        &self,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<PointerValue<'ctx>> {
        match &expression.kind {
            ExpressionKind::Identifier(name) => self
                .load_identifier_value(name, &TypeName::Json, bindings)
                .map(|value| value.into_pointer_value()),
            ExpressionKind::MethodCall(method_call) => {
                let method_type =
                    self.infer_method_call_type(method_call, bindings, current_function)?;
                if method_type != TypeName::Json {
                    return Err(CodegenError::new(format!(
                        "method '{}.{}()' has type {:?}, expected Json",
                        method_call.base_name, method_call.method_name, method_type
                    )));
                }

                self.codegen_method_call_value(
                    &TypeName::Json,
                    method_call,
                    current_function,
                    bindings,
                )
                .map(|value| value.into_pointer_value())
            }
            ExpressionKind::Dereference(dereference) => {
                let (dereference_type, dereference_value, _) =
                    self.codegen_dereference_expression(dereference, current_function, bindings)?;
                if dereference_type != TypeName::Json {
                    return Err(CodegenError::new(format!(
                        "dereference has type {:?}, expected Json",
                        dereference_type
                    )));
                }

                Ok(dereference_value.into_pointer_value())
            }
            ExpressionKind::Call(call) => {
                if call.callee == "print" {
                    return Err(CodegenError::new(
                        "print() does not produce a Json value in LLVM backend",
                    ));
                }

                self.codegen_call_expression(call, current_function, bindings)
                    .map(|value| value.into_pointer_value())
            }
            _ => Err(CodegenError::new(format!(
                "LLVM backend cannot lower {:?} as Json",
                expression
            ))),
        }
    }

    fn codegen_integer_expression(
        &self,
        expected_type: &TypeName,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<IntValue<'ctx>> {
        let int_type = self.llvm_basic_type(expected_type)?.into_int_type();

        match &expression.kind {
            ExpressionKind::IntegerLiteral(value) => Ok(int_type.const_int(*value as u64, true)),
            ExpressionKind::MethodCall(method_call) => {
                let method_type =
                    self.infer_method_call_type(method_call, bindings, current_function)?;
                if &method_type != expected_type {
                    return Err(CodegenError::new(format!(
                        "method '{}.{}()' has type {:?}, expected {:?}",
                        method_call.base_name, method_call.method_name, method_type, expected_type
                    )));
                }

                self.codegen_method_call_value(
                    expected_type,
                    method_call,
                    current_function,
                    bindings,
                )
                .map(|value| value.into_int_value())
            }
            ExpressionKind::Dereference(dereference) => {
                let (dereference_type, dereference_value, _) =
                    self.codegen_dereference_expression(dereference, current_function, bindings)?;
                if &dereference_type != expected_type {
                    return Err(CodegenError::new(format!(
                        "dereference has type {:?}, expected {:?}",
                        dereference_type, expected_type
                    )));
                }

                Ok(dereference_value.into_int_value())
            }
            ExpressionKind::Cast(cast_expression) => self
                .codegen_cast_expression(
                    &cast_expression.target_type,
                    &cast_expression.value,
                    current_function,
                    bindings,
                )
                .map(|value| value.into_int_value()),
            ExpressionKind::Identifier(name) => self
                .load_identifier_value(name, expected_type, bindings)
                .map(|value| value.into_int_value()),
            ExpressionKind::FieldAccess(field_access) => {
                let (field_type, field_value) = self.codegen_field_access_expression(
                    &field_access.base_name,
                    &field_access.field_name,
                    current_function,
                    bindings,
                )?;
                if &field_type != expected_type {
                    return Err(CodegenError::new(format!(
                        "field '{}.{}' has type {:?}, expected {:?}",
                        field_access.base_name, field_access.field_name, field_type, expected_type
                    )));
                }

                Ok(field_value.into_int_value())
            }
            ExpressionKind::IndexAccess(index_access) => {
                let (element_type, element_value) = self.codegen_index_access_expression(
                    &index_access.base_name,
                    &index_access.index,
                    current_function,
                    bindings,
                )?;
                if &element_type != expected_type {
                    return Err(CodegenError::new(format!(
                        "array element '{}[...]' has type {:?}, expected {:?}",
                        index_access.base_name, element_type, expected_type
                    )));
                }

                Ok(element_value.into_int_value())
            }
            ExpressionKind::Call(call) => {
                if call.callee == "print" {
                    return Err(CodegenError::new(
                        "print() does not produce an integer value in LLVM backend",
                    ));
                }

                self.codegen_call_expression(call, current_function, bindings)
                    .map(|value| value.into_int_value())
            }
            ExpressionKind::Binary(binary) => {
                let left = self.codegen_integer_expression(
                    expected_type,
                    &binary.left,
                    current_function,
                    bindings,
                )?;
                let right = self.codegen_integer_expression(
                    expected_type,
                    &binary.right,
                    current_function,
                    bindings,
                )?;

                match binary.operator {
                    loz_ast::BinaryOperator::Add => {
                        self.builder.build_int_add(left, right, "addtmp")
                    }
                    loz_ast::BinaryOperator::Subtract => {
                        self.builder.build_int_sub(left, right, "subtmp")
                    }
                    loz_ast::BinaryOperator::Multiply => {
                        self.builder.build_int_mul(left, right, "multmp")
                    }
                    loz_ast::BinaryOperator::Divide if expected_type.is_signed_integer() => {
                        self.builder.build_int_signed_div(left, right, "divtmp")
                    }
                    loz_ast::BinaryOperator::Divide => {
                        self.builder.build_int_unsigned_div(left, right, "divtmp")
                    }
                    _ => {
                        return Err(CodegenError::new(
                            "comparison expressions are not valid integer values in LLVM backend",
                        ));
                    }
                }
                .map_err(|error| {
                    CodegenError::new(format!(
                        "failed to build integer expression for {:?}: {error}",
                        binary.operator
                    ))
                })
            }
            _ => Err(CodegenError::new(format!(
                "LLVM backend cannot lower {:?} as integer type {:?}",
                expression, expected_type
            ))),
        }
    }

    fn codegen_text_expression(
        &self,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<PointerValue<'ctx>> {
        match &expression.kind {
            ExpressionKind::StringLiteral(value) => self
                .builder
                .build_global_string_ptr(value, "text_literal")
                .map(|value| value.as_pointer_value())
                .map_err(|error| {
                    CodegenError::new(format!("failed to build Text literal: {error}"))
                }),
            ExpressionKind::Identifier(name) => self
                .load_identifier_value(name, &TypeName::Text, bindings)
                .map(|value| value.into_pointer_value()),
            ExpressionKind::MethodCall(method_call) => {
                let method_type =
                    self.infer_method_call_type(method_call, bindings, current_function)?;
                if method_type != TypeName::Text {
                    return Err(CodegenError::new(format!(
                        "method '{}.{}()' has type {:?}, expected Text",
                        method_call.base_name, method_call.method_name, method_type
                    )));
                }

                self.codegen_method_call_value(
                    &TypeName::Text,
                    method_call,
                    current_function,
                    bindings,
                )
                .map(|value| value.into_pointer_value())
            }
            ExpressionKind::Dereference(dereference) => {
                let (dereference_type, dereference_value, _) =
                    self.codegen_dereference_expression(dereference, current_function, bindings)?;
                if dereference_type != TypeName::Text {
                    return Err(CodegenError::new(format!(
                        "dereference has type {:?}, expected Text",
                        dereference_type
                    )));
                }

                Ok(dereference_value.into_pointer_value())
            }
            ExpressionKind::FieldAccess(field_access) => {
                let (field_type, field_value) = self.codegen_field_access_expression(
                    &field_access.base_name,
                    &field_access.field_name,
                    current_function,
                    bindings,
                )?;
                if field_type != TypeName::Text {
                    return Err(CodegenError::new(format!(
                        "field '{}.{}' has type {:?}, expected Text",
                        field_access.base_name, field_access.field_name, field_type
                    )));
                }

                Ok(field_value.into_pointer_value())
            }
            ExpressionKind::IndexAccess(index_access) => {
                let (element_type, element_value) = self.codegen_index_access_expression(
                    &index_access.base_name,
                    &index_access.index,
                    current_function,
                    bindings,
                )?;
                if element_type != TypeName::Text {
                    return Err(CodegenError::new(format!(
                        "array element '{}[...]' has type {:?}, expected Text",
                        index_access.base_name, element_type
                    )));
                }

                Ok(element_value.into_pointer_value())
            }
            ExpressionKind::Call(call) => {
                if call.callee == "print" {
                    return Err(CodegenError::new(
                        "print() does not produce a Text value in LLVM backend",
                    ));
                }

                self.codegen_call_expression(call, current_function, bindings)
                    .map(|value| value.into_pointer_value())
            }
            _ => Err(CodegenError::new(format!(
                "LLVM backend cannot lower {:?} as Text",
                expression
            ))),
        }
    }

    fn codegen_float_expression(
        &self,
        expected_type: &TypeName,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<inkwell::values::FloatValue<'ctx>> {
        let float_type = self.llvm_basic_type(expected_type)?.into_float_type();

        match &expression.kind {
            ExpressionKind::FloatLiteral(value) => Ok(float_type.const_float(*value)),
            ExpressionKind::Dereference(dereference) => {
                let (dereference_type, dereference_value, _) =
                    self.codegen_dereference_expression(dereference, current_function, bindings)?;
                if &dereference_type != expected_type {
                    return Err(CodegenError::new(format!(
                        "dereference has type {:?}, expected {:?}",
                        dereference_type, expected_type
                    )));
                }

                Ok(dereference_value.into_float_value())
            }
            ExpressionKind::Cast(cast_expression) => self
                .codegen_cast_expression(
                    &cast_expression.target_type,
                    &cast_expression.value,
                    current_function,
                    bindings,
                )
                .map(|value| value.into_float_value()),
            ExpressionKind::Identifier(name) => self
                .load_identifier_value(name, expected_type, bindings)
                .map(|value| value.into_float_value()),
            ExpressionKind::FieldAccess(field_access) => {
                let (field_type, field_value) = self.codegen_field_access_expression(
                    &field_access.base_name,
                    &field_access.field_name,
                    current_function,
                    bindings,
                )?;
                if &field_type != expected_type {
                    return Err(CodegenError::new(format!(
                        "field '{}.{}' has type {:?}, expected {:?}",
                        field_access.base_name, field_access.field_name, field_type, expected_type
                    )));
                }

                Ok(field_value.into_float_value())
            }
            ExpressionKind::IndexAccess(index_access) => {
                let (element_type, element_value) = self.codegen_index_access_expression(
                    &index_access.base_name,
                    &index_access.index,
                    current_function,
                    bindings,
                )?;
                if &element_type != expected_type {
                    return Err(CodegenError::new(format!(
                        "array element '{}[...]' has type {:?}, expected {:?}",
                        index_access.base_name, element_type, expected_type
                    )));
                }

                Ok(element_value.into_float_value())
            }
            ExpressionKind::MethodCall(method_call) => {
                let method_type =
                    self.infer_method_call_type(method_call, bindings, current_function)?;
                if &method_type != expected_type {
                    return Err(CodegenError::new(format!(
                        "method '{}.{}()' has type {:?}, expected {:?}",
                        method_call.base_name, method_call.method_name, method_type, expected_type
                    )));
                }

                self.codegen_method_call_value(
                    expected_type,
                    method_call,
                    current_function,
                    bindings,
                )
                .map(|value| value.into_float_value())
            }
            ExpressionKind::Call(call) => {
                if call.callee == "print" {
                    return Err(CodegenError::new(
                        "print() does not produce a float value in LLVM backend",
                    ));
                }

                self.codegen_call_expression(call, current_function, bindings)
                    .map(|value| value.into_float_value())
            }
            ExpressionKind::Binary(binary) => {
                let left = self.codegen_float_expression(
                    expected_type,
                    &binary.left,
                    current_function,
                    bindings,
                )?;
                let right = self.codegen_float_expression(
                    expected_type,
                    &binary.right,
                    current_function,
                    bindings,
                )?;

                match binary.operator {
                    loz_ast::BinaryOperator::Add => {
                        self.builder.build_float_add(left, right, "faddtmp")
                    }
                    loz_ast::BinaryOperator::Subtract => {
                        self.builder.build_float_sub(left, right, "fsubtmp")
                    }
                    loz_ast::BinaryOperator::Multiply => {
                        self.builder.build_float_mul(left, right, "fmultmp")
                    }
                    loz_ast::BinaryOperator::Divide => {
                        self.builder.build_float_div(left, right, "fdivtmp")
                    }
                    _ => {
                        return Err(CodegenError::new(
                            "comparison expressions are not valid float values in LLVM backend",
                        ));
                    }
                }
                .map_err(|error| {
                    CodegenError::new(format!(
                        "failed to build float expression for {:?}: {error}",
                        binary.operator
                    ))
                })
            }
            _ => Err(CodegenError::new(format!(
                "LLVM backend cannot lower {:?} as float type {:?}",
                expression, expected_type
            ))),
        }
    }

    fn codegen_bool_expression(
        &self,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<IntValue<'ctx>> {
        match &expression.kind {
            ExpressionKind::BooleanLiteral(value) => {
                Ok(self.context.bool_type().const_int(u64::from(*value), false))
            }
            ExpressionKind::Dereference(dereference) => {
                let (dereference_type, dereference_value, _) =
                    self.codegen_dereference_expression(dereference, current_function, bindings)?;
                if dereference_type != TypeName::Bool {
                    return Err(CodegenError::new(format!(
                        "dereference has type {:?}, expected Bool",
                        dereference_type
                    )));
                }

                Ok(dereference_value.into_int_value())
            }
            ExpressionKind::Cast(cast_expression) => self
                .codegen_cast_expression(
                    &cast_expression.target_type,
                    &cast_expression.value,
                    current_function,
                    bindings,
                )
                .map(|value| value.into_int_value()),
            ExpressionKind::Identifier(name) => self
                .load_identifier_value(name, &TypeName::Bool, bindings)
                .map(|value| value.into_int_value()),
            ExpressionKind::FieldAccess(field_access) => {
                let (field_type, field_value) = self.codegen_field_access_expression(
                    &field_access.base_name,
                    &field_access.field_name,
                    current_function,
                    bindings,
                )?;
                if field_type != TypeName::Bool {
                    return Err(CodegenError::new(format!(
                        "field '{}.{}' has type {:?}, expected Bool",
                        field_access.base_name, field_access.field_name, field_type
                    )));
                }

                Ok(field_value.into_int_value())
            }
            ExpressionKind::IndexAccess(index_access) => {
                let (element_type, element_value) = self.codegen_index_access_expression(
                    &index_access.base_name,
                    &index_access.index,
                    current_function,
                    bindings,
                )?;
                if element_type != TypeName::Bool {
                    return Err(CodegenError::new(format!(
                        "array element '{}[...]' has type {:?}, expected Bool",
                        index_access.base_name, element_type
                    )));
                }

                Ok(element_value.into_int_value())
            }
            ExpressionKind::MethodCall(method_call) => {
                let method_type =
                    self.infer_method_call_type(method_call, bindings, current_function)?;
                if method_type != TypeName::Bool {
                    return Err(CodegenError::new(format!(
                        "method '{}.{}()' has type {:?}, expected Bool",
                        method_call.base_name, method_call.method_name, method_type
                    )));
                }

                self.codegen_method_call_value(
                    &TypeName::Bool,
                    method_call,
                    current_function,
                    bindings,
                )
                .map(|value| value.into_int_value())
            }
            ExpressionKind::Call(call) => {
                if call.callee == "print" {
                    return Err(CodegenError::new(
                        "print() does not produce a Bool value in LLVM backend",
                    ));
                }

                self.codegen_call_expression(call, current_function, bindings)
                    .map(|value| value.into_int_value())
            }
            ExpressionKind::Binary(binary) => {
                self.codegen_comparison_expression(binary, current_function, bindings)
            }
            _ => Err(CodegenError::new(format!(
                "LLVM backend cannot lower {:?} as Bool",
                expression
            ))),
        }
    }

    fn load_identifier_value(
        &self,
        name: &str,
        expected_type: &TypeName,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        if let Some(binding) = bindings.get(name).cloned() {
            match binding {
                FunctionBinding::Parameter { value, type_name } if type_name == *expected_type => {
                    Ok(value)
                }
                FunctionBinding::Parameter { type_name, .. } => Err(CodegenError::new(format!(
                    "parameter '{}' has type {:?}, expected {:?}",
                    name, type_name, expected_type
                ))),
                FunctionBinding::Local {
                    pointer, type_name, ..
                } if type_name == *expected_type => self
                    .builder
                    .build_load(
                        self.llvm_basic_type(expected_type)?,
                        pointer,
                        &format!("load_{name}"),
                    )
                    .map_err(|error| {
                        CodegenError::new(format!("failed to load local '{name}': {error}"))
                    }),
                FunctionBinding::Local { type_name, .. } => Err(CodegenError::new(format!(
                    "local '{}' has type {:?}, expected {:?}",
                    name, type_name, expected_type
                ))),
            }
        } else if let Some(global) = self.globals.get(name) {
            let global_type = self.global_types.get(name).ok_or_else(|| {
                CodegenError::new(format!("missing type metadata for global '{}'", name))
            })?;
            if global_type != expected_type {
                return Err(CodegenError::new(format!(
                    "global '{}' has type {:?}, expected {:?}",
                    name, global_type, expected_type
                )));
            }

            self.builder
                .build_load(
                    self.llvm_basic_type(expected_type)?,
                    global.as_pointer_value(),
                    &format!("load_{name}"),
                )
                .map_err(|error| {
                    CodegenError::new(format!("failed to load global '{name}': {error}"))
                })
        } else {
            Err(CodegenError::new(format!(
                "unknown identifier '{}' during LLVM codegen",
                name
            )))
        }
    }

    fn codegen_call_expression(
        &self,
        call: &loz_ast::CallExpression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        if self.async_tasks.contains_key(&call.callee) {
            return Err(CodegenError::new(format!(
                "async task '{}' must be awaited",
                call.callee
            )));
        }

        let (parameters, return_type, callee_name) =
            if let Some(function) = self.functions.get(&call.callee) {
                (
                    &function.parameters,
                    &function.return_type,
                    call.callee.clone(),
                )
            } else if let Some(tool) = self.tools.get(&call.callee) {
                (
                    &tool.parameters,
                    &tool.return_type,
                    self.tool_symbol_name(&tool.name),
                )
            } else {
                return Err(CodegenError::new(format!(
                    "unknown function or tool '{}' during LLVM codegen",
                    call.callee
                )));
            };

        if parameters.len() != call.arguments.len() {
            return Err(CodegenError::new(format!(
                "call '{}' expected {} arguments, found {}",
                call.callee,
                parameters.len(),
                call.arguments.len()
            )));
        }

        let callee = self.module.get_function(&callee_name).ok_or_else(|| {
            CodegenError::new(format!(
                "missing LLVM declaration for call target '{}'",
                callee_name
            ))
        })?;

        let arguments = parameters
            .iter()
            .zip(call.arguments.iter())
            .map(|(parameter, argument)| {
                self.codegen_value_expression(
                    &parameter.type_name,
                    argument,
                    current_function,
                    bindings,
                )
                .map(Into::into)
            })
            .collect::<CodegenResult<Vec<_>>>()?;

        let call_site = self
            .builder
            .build_call(callee, &arguments, "calltmp")
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to build call to '{}': {error}",
                    callee_name
                ))
            })?;

        if *return_type == TypeName::Void {
            Ok(self.context.i8_type().const_zero().into())
        } else {
            call_site.try_as_basic_value().basic().ok_or_else(|| {
                CodegenError::new(format!(
                    "call target '{}' did not produce a return value",
                    call.callee
                ))
            })
        }
    }

    fn codegen_await_expression(
        &self,
        await_expression: &AwaitExpression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        let ExpressionKind::Call(call) = &await_expression.expression.kind else {
            return Err(CodegenError::new(
                "await currently requires an async task call expression",
            ));
        };

        let task = self.async_tasks.get(&call.callee).ok_or_else(|| {
            if self.functions.contains_key(&call.callee) || self.tools.contains_key(&call.callee) {
                CodegenError::new(format!("cannot await non-async call '{}'", call.callee))
            } else {
                CodegenError::new(format!(
                    "await target '{}' must be an async task call",
                    call.callee
                ))
            }
        })?;

        if task.parameters.len() != call.arguments.len() {
            return Err(CodegenError::new(format!(
                "call '{}' expected {} arguments, found {}",
                call.callee,
                task.parameters.len(),
                call.arguments.len()
            )));
        }

        let callee_name = self.async_task_symbol_name(&call.callee);
        let callee = self.module.get_function(&callee_name).ok_or_else(|| {
            CodegenError::new(format!(
                "missing LLVM declaration for async task '{}'",
                call.callee
            ))
        })?;

        let arguments = task
            .parameters
            .iter()
            .zip(call.arguments.iter())
            .map(|(parameter, argument)| {
                self.codegen_value_expression(
                    &parameter.type_name,
                    argument,
                    current_function,
                    bindings,
                )
                .map(Into::into)
            })
            .collect::<CodegenResult<Vec<_>>>()?;

        let call_site = self
            .builder
            .build_call(callee, &arguments, "awaittmp")
            .map_err(|error| {
                CodegenError::new(format!(
                    "failed to build await call to '{}': {error}",
                    callee_name
                ))
            })?;

        if task.return_type == TypeName::Void {
            Ok(self.context.i8_type().const_zero().into())
        } else {
            call_site.try_as_basic_value().basic().ok_or_else(|| {
                CodegenError::new(format!(
                    "async task '{}' did not produce a return value",
                    call.callee
                ))
            })
        }
    }

    fn codegen_cast_expression(
        &self,
        target_type: &TypeName,
        value_expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        let source_type =
            self.infer_expression_type(value_expression, bindings, current_function)?;
        let source_value = self.codegen_primitive_expression(
            &source_type,
            value_expression,
            current_function,
            bindings,
        )?;
        self.cast_primitive_value(&source_type, target_type, source_value)
    }

    fn cast_primitive_value(
        &self,
        source_type: &TypeName,
        target_type: &TypeName,
        value: BasicValueEnum<'ctx>,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        if source_type == target_type {
            return Ok(value);
        }

        match (source_type, target_type) {
            (source, target) if source.is_integer() && target.is_integer() => {
                let source_value = value.into_int_value();
                let source_bits = source.bit_width().unwrap_or(64);
                let target_bits = target.bit_width().unwrap_or(64);
                let target_llvm_type = self.llvm_basic_type(target)?.into_int_type();

                if source_bits == target_bits {
                    Ok(source_value.into())
                } else if source_bits < target_bits {
                    if source.is_signed_integer() {
                        self.builder
                            .build_int_s_extend(source_value, target_llvm_type, "cast_sext")
                            .map(Into::into)
                            .map_err(|error| {
                                CodegenError::new(format!(
                                    "failed to build signed integer extension: {error}"
                                ))
                            })
                    } else {
                        self.builder
                            .build_int_z_extend(source_value, target_llvm_type, "cast_zext")
                            .map(Into::into)
                            .map_err(|error| {
                                CodegenError::new(format!(
                                    "failed to build unsigned integer extension: {error}"
                                ))
                            })
                    }
                } else {
                    self.builder
                        .build_int_truncate(source_value, target_llvm_type, "cast_trunc")
                        .map(Into::into)
                        .map_err(|error| {
                            CodegenError::new(format!(
                                "failed to build integer truncation: {error}"
                            ))
                        })
                }
            }
            (source, target) if source.is_integer() && target.is_float() => {
                let source_value = value.into_int_value();
                let target_llvm_type = self.llvm_basic_type(target)?.into_float_type();
                if source.is_signed_integer() {
                    self.builder
                        .build_signed_int_to_float(source_value, target_llvm_type, "cast_sitofp")
                        .map(Into::into)
                        .map_err(|error| {
                            CodegenError::new(format!(
                                "failed to build signed int-to-float cast: {error}"
                            ))
                        })
                } else {
                    self.builder
                        .build_unsigned_int_to_float(source_value, target_llvm_type, "cast_uitofp")
                        .map(Into::into)
                        .map_err(|error| {
                            CodegenError::new(format!(
                                "failed to build unsigned int-to-float cast: {error}"
                            ))
                        })
                }
            }
            (source, target) if source.is_float() && target.is_integer() => {
                let source_value = value.into_float_value();
                let target_llvm_type = self.llvm_basic_type(target)?.into_int_type();
                if target.is_signed_integer() {
                    self.builder
                        .build_float_to_signed_int(source_value, target_llvm_type, "cast_fptosi")
                        .map(Into::into)
                        .map_err(|error| {
                            CodegenError::new(format!(
                                "failed to build float-to-signed-int cast: {error}"
                            ))
                        })
                } else {
                    self.builder
                        .build_float_to_unsigned_int(source_value, target_llvm_type, "cast_fptoui")
                        .map(Into::into)
                        .map_err(|error| {
                            CodegenError::new(format!(
                                "failed to build float-to-unsigned-int cast: {error}"
                            ))
                        })
                }
            }
            (source, target) if source.is_float() && target.is_float() => {
                let source_value = value.into_float_value();
                let source_bits = source.bit_width().unwrap_or(64);
                let target_bits = target.bit_width().unwrap_or(64);
                let target_llvm_type = self.llvm_basic_type(target)?.into_float_type();

                if source_bits == target_bits {
                    Ok(source_value.into())
                } else if source_bits < target_bits {
                    self.builder
                        .build_float_ext(source_value, target_llvm_type, "cast_fpext")
                        .map(Into::into)
                        .map_err(|error| {
                            CodegenError::new(format!("failed to build float extension: {error}"))
                        })
                } else {
                    self.builder
                        .build_float_trunc(source_value, target_llvm_type, "cast_fptrunc")
                        .map(Into::into)
                        .map_err(|error| {
                            CodegenError::new(format!("failed to build float truncation: {error}"))
                        })
                }
            }
            (TypeName::Bool, target) if target.is_integer() => {
                let source_value = value.into_int_value();
                let target_llvm_type = self.llvm_basic_type(target)?.into_int_type();
                if target.bit_width().unwrap_or(1) == 1 {
                    Ok(source_value.into())
                } else {
                    self.builder
                        .build_int_z_extend(source_value, target_llvm_type, "cast_bool_zext")
                        .map(Into::into)
                        .map_err(|error| {
                            CodegenError::new(format!("failed to build Bool-to-int cast: {error}"))
                        })
                }
            }
            (source, TypeName::Bool) if source.is_integer() => {
                let source_value = value.into_int_value();
                let zero = self.llvm_basic_type(source)?.into_int_type().const_zero();
                self.builder
                    .build_int_compare(IntPredicate::NE, source_value, zero, "cast_to_bool")
                    .map(Into::into)
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build int-to-Bool cast: {error}"))
                    })
            }
            _ => Err(CodegenError::new(format!(
                "illegal LLVM cast from {:?} to {:?}",
                source_type, target_type
            ))),
        }
    }

    fn resolve_declaration_type(
        &self,
        declaration: &VariableDeclaration,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<TypeName> {
        let declared_type = if let Some(type_name) = &declaration.type_name {
            type_name.clone()
        } else {
            self.infer_expression_type(&declaration.value, bindings, current_function)?
        };

        match &declared_type {
            TypeName::Array(element_type, _) => Ok(TypeName::Array(
                element_type.clone(),
                Some(self.infer_array_length(&declaration.value, bindings)?),
            )),
            _ => Ok(declared_type),
        }
    }

    fn infer_array_length(
        &self,
        expression: &Expression,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<usize> {
        match &expression.kind {
            ExpressionKind::ArrayLiteral(array_literal) => Ok(array_literal.elements.len()),
            ExpressionKind::Identifier(name) => {
                let binding = bindings.get(name).cloned().ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown identifier '{}' during LLVM array type resolution",
                        name
                    ))
                })?;

                match binding {
                    FunctionBinding::Local {
                        type_name: TypeName::Array(_, Some(length)),
                        ..
                    }
                    | FunctionBinding::Parameter {
                        type_name: TypeName::Array(_, Some(length)),
                        ..
                    } => Ok(length),
                    _ => Err(CodegenError::new(format!(
                        "identifier '{}' does not have a concrete fixed array type in LLVM backend",
                        name
                    ))),
                }
            }
            _ => Err(CodegenError::new(
                "LLVM backend can only infer array lengths from literals or fixed array identifiers",
            )),
        }
    }

    fn infer_expression_type(
        &self,
        expression: &Expression,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
        _current_function: FunctionValue<'ctx>,
    ) -> CodegenResult<TypeName> {
        match &expression.kind {
            ExpressionKind::IntegerLiteral(_) => Ok(TypeName::I64),
            ExpressionKind::FloatLiteral(_) => Ok(TypeName::F64),
            ExpressionKind::BooleanLiteral(_) => Ok(TypeName::Bool),
            ExpressionKind::StringLiteral(_) => Ok(TypeName::Text),
            ExpressionKind::Reference(reference_expression) => {
                let target_type = self.infer_expression_type(
                    &synthetic_identifier(reference_expression.target_name.clone()),
                    bindings,
                    _current_function,
                )?;
                Ok(TypeName::Reference {
                    inner: Box::new(target_type),
                    is_mutable: reference_expression.is_mutable,
                })
            }
            ExpressionKind::Dereference(dereference_expression) => {
                let reference_type = self.infer_expression_type(
                    &dereference_expression.value,
                    bindings,
                    _current_function,
                )?;
                let TypeName::Reference { inner, .. } = reference_type else {
                    return Err(CodegenError::new(
                        "cannot infer dereference type for non-reference value",
                    ));
                };
                Ok(*inner)
            }
            ExpressionKind::Cast(cast_expression) => Ok(cast_expression.target_type.clone()),
            ExpressionKind::Identifier(name) => {
                if let Some(binding) = bindings.get(name) {
                    match binding {
                        FunctionBinding::Parameter { type_name, .. }
                        | FunctionBinding::Local { type_name, .. } => Ok(type_name.clone()),
                    }
                } else {
                    self.global_types.get(name).cloned().ok_or_else(|| {
                        CodegenError::new(format!(
                            "unknown identifier '{}' during LLVM type inference",
                            name
                        ))
                    })
                }
            }
            ExpressionKind::Await(await_expression) => {
                let ExpressionKind::Call(call) = &await_expression.expression.kind else {
                    return Err(CodegenError::new(
                        "await currently requires an async task call expression",
                    ));
                };

                if let Some(task) = self.async_tasks.get(&call.callee) {
                    Ok(task.return_type.clone())
                } else if self.functions.contains_key(&call.callee)
                    || self.tools.contains_key(&call.callee)
                {
                    Err(CodegenError::new(format!(
                        "cannot await non-async call '{}'",
                        call.callee
                    )))
                } else {
                    Err(CodegenError::new(format!(
                        "await target '{}' must be an async task call",
                        call.callee
                    )))
                }
            }
            ExpressionKind::FieldAccess(field_access) => {
                let base_type = self.infer_expression_type(
                    &synthetic_identifier(field_access.base_name.clone()),
                    bindings,
                    _current_function,
                )?;
                let TypeName::Named(struct_name) = base_type else {
                    return Err(CodegenError::new(format!(
                        "cannot infer field access type for non-struct '{}'",
                        field_access.base_name
                    )));
                };
                let struct_declaration = self.structs.get(&struct_name).ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown struct '{}' during LLVM field type inference",
                        struct_name
                    ))
                })?;
                let field = struct_declaration
                    .fields
                    .iter()
                    .find(|field| field.name == field_access.field_name)
                    .ok_or_else(|| {
                        CodegenError::new(format!(
                            "struct '{}' has no field '{}' during LLVM type inference",
                            struct_name, field_access.field_name
                        ))
                    })?;
                Ok(field.type_name.clone())
            }
            ExpressionKind::IndexAccess(index_access) => {
                let base_type = self.infer_expression_type(
                    &synthetic_identifier(index_access.base_name.clone()),
                    bindings,
                    _current_function,
                )?;
                let TypeName::Array(element_type, _) = base_type else {
                    return Err(CodegenError::new(format!(
                        "cannot infer index access type for non-array '{}'",
                        index_access.base_name
                    )));
                };
                Ok(*element_type)
            }
            ExpressionKind::MethodCall(method_call) => {
                self.infer_method_call_type(method_call, bindings, _current_function)
            }
            ExpressionKind::Call(call) => {
                if let Some(function) = self.functions.get(&call.callee) {
                    Ok(function.return_type.clone())
                } else if self.async_tasks.contains_key(&call.callee) {
                    Err(CodegenError::new(format!(
                        "async task '{}' must be awaited",
                        call.callee
                    )))
                } else if let Some(tool) = self.tools.get(&call.callee) {
                    Ok(tool.return_type.clone())
                } else if self.structs.contains_key(&call.callee) {
                    Ok(TypeName::Named(call.callee.clone()))
                } else {
                    Err(CodegenError::new(format!(
                        "unknown call target '{}' during LLVM type inference",
                        call.callee
                    )))
                }
            }
            ExpressionKind::ArrayLiteral(_) => Err(CodegenError::new(
                "LLVM backend requires an expected array type for array literals",
            )),
            ExpressionKind::Binary(binary) => match binary.operator {
                loz_ast::BinaryOperator::Add
                | loz_ast::BinaryOperator::Subtract
                | loz_ast::BinaryOperator::Multiply
                | loz_ast::BinaryOperator::Divide => {
                    let left =
                        self.infer_expression_type(&binary.left, bindings, _current_function)?;
                    let right =
                        self.infer_expression_type(&binary.right, bindings, _current_function)?;
                    self.reconcile_binary_operand_type(&left, &right)
                }
                _ => Ok(TypeName::Bool),
            },
        }
    }

    fn reconcile_binary_operand_type(
        &self,
        left: &TypeName,
        right: &TypeName,
    ) -> CodegenResult<TypeName> {
        if left == right {
            return Ok(left.clone());
        }

        if left.is_integer() && *right == TypeName::I64 {
            return Ok(left.clone());
        }

        if right.is_integer() && *left == TypeName::I64 {
            return Ok(right.clone());
        }

        if left.is_float() && *right == TypeName::F64 {
            return Ok(left.clone());
        }

        if right.is_float() && *left == TypeName::F64 {
            return Ok(right.clone());
        }

        Err(CodegenError::new(format!(
            "LLVM binary operand type mismatch: left={left:?}, right={right:?}"
        )))
    }

    fn const_index_from_expression(
        &self,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<usize> {
        let index_type = self.infer_expression_type(expression, bindings, current_function)?;
        if !index_type.is_integer() {
            return Err(CodegenError::new(format!(
                "array index must be an integer type in LLVM backend, found {:?}",
                index_type
            )));
        }

        let index =
            self.codegen_integer_expression(&index_type, expression, current_function, bindings)?;
        if !index.is_const() {
            return Err(CodegenError::new(
                "LLVM backend only supports constant array indices in this phase",
            ));
        }

        let constant_value = if index_type.is_unsigned_integer() {
            index
                .get_zero_extended_constant()
                .and_then(|value| i64::try_from(value).ok())
        } else {
            index.get_sign_extended_constant()
        };

        let Some(sign_extended_value) = constant_value else {
            return Err(CodegenError::new(
                "array index must be a constant integer value in LLVM backend",
            ));
        };

        usize::try_from(sign_extended_value)
            .map_err(|_| CodegenError::new("array index cannot be negative in LLVM backend"))
    }

    fn codegen_condition_expression(
        &self,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<IntValue<'ctx>> {
        self.codegen_bool_expression(expression, current_function, bindings)
    }

    fn codegen_comparison_expression(
        &self,
        binary: &loz_ast::BinaryExpression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<IntValue<'ctx>> {
        let left_type = self.infer_expression_type(&binary.left, bindings, current_function)?;
        let right_type = self.infer_expression_type(&binary.right, bindings, current_function)?;
        let operand_type = self.reconcile_binary_operand_type(&left_type, &right_type)?;

        if operand_type.is_integer() {
            let left = self.codegen_integer_expression(
                &operand_type,
                &binary.left,
                current_function,
                bindings,
            )?;
            let right = self.codegen_integer_expression(
                &operand_type,
                &binary.right,
                current_function,
                bindings,
            )?;

            let predicate = match binary.operator {
                loz_ast::BinaryOperator::Greater if operand_type.is_signed_integer() => {
                    IntPredicate::SGT
                }
                loz_ast::BinaryOperator::Greater => IntPredicate::UGT,
                loz_ast::BinaryOperator::Less if operand_type.is_signed_integer() => {
                    IntPredicate::SLT
                }
                loz_ast::BinaryOperator::Less => IntPredicate::ULT,
                loz_ast::BinaryOperator::GreaterEqual if operand_type.is_signed_integer() => {
                    IntPredicate::SGE
                }
                loz_ast::BinaryOperator::GreaterEqual => IntPredicate::UGE,
                loz_ast::BinaryOperator::LessEqual if operand_type.is_signed_integer() => {
                    IntPredicate::SLE
                }
                loz_ast::BinaryOperator::LessEqual => IntPredicate::ULE,
                loz_ast::BinaryOperator::Equal => IntPredicate::EQ,
                loz_ast::BinaryOperator::NotEqual => IntPredicate::NE,
                _ => {
                    return Err(CodegenError::new(
                        "unsupported integer comparison operator in LLVM backend",
                    ));
                }
            };

            return self
                .builder
                .build_int_compare(predicate, left, right, "cmp")
                .map_err(|error| {
                    CodegenError::new(format!("failed to build integer comparison: {error}"))
                });
        }

        if operand_type.is_float() {
            let left = self.codegen_float_expression(
                &operand_type,
                &binary.left,
                current_function,
                bindings,
            )?;
            let right = self.codegen_float_expression(
                &operand_type,
                &binary.right,
                current_function,
                bindings,
            )?;

            let predicate = match binary.operator {
                loz_ast::BinaryOperator::Greater => FloatPredicate::OGT,
                loz_ast::BinaryOperator::Less => FloatPredicate::OLT,
                loz_ast::BinaryOperator::GreaterEqual => FloatPredicate::OGE,
                loz_ast::BinaryOperator::LessEqual => FloatPredicate::OLE,
                loz_ast::BinaryOperator::Equal => FloatPredicate::OEQ,
                loz_ast::BinaryOperator::NotEqual => FloatPredicate::ONE,
                _ => {
                    return Err(CodegenError::new(
                        "unsupported float comparison operator in LLVM backend",
                    ));
                }
            };

            return self
                .builder
                .build_float_compare(predicate, left, right, "fcmp")
                .map_err(|error| {
                    CodegenError::new(format!("failed to build float comparison: {error}"))
                });
        }

        if operand_type == TypeName::Bool {
            let left = self.codegen_bool_expression(&binary.left, current_function, bindings)?;
            let right = self.codegen_bool_expression(&binary.right, current_function, bindings)?;
            let predicate = match binary.operator {
                loz_ast::BinaryOperator::Equal => IntPredicate::EQ,
                loz_ast::BinaryOperator::NotEqual => IntPredicate::NE,
                _ => {
                    return Err(CodegenError::new(
                        "only == and != are supported for Bool comparisons in LLVM backend",
                    ));
                }
            };

            return self
                .builder
                .build_int_compare(predicate, left, right, "boolcmp")
                .map_err(|error| {
                    CodegenError::new(format!("failed to build Bool comparison: {error}"))
                });
        }

        Err(CodegenError::new(format!(
            "unsupported comparison operand type {:?} in LLVM backend",
            operand_type
        )))
    }

    fn codegen_print_expression(
        &self,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<()> {
        let expression_type = self.infer_expression_type(expression, bindings, current_function)?;

        match expression_type {
            TypeName::Text => {
                let value = self.codegen_text_expression(expression, current_function, bindings)?;
                let puts = self.module.get_function("puts").ok_or_else(|| {
                    CodegenError::new("puts declaration is missing from LLVM module")
                })?;
                self.builder
                    .build_call(puts, &[value.into()], "puts_text")
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build Text puts call: {error}"))
                    })?;
                Ok(())
            }
            TypeName::Bool => {
                let value = self.codegen_bool_expression(expression, current_function, bindings)?;
                let puts = self.module.get_function("puts").ok_or_else(|| {
                    CodegenError::new("puts declaration is missing from LLVM module")
                })?;
                let true_text = self
                    .builder
                    .build_global_string_ptr("true", "print_true")
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build Bool print string: {error}"))
                    })?;
                let false_text = self
                    .builder
                    .build_global_string_ptr("false", "print_false")
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build Bool print string: {error}"))
                    })?;
                let selected = self
                    .builder
                    .build_select(
                        value,
                        true_text.as_pointer_value(),
                        false_text.as_pointer_value(),
                        "bool_print_ptr",
                    )
                    .map_err(|error| {
                        CodegenError::new(format!("failed to select Bool print string: {error}"))
                    })?;

                self.builder
                    .build_call(puts, &[selected.into()], "puts_bool")
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build Bool puts call: {error}"))
                    })?;
                Ok(())
            }
            type_name if type_name.is_integer() => {
                let value = self.codegen_integer_expression(
                    &type_name,
                    expression,
                    current_function,
                    bindings,
                )?;
                let widened = if type_name.bit_width().unwrap_or(64) < 64 {
                    if type_name.is_signed_integer() {
                        self.builder.build_int_s_extend(
                            value,
                            self.context.i64_type(),
                            "print_sext",
                        )
                    } else {
                        self.builder.build_int_z_extend(
                            value,
                            self.context.i64_type(),
                            "print_zext",
                        )
                    }
                    .map_err(|error| {
                        CodegenError::new(format!("failed to widen integer for print: {error}"))
                    })?
                } else {
                    value
                };

                let printf = self.module.get_function("printf").ok_or_else(|| {
                    CodegenError::new("printf declaration is missing from LLVM module")
                })?;
                let format_string = self
                    .builder
                    .build_global_string_ptr(
                        if type_name.is_signed_integer() {
                            "%lld\n"
                        } else {
                            "%llu\n"
                        },
                        "printf_fmt_int",
                    )
                    .map_err(|error| {
                        CodegenError::new(format!(
                            "failed to build integer printf format string: {error}"
                        ))
                    })?;

                self.builder
                    .build_call(
                        printf,
                        &[format_string.as_pointer_value().into(), widened.into()],
                        "printf_int",
                    )
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build integer printf call: {error}"))
                    })?;
                Ok(())
            }
            type_name if type_name.is_float() => {
                let value = self.codegen_float_expression(
                    &type_name,
                    expression,
                    current_function,
                    bindings,
                )?;
                let widened = if type_name == TypeName::F32 {
                    self.builder
                        .build_float_ext(value, self.context.f64_type(), "print_fpext")
                        .map_err(|error| {
                            CodegenError::new(format!("failed to widen float for print: {error}"))
                        })?
                } else {
                    value
                };
                let printf = self.module.get_function("printf").ok_or_else(|| {
                    CodegenError::new("printf declaration is missing from LLVM module")
                })?;
                let integral_format_string = self
                    .builder
                    .build_global_string_ptr("%.1f\n", "printf_fmt_float_integral")
                    .map_err(|error| {
                        CodegenError::new(format!(
                            "failed to build integral float printf format string: {error}"
                        ))
                    })?;
                let fractional_format_string = self
                    .builder
                    .build_global_string_ptr("%g\n", "printf_fmt_float_fractional")
                    .map_err(|error| {
                        CodegenError::new(format!(
                            "failed to build fractional float printf format string: {error}"
                        ))
                    })?;
                let truncated = self
                    .builder
                    .build_float_to_signed_int(
                        widened,
                        self.context.i64_type(),
                        "print_float_to_i64",
                    )
                    .map_err(|error| {
                        CodegenError::new(format!(
                            "failed to convert float to int for print formatting: {error}"
                        ))
                    })?;
                let restored = self
                    .builder
                    .build_signed_int_to_float(
                        truncated,
                        self.context.f64_type(),
                        "print_i64_to_float",
                    )
                    .map_err(|error| {
                        CodegenError::new(format!(
                            "failed to convert int back to float for print formatting: {error}"
                        ))
                    })?;
                let is_integral = self
                    .builder
                    .build_float_compare(
                        FloatPredicate::OEQ,
                        widened,
                        restored,
                        "print_is_integral",
                    )
                    .map_err(|error| {
                        CodegenError::new(format!("failed to compare float print value: {error}"))
                    })?;
                let format_string = self
                    .builder
                    .build_select(
                        is_integral,
                        integral_format_string.as_pointer_value(),
                        fractional_format_string.as_pointer_value(),
                        "printf_fmt_float",
                    )
                    .map_err(|error| {
                        CodegenError::new(format!(
                            "failed to select float printf format string: {error}"
                        ))
                    })?;

                self.builder
                    .build_call(
                        printf,
                        &[format_string.into(), widened.into()],
                        "printf_float",
                    )
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build float printf call: {error}"))
                    })?;
                Ok(())
            }
            _ => Err(CodegenError::new(format!(
                "print() is not supported for type {:?} in LLVM backend",
                expression_type
            ))),
        }
    }

    fn evaluate_const_primitive_expression(
        &self,
        expected_type: &TypeName,
        expression: &Expression,
    ) -> CodegenResult<BasicValueEnum<'ctx>> {
        match expected_type {
            type_name if type_name.is_integer() => {
                let int_type = self.llvm_basic_type(type_name)?.into_int_type();
                let value = self.evaluate_const_integer_expression(expression)?;
                Ok(int_type
                    .const_int(value as u64, type_name.is_signed_integer())
                    .into())
            }
            type_name if type_name.is_float() => {
                let float_type = self.llvm_basic_type(type_name)?.into_float_type();
                let value = self.evaluate_const_float_expression(expression)?;
                Ok(float_type.const_float(value).into())
            }
            TypeName::Bool => match &expression.kind {
                ExpressionKind::BooleanLiteral(value) => Ok(self
                    .context
                    .bool_type()
                    .const_int(u64::from(*value), false)
                    .into()),
                _ => Err(CodegenError::new(
                    "LLVM backend only supports constant Bool literals for global initializers",
                )),
            },
            _ => Err(CodegenError::new(format!(
                "LLVM global initializers do not support type {:?}",
                expected_type
            ))),
        }
    }

    fn evaluate_const_integer_expression(&self, expression: &Expression) -> CodegenResult<i64> {
        match &expression.kind {
            ExpressionKind::IntegerLiteral(value) => Ok(*value),
            ExpressionKind::Binary(binary) => {
                let left = self.evaluate_const_integer_expression(&binary.left)?;
                let right = self.evaluate_const_integer_expression(&binary.right)?;
                match binary.operator {
                    loz_ast::BinaryOperator::Add => Ok(left + right),
                    loz_ast::BinaryOperator::Subtract => Ok(left - right),
                    loz_ast::BinaryOperator::Multiply => Ok(left * right),
                    loz_ast::BinaryOperator::Divide => Ok(left / right),
                    _ => Err(CodegenError::new(
                        "global integer constant expressions only support arithmetic operators",
                    )),
                }
            }
            _ => Err(CodegenError::new(
                "LLVM backend only supports constant integer expressions for global initializers",
            )),
        }
    }

    fn evaluate_const_float_expression(&self, expression: &Expression) -> CodegenResult<f64> {
        match &expression.kind {
            ExpressionKind::FloatLiteral(value) => Ok(*value),
            ExpressionKind::Binary(binary) => {
                let left = self.evaluate_const_float_expression(&binary.left)?;
                let right = self.evaluate_const_float_expression(&binary.right)?;
                match binary.operator {
                    loz_ast::BinaryOperator::Add => Ok(left + right),
                    loz_ast::BinaryOperator::Subtract => Ok(left - right),
                    loz_ast::BinaryOperator::Multiply => Ok(left * right),
                    loz_ast::BinaryOperator::Divide => Ok(left / right),
                    _ => Err(CodegenError::new(
                        "global float constant expressions only support arithmetic operators",
                    )),
                }
            }
            _ => Err(CodegenError::new(
                "LLVM backend only supports constant float expressions for global initializers",
            )),
        }
    }

    pub fn finish(self) -> String {
        self.module.print_to_string().to_string()
    }
}

pub fn generate_llvm_ir(program: &Program) -> CodegenResult<String> {
    let context = Context::create();
    let mut generator = LlvmIrGenerator::new(&context, "loz_module");
    generator.generate_program(program)?;
    Ok(generator.finish())
}
