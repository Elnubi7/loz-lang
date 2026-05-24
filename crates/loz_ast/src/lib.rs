use std::cmp::{max, min};
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Span {
    pub file_path: Option<String>,
    pub byte_start: usize,
    pub byte_end: usize,
    pub line: usize,
    pub column: usize,
}

impl Span {
    pub fn new(
        file_path: Option<String>,
        byte_start: usize,
        byte_end: usize,
        line: usize,
        column: usize,
    ) -> Self {
        Self {
            file_path,
            byte_start,
            byte_end,
            line,
            column,
        }
    }

    pub fn synthetic() -> Self {
        Self::default()
    }

    pub fn with_file_path(mut self, file_path: impl Into<String>) -> Self {
        self.file_path = Some(file_path.into());
        self
    }

    pub fn is_available(&self) -> bool {
        self.line > 0 && self.column > 0
    }

    pub fn is_zero_width(&self) -> bool {
        self.byte_start >= self.byte_end
    }

    pub fn cover(start: &Self, end: &Self) -> Self {
        let file_path = start.file_path.clone().or_else(|| end.file_path.clone());

        let fallback = if start.is_available() { start } else { end };

        Self {
            file_path,
            byte_start: min(start.byte_start, end.byte_start),
            byte_end: max(start.byte_end, end.byte_end),
            line: fallback.line,
            column: fallback.column,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
}

impl fmt::Display for DiagnosticSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub severity: DiagnosticSeverity,
    pub span: Option<Span>,
    pub file_path: Option<String>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            severity: DiagnosticSeverity::Error,
            span: None,
            file_path: None,
        }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.file_path = self.file_path.or_else(|| span.file_path.clone());
        self.span = Some(span);
        self
    }

    pub fn with_file_path(mut self, file_path: impl Into<String>) -> Self {
        self.file_path = Some(file_path.into());
        self
    }

    pub fn file_path(&self) -> Option<&str> {
        self.span
            .as_ref()
            .and_then(|span| span.file_path.as_deref())
            .or(self.file_path.as_deref())
    }

    pub fn render(&self) -> String {
        let source = self
            .file_path()
            .and_then(|path| fs::read_to_string(path).ok());
        self.render_with_source(source.as_deref())
    }

    pub fn render_with_source(&self, source: Option<&str>) -> String {
        let mut rendered = String::new();

        if let Some(span) = &self.span {
            if span.is_available() {
                let path = self.file_path().unwrap_or("<unknown>");
                rendered.push_str(&format!("{path}:{}:{}\n", span.line, span.column));
            } else if let Some(path) = self.file_path() {
                rendered.push_str(path);
                rendered.push('\n');
            }
        } else if let Some(path) = self.file_path() {
            rendered.push_str(path);
            rendered.push('\n');
        }

        rendered.push_str(&format!("{}: {}", self.severity, self.message));

        if let (Some(span), Some(source)) = (&self.span, source) {
            if let Some((line_text, underline)) = render_source_excerpt(span, source) {
                rendered.push_str("\n\n");
                rendered.push_str("    ");
                rendered.push_str(&line_text);
                rendered.push('\n');
                rendered.push_str("    ");
                rendered.push_str(&underline);
            }
        }

        rendered
    }
}

fn render_source_excerpt(span: &Span, source: &str) -> Option<(String, String)> {
    let line_start = source[..min(span.byte_start, source.len())]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = source[min(span.byte_start, source.len())..]
        .find('\n')
        .map(|index| min(span.byte_start, source.len()) + index)
        .unwrap_or(source.len());

    if line_start > line_end || line_start > source.len() || line_end > source.len() {
        return None;
    }

    let line_text = source[line_start..line_end].to_string();
    let column_offset = span.column.saturating_sub(1);

    let underline_width = if span.byte_end > span.byte_start && span.byte_start < source.len() {
        let end = min(span.byte_end, line_end);
        let width = source[span.byte_start..end].chars().count();
        max(width, 1)
    } else {
        1
    };

    let mut underline = String::new();
    underline.push_str(&" ".repeat(column_offset));
    underline.push_str(&"^".repeat(underline_width));

    Some((line_text, underline))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    ModuleDeclaration(ModuleDeclaration),
    ImportDeclaration(ImportDeclaration),
    VariableDeclaration(VariableDeclaration),
    FunctionDeclaration(FunctionDeclaration),
    AsyncTaskDeclaration(AsyncTaskDeclaration),
    ToolDeclaration(ToolDeclaration),
    AgentDeclaration(AgentDeclaration),
    WorkflowDeclaration(WorkflowDeclaration),
    StructDeclaration(StructDeclaration),
    SchemaDeclaration(SchemaDeclaration),
    ImplBlock(ImplBlock),
    If(IfStatement),
    While(WhileStatement),
    For(ForStatement),
    Break(Span),
    Continue(Span),
    Return(ReturnStatement),
    Assignment(AssignmentStatement),
    Expression(Expression),
}

