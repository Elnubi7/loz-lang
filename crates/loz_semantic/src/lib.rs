use std::collections::HashMap;
use std::fmt;

use loz_ast::{
    BinaryOperator, Expression, FunctionDeclaration, IfStatement, Program, Statement, TypeName,
    VariableDeclaration, WhileStatement,
};

pub type SemanticResult<T> = Result<T, SemanticError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticError {
    pub message: String,
}

impl SemanticError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SemanticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "semantic error: {}", self.message)
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
    symbols: SymbolTable,
    functions: HashMap<String, FunctionSymbol>,
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        let mut functions = HashMap::new();
        functions.insert(
            "print".to_string(),
            FunctionSymbol {
                parameter_types: vec![TypeName::Named("Any".to_string())],
                return_type: TypeName::Void,
            },
        );

        Self {
            symbols: SymbolTable::new(),
            functions,
        }
    }

    pub fn analyze(&mut self, program: &Program) -> SemanticResult<()> {
        self.collect_functions(program)?;

        for statement in &program.statements {
            self.analyze_statement(statement, None)?;
        }

        Ok(())
    }

    fn collect_functions(&mut self, program: &Program) -> SemanticResult<()> {
        for statement in &program.statements {
            if let Statement::FunctionDeclaration(function) = statement {
                if self.functions.contains_key(&function.name) {
                    return Err(SemanticError::new(format!(
                        "duplicate function name '{}'",
                        function.name
                    )));
                }

                self.functions.insert(
                    function.name.clone(),
                    FunctionSymbol {
                        parameter_types: function
                            .parameters
                            .iter()
                            .map(|parameter| parameter.type_name.clone())
                            .collect(),
                        return_type: function.return_type.clone(),
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
    ) -> SemanticResult<()> {
        match statement {
            Statement::VariableDeclaration(declaration) => {
                self.analyze_variable_declaration(declaration)
            }
            Statement::FunctionDeclaration(function) => self.analyze_function_declaration(function),
            Statement::If(if_statement) => {
                self.analyze_if_statement(if_statement, current_return_type)
            }
            Statement::While(while_statement) => {
                self.analyze_while_statement(while_statement, current_return_type)
            }
            Statement::Return(return_statement) => {
                let expected_type = current_return_type.ok_or_else(|| {
                    SemanticError::new("return statement is not allowed outside a function")
                })?;

                let actual_type = self.infer_expression_type(&return_statement.value)?;
                self.ensure_type_matches(expected_type, &actual_type, "return statement")
            }
            Statement::Assignment(assignment) => {
                let symbol = self
                    .symbols
                    .lookup(&assignment.target)
                    .cloned()
                    .ok_or_else(|| {
                        SemanticError::new(format!(
                            "assignment to undeclared identifier '{}'",
                            assignment.target
                        ))
                    })?;

                if !symbol.is_mutable {
                    return Err(SemanticError::new(format!(
                        "cannot reassign immutable value '{}'",
                        assignment.target
                    )));
                }

                let value_type = self.infer_expression_type(&assignment.value)?;
                self.ensure_type_matches(&symbol.type_name, &value_type, "assignment")
            }
            Statement::Expression(expression) => {
                self.infer_expression_type(expression)?;
                Ok(())
            }
        }
    }

    fn analyze_variable_declaration(
        &mut self,
        declaration: &VariableDeclaration,
    ) -> SemanticResult<()> {
        let value_type = self.infer_expression_type(&declaration.value)?;
        self.ensure_type_matches(&declaration.type_name, &value_type, "variable declaration")?;

        self.symbols.insert(
            declaration.name.clone(),
            Symbol {
                type_name: declaration.type_name.clone(),
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
        self.ensure_type_matches(&TypeName::Bool, &condition_type, "if condition")?;

        self.symbols.push_scope();
        for statement in &if_statement.then_branch {
            self.analyze_statement(statement, current_return_type)?;
        }
        self.symbols.pop_scope();

        if let Some(else_branch) = &if_statement.else_branch {
            self.symbols.push_scope();
            for statement in else_branch {
                self.analyze_statement(statement, current_return_type)?;
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
        self.ensure_type_matches(&TypeName::Bool, &condition_type, "while condition")?;

        self.symbols.push_scope();
        for statement in &while_statement.body {
            self.analyze_statement(statement, current_return_type)?;
        }
        self.symbols.pop_scope();

        Ok(())
    }

    fn analyze_function_declaration(
        &mut self,
        function: &FunctionDeclaration,
    ) -> SemanticResult<()> {
        self.symbols.push_scope();

        for parameter in &function.parameters {
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

            self.analyze_statement(statement, Some(&function.return_type))?;
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

    fn infer_expression_type(&self, expression: &Expression) -> SemanticResult<TypeName> {
        match expression {
            Expression::IntegerLiteral(_) => Ok(TypeName::Int),
            Expression::FloatLiteral(_) => Ok(TypeName::Float),
            Expression::BooleanLiteral(_) => Ok(TypeName::Bool),
            Expression::StringLiteral(_) => Ok(TypeName::Text),
            Expression::Identifier(name) => self
                .symbols
                .lookup(name)
                .map(|symbol| symbol.type_name.clone())
                .ok_or_else(|| SemanticError::new(format!("undeclared identifier '{}'", name))),
            Expression::Call(call) => {
                let function = self.functions.get(&call.callee).ok_or_else(|| {
                    SemanticError::new(format!("undeclared function '{}'", call.callee))
                })?;

                let accepts_any = function.parameter_types.len() == 1
                    && function.parameter_types[0] == TypeName::Named("Any".to_string())
                    && call.callee == "print";

                if !accepts_any && function.parameter_types.len() != call.arguments.len() {
                    return Err(SemanticError::new(format!(
                        "function '{}' expected {} arguments, found {}",
                        call.callee,
                        function.parameter_types.len(),
                        call.arguments.len()
                    )));
                }

                if !accepts_any {
                    for (argument, parameter_type) in
                        call.arguments.iter().zip(function.parameter_types.iter())
                    {
                        let argument_type = self.infer_expression_type(argument)?;
                        self.ensure_type_matches(parameter_type, &argument_type, "function call")?;
                    }
                } else if call.arguments.len() != 1 {
                    return Err(SemanticError::new(
                        "built-in function 'print' expects exactly one argument",
                    ));
                } else {
                    self.infer_expression_type(&call.arguments[0])?;
                }

                Ok(function.return_type.clone())
            }
            Expression::Binary(binary) => {
                let left_type = self.infer_expression_type(&binary.left)?;
                let right_type = self.infer_expression_type(&binary.right)?;

                if left_type != right_type {
                    return Err(SemanticError::new(format!(
                        "binary expression type mismatch: left is {:?}, right is {:?}",
                        left_type, right_type
                    )));
                }

                match binary.operator {
                    BinaryOperator::Add
                    | BinaryOperator::Subtract
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide => match left_type {
                        TypeName::Int | TypeName::Float => Ok(left_type),
                        _ => Err(SemanticError::new(format!(
                            "operator {:?} is not supported for type {:?}",
                            binary.operator, left_type
                        ))),
                    },
                    BinaryOperator::Greater
                    | BinaryOperator::Less
                    | BinaryOperator::GreaterEqual
                    | BinaryOperator::LessEqual
                    | BinaryOperator::Equal
                    | BinaryOperator::NotEqual => match left_type {
                        TypeName::Int => Ok(TypeName::Bool),
                        _ => Err(SemanticError::new(format!(
                            "comparison operator {:?} is only supported for Int in this phase",
                            binary.operator
                        ))),
                    },
                }
            }
        }
    }

    fn ensure_type_matches(
        &self,
        expected: &TypeName,
        actual: &TypeName,
        context: &str,
    ) -> SemanticResult<()> {
        if expected == actual {
            Ok(())
        } else {
            Err(SemanticError::new(format!(
                "type mismatch in {}: expected {:?}, found {:?}",
                context, expected, actual
            )))
        }
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
    use loz_parser::parse_program;

    use super::analyze;

    fn analyze_source(source: &str) -> Result<(), String> {
        let program = parse_program(tokenize(source)).map_err(|error| error.to_string())?;
        analyze(&program).map_err(|error| error.to_string())
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
    fn rejects_undeclared_identifier() {
        let source = r#"func add() -> Int {
    return unknownVar;
}
"#;

        let error = analyze_source(source).unwrap_err();
        assert!(error.contains("undeclared identifier"));
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
}
