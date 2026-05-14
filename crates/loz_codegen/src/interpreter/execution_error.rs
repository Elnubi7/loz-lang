use std::fmt;

pub type ExecutionResult<T> = Result<T, ExecutionError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionError {
    pub message: String,
}

impl ExecutionError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "execution error: {}", self.message)
    }
}

impl std::error::Error for ExecutionError {}
