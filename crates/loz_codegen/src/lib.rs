pub mod interpreter;
pub mod llvm;

pub use interpreter::{
    ExecutionEnvironment, ExecutionError, ExecutionResult, Interpreter, RuntimeValue, execute,
};
pub use llvm::{CodegenError, CodegenResult, LlvmIrGenerator, generate_llvm_ir};

#[cfg(test)]
mod tests {
    use loz_lexer::tokenize;
    use loz_parser::parse_program;
    use loz_semantic::analyze;

    use crate::{Interpreter, RuntimeValue, generate_llvm_ir};

    fn prepare_program(source: &str) -> loz_ast::Program {
        let program = parse_program(tokenize(source)).unwrap();
        analyze(&program).unwrap();
        program
    }

    #[test]
    fn executes_main_and_returns_void() {
        let source = r#"const x: Int = 10;

func add(a: Int, b: Int) -> Int {
    return a + b;
}

func main() -> Void {
    print(add(x, 20));
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Void);
    }

    #[test]
    fn executes_assignments_and_returns_function_value() {
        let source = r#"mut x: Int = 10;

func main() -> Int {
    x = x + 5;
    return x;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(15));
    }

    #[test]
    fn generates_llvm_ir_for_int_main() {
        let source = r#"func main() -> Int {
    mut x: Int = 0;
    while x < 10 {
        x = x + 1;
    }
    return x;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define i64 @main()"));
        assert!(ir.contains("alloca i64"));
        assert!(ir.contains("store i64 0"));
        assert!(ir.contains("load i64"));
        assert!(ir.contains("icmp slt i64"));
        assert!(ir.contains("br i1"));
        assert!(ir.contains("loopcond:"));
        assert!(ir.contains("loopbody:"));
        assert!(ir.contains("loopexit:"));
    }

    #[test]
    fn generates_llvm_ir_for_native_print() {
        let source = r#"func main() -> Int {
    print(30);
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare i32 @printf(ptr, ...)"));
        assert!(ir.contains("@printf_fmt = private unnamed_addr constant"));
        assert!(ir.contains("%ld\\0A\\00"));
        assert!(ir.contains("call i32 (ptr, ...) @printf"));
        assert!(ir.contains("ret i64 0"));
    }

    #[test]
    fn keeps_global_const_and_function_call_support_in_llvm_ir() {
        let source = r#"const x: Int = 10;

func add(a: Int, b: Int) -> Int {
    return a + b;
}

func main() -> Int {
    return add(x, 20);
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("@x = constant i64 10"));
        assert!(ir.contains("define i64 @add(i64 %0, i64 %1)"));
        assert!(ir.contains("call i64 @add"));
    }

    #[test]
    fn rejects_unsupported_top_level_for_llvm_ir() {
        let source = r#"10;

func main() -> Int {
    return 30;
}
"#;

        let program = prepare_program(source);
        let error = generate_llvm_ir(&program).unwrap_err();

        assert!(
            error
                .message
                .contains("top-level const Int globals and function declarations")
        );
    }

    #[test]
    fn rejects_mutable_global_for_llvm_ir() {
        let source = r#"mut x: Int = 10;

func main() -> Int {
    return 30;
}
"#;

        let program = prepare_program(source);
        let error = generate_llvm_ir(&program).unwrap_err();

        assert!(error.message.contains("mutable global"));
    }
}
