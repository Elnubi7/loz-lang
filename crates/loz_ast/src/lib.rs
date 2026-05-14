#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    VariableDeclaration(VariableDeclaration),
    FunctionDeclaration(FunctionDeclaration),
    If(IfStatement),
    While(WhileStatement),
    Return(ReturnStatement),
    Assignment(AssignmentStatement),
    Expression(Expression),
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariableDeclaration {
    pub is_mutable: bool,
    pub name: String,
    pub type_name: TypeName,
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDeclaration {
    pub name: String,
    pub parameters: Vec<FunctionParameter>,
    pub return_type: TypeName,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionParameter {
    pub name: String,
    pub type_name: TypeName,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStatement {
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStatement {
    pub condition: Expression,
    pub then_branch: Vec<Statement>,
    pub else_branch: Option<Vec<Statement>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhileStatement {
    pub condition: Expression,
    pub body: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentStatement {
    pub target: String,
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    IntegerLiteral(i64),
    FloatLiteral(f64),
    BooleanLiteral(bool),
    StringLiteral(String),
    Identifier(String),
    Call(CallExpression),
    Binary(BinaryExpression),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallExpression {
    pub callee: String,
    pub arguments: Vec<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BinaryExpression {
    pub left: Box<Expression>,
    pub operator: BinaryOperator,
    pub right: Box<Expression>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Greater,
    Less,
    GreaterEqual,
    LessEqual,
    Equal,
    NotEqual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeName {
    Int,
    Float,
    Bool,
    Text,
    Char,
    Void,
    Named(String),
}
