use std::collections::HashMap;

use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::{FunctionValue, GlobalValue, IntValue, PointerValue};
use loz_ast::{
    Expression, FunctionDeclaration, IfStatement, Program, Statement, VariableDeclaration,
    WhileStatement,
};

use super::{CodegenError, CodegenResult};

pub struct LlvmIrGenerator<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    globals: HashMap<String, GlobalValue<'ctx>>,
}

#[derive(Clone, Copy)]
enum FunctionBinding<'ctx> {
    Parameter(IntValue<'ctx>),
    Local {
        pointer: PointerValue<'ctx>,
        is_mutable: bool,
    },
}

impl<'ctx> LlvmIrGenerator<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self {
            context,
            module: context.create_module(module_name),
            builder: context.create_builder(),
            globals: HashMap::new(),
        }
    }

    pub fn generate_program(&mut self, program: &Program) -> CodegenResult<()> {
        self.declare_printf();

        for statement in &program.statements {
            match statement {
                Statement::VariableDeclaration(declaration) => self.declare_global(declaration)?,
                Statement::FunctionDeclaration(function) => self.declare_function(function)?,
                _ => {
                    return Err(CodegenError::new(
                        "LLVM backend only supports top-level const Int globals and function declarations",
                    ));
                }
            }
        }

        for statement in &program.statements {
            if let Statement::FunctionDeclaration(function) = statement {
                self.define_function_body(function)?;
            }
        }

        if self.module.get_function("main").is_none() {
            return Err(CodegenError::new(
                "LLVM backend requires a 'main' function returning Int",
            ));
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

    fn declare_global(&mut self, declaration: &VariableDeclaration) -> CodegenResult<()> {
        if declaration.is_mutable {
            return Err(CodegenError::new(format!(
                "LLVM backend does not support mutable global '{}' yet",
                declaration.name
            )));
        }

        if !matches!(declaration.type_name, loz_ast::TypeName::Int) {
            return Err(CodegenError::new(format!(
                "LLVM backend only supports Int global '{}' currently",
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

        let initializer = self.context.i64_type().const_int(
            self.evaluate_const_int_expression(&declaration.value)? as u64,
            true,
        );

        let global = self
            .module
            .add_global(self.context.i64_type(), None, &declaration.name);
        global.set_initializer(&initializer);
        global.set_constant(true);
        self.globals.insert(declaration.name.clone(), global);

        Ok(())
    }

    fn declare_function(&self, function: &FunctionDeclaration) -> CodegenResult<()> {
        if !matches!(function.return_type, loz_ast::TypeName::Int) {
            return Err(CodegenError::new(format!(
                "LLVM backend only supports Int return type for function '{}'",
                function.name
            )));
        }

        if self.module.get_function(&function.name).is_some()
            || self.globals.contains_key(&function.name)
        {
            return Err(CodegenError::new(format!(
                "duplicate function '{}' during LLVM declaration",
                function.name
            )));
        }

        let parameter_types = function
            .parameters
            .iter()
            .map(|parameter| match parameter.type_name {
                loz_ast::TypeName::Int => Ok(self.context.i64_type().into()),
                _ => Err(CodegenError::new(format!(
                    "LLVM backend only supports Int parameters for function '{}'",
                    function.name
                ))),
            })
            .collect::<CodegenResult<Vec<_>>>()?;

        let function_type = self.context.i64_type().fn_type(&parameter_types, false);
        self.module
            .add_function(&function.name, function_type, None);
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
            if let Some(return_value) =
                self.codegen_function_statement(statement, function_value, &mut bindings)?
            {
                self.builder
                    .build_return(Some(&return_value))
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build return: {error}"))
                    })?;
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

    fn build_function_scope(
        &self,
        function: &FunctionDeclaration,
        function_value: FunctionValue<'ctx>,
    ) -> CodegenResult<HashMap<String, FunctionBinding<'ctx>>> {
        let mut bindings = HashMap::new();

        for (index, parameter) in function.parameters.iter().enumerate() {
            let value = function_value
                .get_nth_param(index as u32)
                .ok_or_else(|| {
                    CodegenError::new(format!(
                        "missing LLVM parameter {} for function '{}'",
                        index, function.name
                    ))
                })?
                .into_int_value();

            bindings.insert(parameter.name.clone(), FunctionBinding::Parameter(value));
        }

        Ok(bindings)
    }

    fn codegen_function_statement(
        &self,
        statement: &Statement,
        current_function: FunctionValue<'ctx>,
        bindings: &mut HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<Option<IntValue<'ctx>>> {
        match statement {
            Statement::VariableDeclaration(declaration) => {
                self.codegen_local_variable_declaration(declaration, current_function, bindings)?;
                Ok(None)
            }
            Statement::Assignment(assignment) => {
                let binding = bindings.get(&assignment.target).copied().ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown local identifier '{}' during LLVM assignment",
                        assignment.target
                    ))
                })?;

                let FunctionBinding::Local {
                    pointer,
                    is_mutable,
                } = binding
                else {
                    return Err(CodegenError::new(format!(
                        "LLVM backend only supports assignment to local variables, not '{}'",
                        assignment.target
                    )));
                };

                if !is_mutable {
                    return Err(CodegenError::new(format!(
                        "cannot assign to immutable local '{}'",
                        assignment.target
                    )));
                }

                let value =
                    self.codegen_int_expression(&assignment.value, current_function, bindings)?;
                self.builder.build_store(pointer, value).map_err(|error| {
                    CodegenError::new(format!(
                        "failed to store local '{}': {error}",
                        assignment.target
                    ))
                })?;
                Ok(None)
            }
            Statement::If(if_statement) => {
                self.codegen_if_statement(if_statement, current_function, bindings)?;
                Ok(None)
            }
            Statement::While(while_statement) => {
                self.codegen_while_statement(while_statement, current_function, bindings)?;
                Ok(None)
            }
            Statement::Return(return_statement) => {
                let value = self.codegen_int_expression(
                    &return_statement.value,
                    current_function,
                    bindings,
                )?;
                Ok(Some(value))
            }
            Statement::Expression(expression) => {
                self.codegen_expression_statement(expression, current_function, bindings)?;
                Ok(None)
            }
            Statement::FunctionDeclaration(_) => Err(CodegenError::new(
                "nested function declarations are not supported in LLVM backend",
            )),
        }
    }

    fn codegen_expression_statement(
        &self,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<()> {
        if let Expression::Call(call) = expression {
            if call.callee == "print" {
                if call.arguments.len() != 1 {
                    return Err(CodegenError::new(
                        "print() expects exactly one Int argument in LLVM backend",
                    ));
                }

                let value =
                    self.codegen_int_expression(&call.arguments[0], current_function, bindings)?;
                let printf = self.module.get_function("printf").ok_or_else(|| {
                    CodegenError::new("printf declaration is missing from LLVM module")
                })?;
                let format_string = self
                    .builder
                    .build_global_string_ptr("%ld\n", "printf_fmt")
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build printf format string: {error}"))
                    })?;

                self.builder
                    .build_call(
                        printf,
                        &[format_string.as_pointer_value().into(), value.into()],
                        "printf_call",
                    )
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build printf call: {error}"))
                    })?;

                return Ok(());
            }
        }

        self.codegen_int_expression(expression, current_function, bindings)?;
        Ok(())
    }

    fn codegen_if_statement(
        &self,
        if_statement: &IfStatement,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<()> {
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
            self.codegen_branch_statements(else_branch, current_function, &mut else_bindings)?
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

        self.builder.position_at_end(merge_block);
        Ok(())
    }

    fn codegen_while_statement(
        &self,
        while_statement: &WhileStatement,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
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
        let body_terminated = self.codegen_branch_statements(
            &while_statement.body,
            current_function,
            &mut body_bindings,
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
    ) -> CodegenResult<bool> {
        for statement in statements {
            if let Some(return_value) =
                self.codegen_function_statement(statement, current_function, bindings)?
            {
                self.builder
                    .build_return(Some(&return_value))
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build branch return: {error}"))
                    })?;
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
        if !matches!(declaration.type_name, loz_ast::TypeName::Int) {
            return Err(CodegenError::new(format!(
                "LLVM backend only supports local Int variable '{}' currently",
                declaration.name
            )));
        }

        if bindings.contains_key(&declaration.name) {
            return Err(CodegenError::new(format!(
                "duplicate local identifier '{}' during LLVM codegen",
                declaration.name
            )));
        }

        let pointer = self.create_entry_alloca(current_function, &declaration.name)?;
        let value = self.codegen_int_expression(&declaration.value, current_function, bindings)?;
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
            },
        );

        Ok(())
    }

    fn create_entry_alloca(
        &self,
        function: FunctionValue<'ctx>,
        name: &str,
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

        builder
            .build_alloca(self.context.i64_type(), name)
            .map_err(|error| {
                CodegenError::new(format!("failed to allocate local '{name}': {error}"))
            })
    }

    fn codegen_int_expression(
        &self,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<IntValue<'ctx>> {
        let _ = current_function;

        match expression {
            Expression::IntegerLiteral(value) => {
                Ok(self.context.i64_type().const_int(*value as u64, true))
            }
            Expression::Identifier(name) => {
                if let Some(binding) = bindings.get(name).copied() {
                    match binding {
                        FunctionBinding::Parameter(value) => Ok(value),
                        FunctionBinding::Local { pointer, .. } => {
                            let value = self
                                .builder
                                .build_load(
                                    self.context.i64_type(),
                                    pointer,
                                    &format!("load_{name}"),
                                )
                                .map_err(|error| {
                                    CodegenError::new(format!(
                                        "failed to load local '{name}': {error}"
                                    ))
                                })?
                                .into_int_value();
                            Ok(value)
                        }
                    }
                } else if let Some(global) = self.globals.get(name) {
                    let value = self
                        .builder
                        .build_load(
                            self.context.i64_type(),
                            global.as_pointer_value(),
                            &format!("load_{name}"),
                        )
                        .map_err(|error| {
                            CodegenError::new(format!("failed to load global '{name}': {error}"))
                        })?
                        .into_int_value();
                    Ok(value)
                } else {
                    Err(CodegenError::new(format!(
                        "unknown identifier '{}' during LLVM codegen",
                        name
                    )))
                }
            }
            Expression::Call(call) => {
                if call.callee == "print" {
                    return Err(CodegenError::new(
                        "print() does not produce an Int value in LLVM backend",
                    ));
                }

                let callee = self.module.get_function(&call.callee).ok_or_else(|| {
                    CodegenError::new(format!(
                        "unknown function '{}' during LLVM codegen",
                        call.callee
                    ))
                })?;

                let arguments = call
                    .arguments
                    .iter()
                    .map(|argument| {
                        self.codegen_int_expression(argument, current_function, bindings)
                            .map(Into::into)
                    })
                    .collect::<CodegenResult<Vec<_>>>()?;

                let call_site = self
                    .builder
                    .build_call(callee, &arguments, "calltmp")
                    .map_err(|error| {
                        CodegenError::new(format!(
                            "failed to build call to '{}': {error}",
                            call.callee
                        ))
                    })?;

                let value = call_site
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| {
                        CodegenError::new(format!(
                            "function '{}' did not produce a return value",
                            call.callee
                        ))
                    })?
                    .into_int_value();
                Ok(value)
            }
            Expression::Binary(binary) => {
                let left = self.codegen_int_expression(&binary.left, current_function, bindings)?;
                let right =
                    self.codegen_int_expression(&binary.right, current_function, bindings)?;

                match binary.operator {
                    loz_ast::BinaryOperator::Add => self
                        .builder
                        .build_int_add(left, right, "addtmp")
                        .map_err(|error| {
                            CodegenError::new(format!("failed to build integer add: {error}"))
                        }),
                    loz_ast::BinaryOperator::Subtract => self
                        .builder
                        .build_int_sub(left, right, "subtmp")
                        .map_err(|error| {
                            CodegenError::new(format!("failed to build integer sub: {error}"))
                        }),
                    loz_ast::BinaryOperator::Multiply => self
                        .builder
                        .build_int_mul(left, right, "multmp")
                        .map_err(|error| {
                            CodegenError::new(format!("failed to build integer mul: {error}"))
                        }),
                    loz_ast::BinaryOperator::Divide => self
                        .builder
                        .build_int_signed_div(left, right, "divtmp")
                        .map_err(|error| {
                            CodegenError::new(format!("failed to build integer div: {error}"))
                        }),
                    _ => Err(CodegenError::new(
                        "comparison expressions are not valid Int values in LLVM backend",
                    )),
                }
            }
            _ => Err(CodegenError::new(
                "LLVM backend only supports Int identifiers, Int calls, integer literals, and integer binary expressions",
            )),
        }
    }

    fn codegen_condition_expression(
        &self,
        expression: &Expression,
        current_function: FunctionValue<'ctx>,
        bindings: &HashMap<String, FunctionBinding<'ctx>>,
    ) -> CodegenResult<IntValue<'ctx>> {
        match expression {
            Expression::Binary(binary) => {
                let left = self.codegen_int_expression(&binary.left, current_function, bindings)?;
                let right =
                    self.codegen_int_expression(&binary.right, current_function, bindings)?;

                let predicate = match binary.operator {
                    loz_ast::BinaryOperator::Greater => IntPredicate::SGT,
                    loz_ast::BinaryOperator::Less => IntPredicate::SLT,
                    loz_ast::BinaryOperator::GreaterEqual => IntPredicate::SGE,
                    loz_ast::BinaryOperator::LessEqual => IntPredicate::SLE,
                    loz_ast::BinaryOperator::Equal => IntPredicate::EQ,
                    loz_ast::BinaryOperator::NotEqual => IntPredicate::NE,
                    _ => {
                        return Err(CodegenError::new(
                            "if conditions in LLVM backend must use Int comparison operators",
                        ));
                    }
                };

                self.builder
                    .build_int_compare(predicate, left, right, "ifcond")
                    .map_err(|error| {
                        CodegenError::new(format!("failed to build integer comparison: {error}"))
                    })
            }
            _ => Err(CodegenError::new(
                "if conditions in LLVM backend must be Int comparison expressions",
            )),
        }
    }

    fn evaluate_const_int_expression(&self, expression: &Expression) -> CodegenResult<i64> {
        match expression {
            Expression::IntegerLiteral(value) => Ok(*value),
            Expression::Binary(binary) => {
                let left = self.evaluate_const_int_expression(&binary.left)?;
                let right = self.evaluate_const_int_expression(&binary.right)?;

                match binary.operator {
                    loz_ast::BinaryOperator::Add => Ok(left + right),
                    loz_ast::BinaryOperator::Subtract => Ok(left - right),
                    loz_ast::BinaryOperator::Multiply => Ok(left * right),
                    loz_ast::BinaryOperator::Divide => Ok(left / right),
                    _ => Err(CodegenError::new(
                        "global constant expressions only support arithmetic operators",
                    )),
                }
            }
            _ => Err(CodegenError::new(
                "LLVM backend only supports constant integer expressions for global initializers",
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
