mod codegen_error;
mod llvm_generator;

pub use codegen_error::{CodegenError, CodegenResult};
pub use llvm_generator::{LlvmIrGenerator, generate_llvm_ir};
