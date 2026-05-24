use std::collections::HashMap;

use super::{ExecutionError, ExecutionResult, RuntimeValue};

#[derive(Debug, Clone)]
struct RuntimeBinding {
    value: RuntimeValue,
    is_mutable: bool,
}

#[derive(Debug, Default)]
pub struct ExecutionEnvironment {
    scopes: Vec<HashMap<String, RuntimeBinding>>,
}

impl ExecutionEnvironment {
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

    pub fn define(
        &mut self,
        name: String,
        value: RuntimeValue,
        is_mutable: bool,
    ) -> ExecutionResult<()> {
        let current_scope = self
            .scopes
            .last_mut()
            .expect("execution environment always has at least one scope");

        if current_scope.contains_key(&name) {
            return Err(ExecutionError::new(format!(
                "duplicate runtime binding '{}'",
                name
            )));
        }

        current_scope.insert(name, RuntimeBinding { value, is_mutable });
        Ok(())
    }

    pub fn assign(&mut self, name: &str, value: RuntimeValue) -> ExecutionResult<()> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(binding) = scope.get_mut(name) {
                if !binding.is_mutable {
                    return Err(ExecutionError::new(format!(
                        "cannot assign to immutable value '{}'",
                        name
                    )));
                }

                binding.value = value;
                return Ok(());
            }
        }

        Err(ExecutionError::new(format!(
            "assignment to unknown runtime binding '{}'",
            name
        )))
    }

    pub fn get_binding(&self, name: &str) -> ExecutionResult<(RuntimeValue, bool)> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .map(|binding| (binding.value.clone(), binding.is_mutable))
            .ok_or_else(|| ExecutionError::new(format!("unknown runtime binding '{}'", name)))
    }

    pub fn get(&self, name: &str) -> ExecutionResult<RuntimeValue> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name))
            .map(|binding| binding.value.clone())
            .ok_or_else(|| ExecutionError::new(format!("unknown runtime binding '{}'", name)))
    }
}
