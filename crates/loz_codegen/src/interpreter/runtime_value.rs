use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    Void,
}

impl fmt::Display for RuntimeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeValue::Int(value) => write!(f, "{value}"),
            RuntimeValue::Float(value) => write!(f, "{value}"),
            RuntimeValue::Bool(value) => write!(f, "{value}"),
            RuntimeValue::Text(value) => write!(f, "{value}"),
            RuntimeValue::Void => write!(f, "void"),
        }
    }
}
