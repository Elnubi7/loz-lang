use std::collections::{HashMap, HashSet};
use std::fmt;

use serde_json::Value as JsonValue;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MapKey {
    Int(i64),
    Bool(bool),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeMap {
    pub entries: HashMap<MapKey, RuntimeValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeSet {
    pub entries: HashSet<MapKey>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
    Json(JsonValue),
    Reference {
        target: String,
        is_mutable: bool,
    },
    Array(Vec<RuntimeValue>),
    Map(RuntimeMap),
    Set(RuntimeSet),
    Struct {
        name: String,
        fields: Vec<RuntimeValue>,
    },
    Void,
}

impl fmt::Display for RuntimeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeValue::Int(value) => write!(f, "{value}"),
            RuntimeValue::Float(value) => {
                if value.fract() == 0.0 {
                    write!(f, "{value:.1}")
                } else {
                    write!(f, "{value}")
                }
            }
            RuntimeValue::Bool(value) => write!(f, "{value}"),
            RuntimeValue::Text(value) => write!(f, "{value}"),
            RuntimeValue::Json(value) => write!(f, "{value}"),
            RuntimeValue::Reference { target, .. } => write!(f, "ref {target}"),
            RuntimeValue::Array(values) => write!(f, "Array<{} items>", values.len()),
            RuntimeValue::Map(map) => write!(f, "Map<{} entries>", map.entries.len()),
            RuntimeValue::Set(set) => write!(f, "Set<{} items>", set.entries.len()),
            RuntimeValue::Struct { name, fields } => write!(f, "{name}<{} fields>", fields.len()),
            RuntimeValue::Void => write!(f, "void"),
        }
    }
}