impl Statement {
    pub fn span(&self) -> &Span {
        match self {
            Self::ModuleDeclaration(declaration) => &declaration.span,
            Self::ImportDeclaration(declaration) => &declaration.span,
            Self::VariableDeclaration(declaration) => &declaration.span,
            Self::FunctionDeclaration(declaration) => &declaration.span,
            Self::AsyncTaskDeclaration(declaration) => &declaration.span,
            Self::ToolDeclaration(declaration) => &declaration.span,
            Self::AgentDeclaration(declaration) => &declaration.span,
            Self::WorkflowDeclaration(declaration) => &declaration.span,
            Self::StructDeclaration(declaration) => &declaration.span,
            Self::SchemaDeclaration(declaration) => &declaration.span,
            Self::ImplBlock(declaration) => &declaration.span,
            Self::If(statement) => &statement.span,
            Self::While(statement) => &statement.span,
            Self::For(statement) => &statement.span,
            Self::Break(span) | Self::Continue(span) => span,
            Self::Return(statement) => &statement.span,
            Self::Assignment(statement) => &statement.span,
            Self::Expression(expression) => &expression.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDeclaration {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportDeclaration {
    pub module_name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariableDeclaration {
    pub is_mutable: bool,
    pub name: String,
    pub type_name: Option<TypeName>,
    pub value: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDeclaration {
    pub name: String,
    pub parameters: Vec<FunctionParameter>,
    pub return_type: TypeName,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolDeclaration {
    pub name: String,
    pub parameters: Vec<FunctionParameter>,
    pub return_type: TypeName,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsyncTaskDeclaration {
    pub name: String,
    pub parameters: Vec<FunctionParameter>,
    pub return_type: TypeName,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentDeclaration {
    pub name: String,
    pub model: Option<Expression>,
    pub tools: Option<Expression>,
    pub tasks: Vec<AgentTaskDeclaration>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentTaskDeclaration {
    pub name: String,
    pub parameters: Vec<FunctionParameter>,
    pub return_type: TypeName,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowDeclaration {
    pub name: String,
    pub steps: Vec<WorkflowStep>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowStep {
    pub name: String,
    pub target: WorkflowTarget,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowTarget {
    FunctionOrTool(String),
    AgentTask {
        agent_name: String,
        task_name: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionParameter {
    pub name: String,
    pub type_name: TypeName,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDeclaration {
    pub name: String,
    pub fields: Vec<StructField>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub name: String,
    pub type_name: TypeName,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaDeclaration {
    pub name: String,
    pub fields: Vec<SchemaField>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SchemaField {
    pub name: String,
    pub type_name: TypeName,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImplBlock {
    pub target_name: String,
    pub methods: Vec<FunctionDeclaration>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReturnStatement {
    pub value: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStatement {
    pub condition: Expression,
    pub then_branch: Vec<Statement>,
    pub else_branch: Option<Vec<Statement>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhileStatement {
    pub condition: Expression,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForStatement {
    pub variable_name: String,
    pub is_mutable: bool,
    pub iterable: Expression,
    pub body: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentStatement {
    pub target: AssignmentTarget,
    pub value: Expression,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssignmentTarget {
    Identifier(String),
    Dereference(DereferenceExpression),
    FieldAccess(FieldAccessExpression),
    IndexAccess(IndexAccessExpression),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expression {
    pub kind: ExpressionKind,
    pub span: Span,
}

impl Expression {
    pub fn new(kind: ExpressionKind, span: Span) -> Self {
        Self { kind, span }
    }

    #[allow(non_snake_case)]
    pub fn IntegerLiteral(value: i64) -> Self {
        Self::new(ExpressionKind::IntegerLiteral(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn FloatLiteral(value: f64) -> Self {
        Self::new(ExpressionKind::FloatLiteral(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn BooleanLiteral(value: bool) -> Self {
        Self::new(ExpressionKind::BooleanLiteral(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn StringLiteral(value: String) -> Self {
        Self::new(ExpressionKind::StringLiteral(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn Identifier(value: String) -> Self {
        Self::new(ExpressionKind::Identifier(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn Await(value: AwaitExpression) -> Self {
        Self::new(ExpressionKind::Await(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn Reference(value: ReferenceExpression) -> Self {
        Self::new(ExpressionKind::Reference(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn Dereference(value: DereferenceExpression) -> Self {
        Self::new(ExpressionKind::Dereference(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn Cast(value: CastExpression) -> Self {
        Self::new(ExpressionKind::Cast(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn FieldAccess(value: FieldAccessExpression) -> Self {
        Self::new(ExpressionKind::FieldAccess(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn MethodCall(value: MethodCallExpression) -> Self {
        Self::new(ExpressionKind::MethodCall(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn ArrayLiteral(value: ArrayLiteralExpression) -> Self {
        Self::new(ExpressionKind::ArrayLiteral(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn IndexAccess(value: IndexAccessExpression) -> Self {
        Self::new(ExpressionKind::IndexAccess(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn Call(value: CallExpression) -> Self {
        Self::new(ExpressionKind::Call(value), Span::default())
    }

    #[allow(non_snake_case)]
    pub fn Binary(value: BinaryExpression) -> Self {
        Self::new(ExpressionKind::Binary(value), Span::default())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExpressionKind {
    IntegerLiteral(i64),
    FloatLiteral(f64),
    BooleanLiteral(bool),
    StringLiteral(String),
    Identifier(String),
    Await(AwaitExpression),
    Reference(ReferenceExpression),
    Dereference(DereferenceExpression),
    Cast(CastExpression),
    FieldAccess(FieldAccessExpression),
    MethodCall(MethodCallExpression),
    ArrayLiteral(ArrayLiteralExpression),
    IndexAccess(IndexAccessExpression),
    Call(CallExpression),
    Binary(BinaryExpression),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallExpression {
    pub callee: String,
    pub arguments: Vec<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AwaitExpression {
    pub expression: Box<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceExpression {
    pub target_name: String,
    pub is_mutable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DereferenceExpression {
    pub value: Box<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CastExpression {
    pub target_type: TypeName,
    pub value: Box<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldAccessExpression {
    pub base_name: String,
    pub field_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MethodCallExpression {
    pub base_name: String,
    pub method_name: String,
    pub arguments: Vec<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArrayLiteralExpression {
    pub elements: Vec<Expression>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexAccessExpression {
    pub base_name: String,
    pub index: Box<Expression>,
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
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Text,
    Json,
    Char,
    Void,
    Reference {
        inner: Box<TypeName>,
        is_mutable: bool,
    },
    Array(Box<TypeName>, Option<usize>),
    Map(Box<TypeName>, Box<TypeName>),
    Set(Box<TypeName>),
    Named(String),
}

impl TypeName {
    pub fn is_signed_integer(&self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32 | Self::I64)
    }

    pub fn is_unsigned_integer(&self) -> bool {
        matches!(self, Self::U8 | Self::U16 | Self::U32 | Self::U64)
    }

    pub fn is_integer(&self) -> bool {
        self.is_signed_integer() || self.is_unsigned_integer()
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    pub fn is_numeric(&self) -> bool {
        self.is_integer() || self.is_float()
    }

    pub fn is_reference(&self) -> bool {
        matches!(self, Self::Reference { .. })
    }

    pub fn bit_width(&self) -> Option<u32> {
        match self {
            Self::I8 | Self::U8 => Some(8),
            Self::I16 | Self::U16 => Some(16),
            Self::I32 | Self::U32 | Self::F32 => Some(32),
            Self::I64 | Self::U64 | Self::F64 => Some(64),
            _ => None,
        }
    }
}

pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
