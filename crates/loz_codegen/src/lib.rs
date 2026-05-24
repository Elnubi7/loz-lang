pub mod interpreter;
pub mod llvm;

pub use interpreter::{
    ExecutionEnvironment, ExecutionError, ExecutionResult, Interpreter, RuntimeValue,
    WorkflowStepOutcome, execute,
};
pub use llvm::{CodegenError, CodegenResult, LlvmIrGenerator, generate_llvm_ir};

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use loz_lexer::tokenize;
    use loz_parser::parse_program;
    use loz_semantic::analyze;

    use crate::{Interpreter, RuntimeValue, WorkflowStepOutcome, generate_llvm_ir};

    static TEST_MODULE_COUNTER: AtomicU64 = AtomicU64::new(0);
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn prepare_program(source: &str) -> loz_ast::Program {
        let program = parse_program(tokenize(source).unwrap()).unwrap();
        analyze(&program).unwrap();
        program
    }

    fn python_executable() -> String {
        env::var("LOZ_PYTHON_PATH").unwrap_or_else(|_| "python3".to_string())
    }

    fn python_is_available() -> bool {
        Command::new(python_executable())
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn write_test_module(module_source: &str) -> (String, PathBuf) {
        let current_dir = env::current_dir().unwrap();
        let unique = format!(
            "{}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            TEST_MODULE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let module_name = format!("loz_codegen_python_test_{unique}");
        let module_path = current_dir.join(format!("{module_name}.py"));
        fs::write(&module_path, module_source).unwrap();
        (module_name, module_path)
    }

    fn with_env_vars<T>(updates: &[(&str, Option<&str>)], test: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let previous = updates
            .iter()
            .map(|(key, _)| ((*key).to_string(), env::var(key).ok()))
            .collect::<Vec<_>>();

        for (key, value) in updates {
            unsafe {
                match value {
                    Some(value) => env::set_var(key, value),
                    None => env::remove_var(key),
                }
            }
        }

        let result = test();

        for (key, value) in previous {
            unsafe {
                match value {
                    Some(value) => env::set_var(&key, value),
                    None => env::remove_var(&key),
                }
            }
        }

        result
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
    fn executes_main_with_agent_declaration_present() {
        let source = r#"agent SupportAgent {
    model: "mock";
    tools: [];

    task answer(question: Text) -> Text {
        return llm.ask(question);
    }
}

func main() -> i32 {
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn executes_workflow_sequentially() {
        let source = r#"func prepare() -> Text {
    return "prepared";
}

tool get_data() -> Json {
    return json.parse("{\"name\":\"Ahmed\"}");
}

workflow Onboarding {
    step prepare;
    step get_data;
}

func main() -> i32 {
    return 0;
}
"#;

        let program = prepare_program(source);
        let outcomes = Interpreter::new()
            .execute_workflow(&program, "Onboarding")
            .unwrap();

        assert_eq!(
            outcomes,
            vec![
                WorkflowStepOutcome {
                    step_name: "prepare".to_string(),
                    result: Some(RuntimeValue::Text("prepared".to_string())),
                },
                WorkflowStepOutcome {
                    step_name: "get_data".to_string(),
                    result: Some(RuntimeValue::Json(serde_json::json!({"name": "Ahmed"}))),
                },
            ]
        );
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
    fn executes_mutable_struct_field_assignment_and_returns_value() {
        let source = r#"struct Point {
    x: Int,
    y: Int
}

func main() -> Int {
    mut p: Point = Point(10, 20);
    p.x = 50;
    return p.x;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(50));
    }

    #[test]
    fn executes_struct_construction_and_returns_int() {
        let source = r#"struct Point {
    x: Int,
    y: Int
}

func main() -> Int {
    const p: Point = Point(10, 20);
    print("Struct instance created");
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn executes_struct_field_access_and_returns_value() {
        let source = r#"struct Point {
    x: Int,
    y: Int
}

func main() -> Int {
    const p: Point = Point(10, 20);
    return p.x;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(10));
    }

    #[test]
    fn executes_struct_parameters_and_returns() {
        let source = r#"struct Point {
    x: Int,
    y: Int
}

func make_point() -> Point {
    return Point(10, 20);
}

func sum(p: Point) -> Int {
    return p.x + p.y;
}

func main() -> Int {
    const p: Point = make_point();
    return sum(p);
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(30));
    }

    #[test]
    fn executes_struct_method_call() {
        let source = r#"struct Point {
    x: i32,
    y: i32
}

impl Point {
    func sum(self) -> i32 {
        return self.x + self.y;
    }
}

func main() -> i32 {
    const p: Point = Point(10, 20);
    return p.sum();
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(30));
    }

    #[test]
    fn executes_array_literal_and_index_access() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20, 30];
    return nums[1];
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(20));
    }

    #[test]
    fn executes_mutable_array_element_assignment() {
        let source = r#"func main() -> Int {
    mut nums: Array<Int> = [10, 20, 30];
    nums[1] = 99;
    return nums[1];
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(99));
    }

    #[test]
    fn executes_array_len_push_and_pop() {
        let source = r#"func main() -> Int {
    mut nums: Array<Int> = [10, 20];
    nums.push(30);
    print(nums.len());
    return nums.pop();
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(30));
    }

    #[test]
    fn executes_tool_returning_json() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

func main() -> i32 {
    const user: Json = get_user(1);
    print(json.get_text(user, "name"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn executes_tool_using_schema_require() {
        let source = r#"schema User {
    id: i32,
    name: Text
}

tool get_user(id: i32) -> Json {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
    return schema.require("User", user);
}

func main() -> i32 {
    const user: Json = get_user(1);
    print(schema.validate("User", user));
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn executes_tool_calling_another_tool() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

tool get_name(id: i32) -> Text {
    const user: Json = get_user(id);
    return json.get_text(user, "name");
}

func main() -> i32 {
    print(get_name(1));
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn function_can_call_tool() {
        let source = r#"tool score_user(user: Json) -> i32 {
    return json.get_i32(user, "id");
}

func load_user() -> Json {
    return json.parse("{\"id\":7,\"name\":\"Ahmed\"}");
}

func main() -> i32 {
    return score_user(load_user());
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(7));
    }

    #[test]
    fn executes_python_call_and_returns_json() {
        if !python_is_available() {
            return;
        }

        let (module_name, module_path) = write_test_module(
            r#"def analyze_text(payload):
    text = payload["text"]
    return {"length": len(text), "label": "ok"}
"#,
        );

        let source = format!(
            r#"func main() -> i32 {{
    const payload: Json = json.parse("{{\"text\":\"hello\"}}");
    const result: Json = python.call("{module_name}.analyze_text", payload);
    return json.get_i32(result, "length");
}}
"#
        );

        let program = prepare_program(&source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(5));
        let _ = fs::remove_file(module_path);
    }

    #[test]
    fn surfaces_python_bridge_errors_in_interpreter() {
        if !python_is_available() {
            return;
        }

        let (module_name, module_path) = write_test_module(
            r#"def fail(payload):
    raise RuntimeError("boom")
"#,
        );

        let source = format!(
            r#"func main() -> i32 {{
    const payload: Json = json.parse("{{\"text\":\"hello\"}}");
    const result: Json = python.call("{module_name}.fail", payload);
    return json.get_i32(result, "length");
}}
"#
        );

        let program = prepare_program(&source);
        let error = Interpreter::new().execute_program(&program).unwrap_err();

        assert!(error.to_string().contains("python.call() failed"));
        let _ = fs::remove_file(module_path);
    }

    #[test]
    fn executes_llm_ask_with_mock_provider() {
        with_env_vars(
            &[
                ("LOZ_LLM_PROVIDER", Some("mock")),
                ("LOZ_LLM_MOCK_RESPONSE", None),
                ("GITHUB_TOKEN", None),
                ("LOZ_MODEL", None),
            ],
            || {
                let source = r#"func main() -> Text {
    return llm.ask("hello");
}
"#;

                let program = prepare_program(source);
                let result = Interpreter::new().execute_program(&program).unwrap();

                assert_eq!(result, RuntimeValue::Text("[mock] hello".to_string()));
            },
        );
    }

    #[test]
    fn rejects_unknown_llm_provider_in_interpreter() {
        with_env_vars(
            &[
                ("LOZ_LLM_PROVIDER", Some("bad-provider")),
                ("LOZ_LLM_MOCK_RESPONSE", None),
                ("GITHUB_TOKEN", None),
                ("LOZ_MODEL", None),
            ],
            || {
                let source = r#"func main() -> Text {
    return llm.ask("hello");
}
"#;

                let program = prepare_program(source);
                let error = Interpreter::new().execute_program(&program).unwrap_err();

                assert!(error.to_string().contains("unknown LLM provider"));
            },
        );
    }

    #[test]
    fn rejects_missing_github_token_in_interpreter() {
        with_env_vars(
            &[
                ("LOZ_LLM_PROVIDER", Some("github")),
                ("LOZ_MODEL", Some("openai/gpt-4.1-mini")),
                ("GITHUB_TOKEN", None),
                ("LOZ_LLM_MOCK_RESPONSE", None),
            ],
            || {
                let source = r#"func main() -> Text {
    return llm.ask("hello");
}
"#;

                let program = prepare_program(source);
                let error = Interpreter::new().execute_program(&program).unwrap_err();

                assert!(error.to_string().contains("GITHUB_TOKEN is required"));
            },
        );
    }

    #[test]
    fn executes_map_construction_and_methods() {
        let source = r#"func main() -> i32 {
    mut scores: Map<Text, i32> = Map();
    scores.insert("ahmed", 100);
    print(scores.contains("ahmed"));
    return scores.get("ahmed");
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(100));
    }

    #[test]
    fn executes_set_construction_and_methods() {
        let source = r#"func main() -> i32 {
    mut ids: Set<i32> = Set();
    ids.add(10);
    ids.add(20);
    print(ids.contains(10));
    print(ids.contains(30));
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn executes_all_io_read_methods() {
        let source = r#"func main() -> i32 {
    const name: Text = io.read_line();
    const tiny_signed: i8 = io.read_i8();
    const small_signed: i16 = io.read_i16();
    const age: i32 = io.read_i32();
    const big_signed: i64 = io.read_i64();
    const tiny_unsigned: u8 = io.read_u8();
    const small_unsigned: u16 = io.read_u16();
    const medium_unsigned: u32 = io.read_u32();
    const big_unsigned: u64 = io.read_u64();
    const ratio32: f32 = io.read_f32();
    const score: f64 = io.read_f64();
    const active: Bool = io.read_bool();
    print(name);
    print(tiny_signed);
    print(small_signed);
    print(age);
    print(big_signed);
    print(tiny_unsigned);
    print(small_unsigned);
    print(medium_unsigned);
    print(big_unsigned);
    print(ratio32);
    print(score);
    print(active);
    return age;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::with_input_lines([
            "Ahmed".to_string(),
            "-5".to_string(),
            "-300".to_string(),
            "42".to_string(),
            "-9000".to_string(),
            "250".to_string(),
            "65000".to_string(),
            "4000000000".to_string(),
            "99".to_string(),
            "3.25".to_string(),
            "98.5".to_string(),
            "yes".to_string(),
        ])
        .execute_program(&program)
        .unwrap();

        assert_eq!(result, RuntimeValue::Int(42));
    }

    #[test]
    fn executes_json_parse_and_accessors() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\",\"active\":true,\"score\":98.5}");
    print(json.get_text(user, "name"));
    print(json.get_i32(user, "id"));
    print(json.get_i64(user, "id"));
    print(json.get_f64(user, "score"));
    print(json.get_bool(user, "active"));
    print(json.has(user, "email"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn executes_json_stringify() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
    print(json.stringify(user));
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn executes_schema_validate_and_require() {
        let source = r#"schema User {
    id: i32,
    name: Text,
    active: Bool
}

func main() -> i32 {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\",\"active\":true}");
    print(schema.validate("User", user));
    const checked: Json = schema.require("User", user);
    print(json.get_text(checked, "name"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn returns_false_for_invalid_schema_validation() {
        let source = r#"schema User {
    id: i32,
    name: Text
}

func main() -> Bool {
    const user: Json = json.parse("{\"id\":\"wrong\",\"name\":\"Ahmed\"}");
    return schema.validate("User", user);
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Bool(false));
    }

    #[test]
    fn rejects_invalid_schema_require_at_runtime() {
        let source = r#"schema User {
    id: i32,
    name: Text
}

func main() -> Json {
    const user: Json = json.parse("{\"id\":\"wrong\",\"name\":\"Ahmed\"}");
    return schema.require("User", user);
}
"#;

        let program = prepare_program(source);
        let error = Interpreter::new().execute_program(&program).unwrap_err();

        assert!(error.to_string().contains("schema.require() failed"));
    }

    #[test]
    fn rejects_invalid_io_read_i32_input_at_runtime() {
        let source = r#"func main() -> i32 {
    return io.read_i32();
}
"#;

        let program = prepare_program(source);
        let error = Interpreter::with_input_lines(["not-a-number".to_string()])
            .execute_program(&program)
            .unwrap_err();

        assert!(error.to_string().contains("failed to parse io.read_i32()"));
    }

    #[test]
    fn rejects_invalid_json_at_runtime() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{");
    return 0;
}
"#;

        let program = prepare_program(source);
        let error = Interpreter::new().execute_program(&program).unwrap_err();

        assert!(error.to_string().contains("invalid JSON"));
    }

    #[test]
    fn rejects_missing_json_key_at_runtime() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{\"id\":1}");
    return json.get_i32(user, "missing");
}
"#;

        let program = prepare_program(source);
        let error = Interpreter::new().execute_program(&program).unwrap_err();

        assert!(error.to_string().contains("missing key"));
    }

    #[test]
    fn rejects_wrong_json_value_type_at_runtime() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{\"name\":\"Ahmed\"}");
    return json.get_i32(user, "name");
}
"#;

        let program = prepare_program(source);
        let error = Interpreter::new().execute_program(&program).unwrap_err();

        assert!(error.to_string().contains("wrong type"));
    }

    #[test]
    fn rejects_invalid_io_read_bool_input_at_runtime() {
        let source = r#"func main() -> Bool {
    return io.read_bool();
}
"#;

        let program = prepare_program(source);
        let error = Interpreter::with_input_lines(["maybe".to_string()])
            .execute_program(&program)
            .unwrap_err();

        assert!(error.to_string().contains("failed to parse io.read_bool()"));
    }

    #[test]
    fn rejects_out_of_range_io_read_u8_input_at_runtime() {
        let source = r#"func main() -> u8 {
    return io.read_u8();
}
"#;

        let program = prepare_program(source);
        let error = Interpreter::with_input_lines(["999".to_string()])
            .execute_program(&program)
            .unwrap_err();

        assert!(error.to_string().contains("failed to parse io.read_u8()"));
    }

    #[test]
    fn rejects_map_get_for_missing_key_at_runtime() {
        let source = r#"func main() -> i32 {
    mut scores: Map<Text, i32> = Map();
    return scores.get("missing");
}
"#;

        let program = prepare_program(source);
        let error = Interpreter::new().execute_program(&program).unwrap_err();

        assert!(error.to_string().contains("does not contain key"));
    }

    #[test]
    fn rejects_pop_from_empty_array_at_runtime() {
        let source = r#"func main() -> Int {
    mut nums: Array<Int> = [10];
    nums.pop();
    return nums.pop();
}
"#;

        let program = prepare_program(source);
        let error = Interpreter::new().execute_program(&program).unwrap_err();

        assert!(error.to_string().contains("empty array"));
    }

    #[test]
    fn executes_for_loop_with_mutable_loop_variable() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20, 30];
    mut sum: Int = 0;
    for mut x in nums {
        x = x + 1;
        sum = sum + x;
    }
    return sum;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(63));
    }

    #[test]
    fn executes_break_and_continue_in_loops() {
        let source = r#"func main() -> i32 {
    const nums: Array<i32> = [10, 20, 30];
    mut sum: i32 = 0;

    for x in nums {
        if x == 20 {
            continue;
        }

        sum = sum + x;

        if x == 30 {
            break;
        }
    }

    mut y: i32 = 0;
    while y < 5 {
        y = y + 1;

        if y == 2 {
            continue;
        }

        if y == 3 {
            break;
        }
    }

    return sum + y;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(43));
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
        assert!(ir.contains("@printf_fmt_int = private unnamed_addr constant"));
        assert!(ir.contains("%lld\\0A\\00"));
        assert!(ir.contains("call i32 (ptr, ...) @printf"));
        assert!(ir.contains("ret i64 0"));
    }

    #[test]
    fn generates_llvm_ir_for_multiple_numeric_types() {
        let source = r#"func main() -> i32 {
    const x: i32 = 10;
    const y: i32 = 20;
    const pi: f64 = 3.14;
    const ok: Bool = true;
    print(x + y);
    print(pi);
    print(ok);
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define i32 @main()"));
        assert!(ir.contains("alloca i32"));
        assert!(ir.contains("add i32"));
        assert!(ir.contains("double 3.140000e+00"));
        assert!(ir.contains("%.1f\\0A\\00"));
        assert!(ir.contains("%g\\0A\\00"));
        assert!(ir.contains("true\\00"));
    }

    #[test]
    fn executes_explicit_casts() {
        let source = r#"func main() -> i32 {
    const x: i32 = 10;
    const y: f64 = as<f64>(x);
    const z: Bool = as<Bool>(1);
    print(y);
    print(z);
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn generates_llvm_ir_for_explicit_casts() {
        let source = r#"func main() -> i32 {
    const x: i32 = 10;
    const y: f64 = as<f64>(x);
    const z: Bool = as<Bool>(1);
    print(y);
    print(z);
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("sitofp i32"));
        assert!(ir.contains("store i1 true"));
        assert!(ir.contains("call i32 (ptr, ...) @printf"));
        assert!(ir.contains("call i32 @puts"));
    }

    #[test]
    fn executes_references_and_dereference_assignment() {
        let source = r#"func main() -> i32 {
    mut x: i32 = 10;
    const rx = ref x;
    print(*rx);
    const ry = mut ref x;
    *ry = 50;
    return x;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(50));
    }

    #[test]
    fn generates_llvm_ir_for_references() {
        let source = r#"func main() -> i32 {
    mut x: i32 = 10;
    const rx = ref x;
    print(*rx);
    const ry = mut ref x;
    *ry = 50;
    return x;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("alloca ptr"));
        assert!(ir.contains("store ptr %x, ptr %rx"));
        assert!(ir.contains("load ptr, ptr %rx"));
        assert!(ir.contains("store i32 50, ptr"));
    }

    #[test]
    fn generates_llvm_ir_for_native_text_print() {
        let source = r#"func main() -> Int {
    print("Hello Loz");
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare i32 @puts(ptr)"));
        assert!(ir.contains("Hello Loz\\00"));
        assert!(ir.contains("call i32 @puts"));
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
    fn allows_top_level_struct_declarations_in_llvm_ir() {
        let source = r#"struct Point {
    x: Int,
    y: Int,
}

func main() -> Int {
    print("Struct phase started");
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define i64 @main()"));
        assert!(ir.contains("call i32 @puts"));
    }

    #[test]
    fn allows_struct_construction_in_llvm_ir() {
        let source = r#"struct Point {
    x: Int,
    y: Int
}

func main() -> Int {
    const p: Point = Point(10, 20);
    print("Struct instance created");
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("alloca { i64, i64 }"));
        assert!(ir.contains("store { i64, i64 } { i64 10, i64 20 }, ptr %p"));
        assert!(ir.contains("call i32 @puts"));
    }

    #[test]
    fn allows_struct_field_access_in_llvm_ir() {
        let source = r#"struct Point {
    x: Int,
    y: Int
}

func main() -> Int {
    const p: Point = Point(10, 20);
    print(p.x);
    return p.y;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("extractvalue { i64, i64 }"));
        assert!(ir.contains("call i32 (ptr, ...) @printf"));
        assert!(ir.contains("ret i64 %extract_p_y"));
    }

    #[test]
    fn allows_mutable_struct_field_assignment_in_llvm_ir() {
        let source = r#"struct Point {
    x: Int,
    y: Int
}

func main() -> Int {
    mut p: Point = Point(10, 20);
    p.x = 50;
    print(p.x);
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("insertvalue { i64, i64 }"));
        assert!(ir.contains("store { i64, i64 }"));
        assert!(ir.contains("call i32 (ptr, ...) @printf"));
    }

    #[test]
    fn allows_struct_parameters_and_returns_in_llvm_ir() {
        let source = r#"struct Point {
    x: Int,
    y: Int
}

func make_point() -> Point {
    return Point(10, 20);
}

func sum(p: Point) -> Int {
    return p.x + p.y;
}

func main() -> Int {
    const p: Point = make_point();
    return sum(p);
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define { i64, i64 } @make_point()"));
        assert!(ir.contains("define i64 @sum({ i64, i64 } %0)"));
        assert!(ir.contains("call { i64, i64 } @make_point"));
        assert!(ir.contains("call i64 @sum"));
        assert!(ir.contains("extractvalue { i64, i64 } %0, 0"));
        assert!(ir.contains("extractvalue { i64, i64 } %0, 1"));
    }

    #[test]
    fn allows_struct_methods_in_llvm_ir() {
        let source = r#"struct Point {
    x: i32,
    y: i32
}

impl Point {
    func sum(self) -> i32 {
        return self.x + self.y;
    }
}

func main() -> i32 {
    const p: Point = Point(10, 20);
    print(p.sum());
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define i32 @__loz_method__Point__sum({ i32, i32 } %0)"));
        assert!(ir.contains("call i32 @__loz_method__Point__sum"));
        assert!(ir.contains("extractvalue { i32, i32 } %0, 0"));
        assert!(ir.contains("extractvalue { i32, i32 } %0, 1"));
    }

    #[test]
    fn allows_array_literal_and_index_access_in_llvm_ir() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20, 30];
    print(nums[1]);
    return nums[1];
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("alloca [3 x i64]"));
        assert!(ir.contains("extractvalue [3 x i64]"));
        assert!(ir.contains("call i32 (ptr, ...) @printf"));
        assert!(ir.contains("ret i64 %extract_nums_1"));
    }

    #[test]
    fn allows_mutable_array_element_assignment_in_llvm_ir() {
        let source = r#"func main() -> Int {
    mut nums: Array<Int> = [10, 20, 30];
    nums[1] = 99;
    print(nums[1]);
    return nums[1];
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("insertvalue [3 x i64]"));
        assert!(ir.contains("store [3 x i64]"));
        assert!(ir.contains("extractvalue [3 x i64]"));
        assert!(ir.contains("call i32 (ptr, ...) @printf"));
    }

    #[test]
    fn allows_array_len_in_llvm_ir() {
        let source = r#"func main() -> u64 {
    const nums: Array<Int> = [10, 20, 30];
    print(nums.len());
    return nums.len();
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define i64 @main()"));
        assert!(ir.contains("store [3 x i64]"));
        assert!(ir.contains("call i32 (ptr, ...) @printf"));
        assert!(ir.contains("ret i64 3"));
    }

    #[test]
    fn rejects_array_push_in_llvm_ir() {
        let source = r#"func main() -> Int {
    mut nums: Array<Int> = [10, 20];
    nums.push(30);
    return 0;
}
"#;

        let program = prepare_program(source);
        let error = generate_llvm_ir(&program).unwrap_err();

        assert!(error.message.contains("Array.push()"));
    }

    #[test]
    fn rejects_map_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    mut scores: Map<Text, i32> = Map();
    scores.insert("ahmed", 100);
    return scores.get("ahmed");
}
"#;

        let program = prepare_program(source);
        let error = generate_llvm_ir(&program).unwrap_err();

        assert!(error.message.contains("Map"));
    }

    #[test]
    fn rejects_set_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    mut ids: Set<i32> = Set();
    ids.add(10);
    return 0;
}
"#;

        let program = prepare_program(source);
        let error = generate_llvm_ir(&program).unwrap_err();

        assert!(error.message.contains("Set"));
    }

    #[test]
    fn allows_io_read_i32_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    const age: i32 = io.read_i32();
    print(age);
    return age;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare i32 @scanf(ptr, ...)"));
        assert!(ir.contains("alloca i32"));
        assert!(ir.contains("c\"%d\\00\""));
        assert!(ir.contains("call i32 (ptr, ...) @scanf"));
    }

    #[test]
    fn allows_io_read_i64_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    const age: i64 = io.read_i64();
    print(age);
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare i32 @scanf(ptr, ...)"));
        assert!(ir.contains("alloca i64"));
        assert!(ir.contains("c\"%lld\\00\""));
        assert!(ir.contains("call i32 (ptr, ...) @scanf"));
    }

    #[test]
    fn allows_io_read_f64_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    const score: f64 = io.read_f64();
    print(score);
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare i32 @scanf(ptr, ...)"));
        assert!(ir.contains("alloca double"));
        assert!(ir.contains("c\"%lf\\00\""));
        assert!(ir.contains("call i32 (ptr, ...) @scanf"));
    }

    #[test]
    fn allows_io_read_bool_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    const active: Bool = io.read_bool();
    print(active);
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare i32 @scanf(ptr, ...)"));
        assert!(ir.contains("declare i32 @strcmp(ptr, ptr)"));
        assert!(ir.contains("c\"%15s\\00\""));
        assert!(ir.contains("call i32 @strcmp"));
    }

    #[test]
    fn allows_io_read_line_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    const name: Text = io.read_line();
    print(name);
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("@loz_stdin_buffer = internal global [1024 x i8] zeroinitializer"));
        assert!(ir.contains("declare ptr @fgets(ptr, i32, ptr)"));
        assert!(ir.contains("declare i64 @strcspn(ptr, ptr)"));
        assert!(ir.contains("load ptr, ptr @stdin"));
        assert!(ir.contains("call ptr @fgets"));
        assert!(ir.contains("call i64 @strcspn"));
        assert!(ir.contains("alloca ptr"));
        assert!(ir.contains("call i32 @puts(ptr"));
    }

    #[test]
    fn allows_text_parameter_and_return_in_llvm_ir() {
        let source = r#"func echo_name(name: Text) -> Text {
    return name;
}

func main() -> i32 {
    const name: Text = io.read_line();
    print(echo_name(name));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define ptr @echo_name(ptr %0)"));
        assert!(ir.contains("ret ptr %0"));
        assert!(ir.contains("call ptr @echo_name"));
        assert!(ir.contains("call i32 @puts(ptr"));
    }

    #[test]
    fn allows_json_parse_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{\"id\":1}");
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare ptr @loz_json_parse(ptr)"));
        assert!(ir.contains("alloca ptr"));
        assert!(ir.contains("call ptr @loz_json_parse"));
    }

    #[test]
    fn allows_json_getters_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
    print(json.get_text(user, "name"));
    return json.get_i32(user, "id");
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare ptr @loz_json_get_text(ptr, ptr)"));
        assert!(ir.contains("declare i32 @loz_json_get_i32(ptr, ptr)"));
        assert!(ir.contains("call ptr @loz_json_get_text"));
        assert!(ir.contains("call i32 @loz_json_get_i32"));
        assert!(ir.contains("call i32 @puts(ptr"));
    }

    #[test]
    fn allows_json_has_and_function_parameter_return_in_llvm_ir() {
        let source = r#"func get_user() -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

func get_name(user: Json) -> Text {
    return json.get_text(user, "name");
}

func main() -> i32 {
    const user: Json = get_user();
    print(get_name(user));
    print(json.has(user, "email"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define ptr @get_user()"));
        assert!(ir.contains("define ptr @get_name(ptr %0)"));
        assert!(ir.contains("call ptr @get_user"));
        assert!(ir.contains("call ptr @get_name"));
        assert!(ir.contains("declare i1 @loz_json_has(ptr, ptr)"));
        assert!(ir.contains("call i1 @loz_json_has"));
    }

    #[test]
    fn allows_schema_validate_in_llvm_ir() {
        let source = r#"schema User {
    id: i32,
    name: Text,
    active: Bool
}

func main() -> i32 {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\",\"active\":true}");
    print(schema.validate("User", user));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare i1 @loz_schema_validate(ptr, ptr)"));
        assert!(ir.contains("@schema_User_descriptor = private constant"));
        assert!(ir.contains("User|id:i32;name:Text;active:Bool\\00"));
        assert!(ir.contains("call i1 @loz_schema_validate"));
    }

    #[test]
    fn allows_schema_require_in_llvm_ir() {
        let source = r#"schema User {
    id: i32,
    name: Text
}

func main() -> i32 {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
    const checked: Json = schema.require("User", user);
    print(json.get_text(checked, "name"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare ptr @loz_schema_require(ptr, ptr)"));
        assert!(ir.contains("call ptr @loz_schema_require"));
        assert!(ir.contains("call ptr @loz_json_get_text"));
    }

    #[test]
    fn lowers_python_call_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    const payload: Json = json.parse("{\"text\":\"hello\"}");
    const result: Json = python.call("tools.analyze_text", payload);
    return json.get_i32(result, "length");
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare ptr @loz_python_call(ptr, ptr)"));
        assert!(ir.contains("call ptr @loz_python_call"));
    }

    #[test]
    fn lowers_tool_using_python_call_in_llvm_ir() {
        let source = r#"tool analyze_text(payload: Json) -> Json {
    return python.call("tools.analyze_text", payload);
}

func main() -> i32 {
    const payload: Json = json.parse("{\"text\":\"hello\"}");
    const result: Json = analyze_text(payload);
    return json.get_i32(result, "length");
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define ptr @__loz_tool__analyze_text(ptr %0)"));
        assert!(ir.contains("call ptr @loz_python_call"));
        assert!(ir.contains("call ptr @__loz_tool__analyze_text"));
    }

    #[test]
    fn lowers_llm_ask_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    const answer: Text = llm.ask("hello");
    print(answer);
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare ptr @loz_llm_ask(ptr)"));
        assert!(ir.contains("call ptr @loz_llm_ask"));
    }

    #[test]
    fn lowers_tool_using_llm_ask_in_llvm_ir() {
        let source = r#"tool answer_user(question: Text) -> Text {
    return llm.ask(question);
}

func main() -> i32 {
    print(answer_user("What is Loz?"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define ptr @__loz_tool__answer_user(ptr %0)"));
        assert!(ir.contains("call ptr @loz_llm_ask"));
        assert!(ir.contains("call ptr @__loz_tool__answer_user"));
    }

    #[test]
    fn lowers_agent_task_using_llm_ask_in_llvm_ir() {
        let source = r#"agent SupportAgent {
    model: "mock";
    tools: [];

    task answer(question: Text) -> Text {
        return llm.ask(question);
    }
}

func main() -> i32 {
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define ptr @__loz_agent__SupportAgent__answer(ptr %0)"));
        assert!(ir.contains("call ptr @loz_llm_ask"));
    }

    #[test]
    fn lowers_workflow_with_tool_steps_in_llvm_ir() {
        let source = r#"tool get_data() -> Json {
    return json.parse("{\"name\":\"Ahmed\"}");
}

tool validate_data() -> Bool {
    return true;
}

workflow Onboarding {
    step get_data;
    step validate_data;
}

func main() -> i32 {
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define void @__loz_workflow__Onboarding()"));
        assert!(ir.contains("call ptr @__loz_tool__get_data"));
        assert!(ir.contains("call i1 @__loz_tool__validate_data"));
    }

    #[test]
    fn lowers_workflow_with_function_steps_in_llvm_ir() {
        let source = r#"func prepare() -> Text {
    return "prepared";
}

func done() -> Bool {
    return true;
}

workflow BasicFlow {
    step prepare;
    step done;
}

func main() -> i32 {
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define void @__loz_workflow__BasicFlow()"));
        assert!(ir.contains("call ptr @prepare"));
        assert!(ir.contains("call i1 @done"));
    }

    #[test]
    fn executes_async_task_sequentially_in_interpreter() {
        let source = r#"async task get_name() -> Text {
    return "Ahmed";
}

func main() -> i32 {
    const name: Text = await get_name();
    print(name);
    return 0;
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Int(0));
    }

    #[test]
    fn executes_async_json_task_calling_tool_in_interpreter() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

async task fetch_user(id: i32) -> Json {
    return get_user(id);
}

func main() -> Text {
    const user: Json = await fetch_user(1);
    return json.get_text(user, "name");
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Text("Ahmed".to_string()));
    }

    #[test]
    fn executes_nested_async_await_in_interpreter() {
        let source = r#"async task get_name() -> Text {
    return "Ahmed";
}

async task get_wrapped_name() -> Text {
    return await get_name();
}

func main() -> Text {
    return await get_wrapped_name();
}
"#;

        let program = prepare_program(source);
        let result = Interpreter::new().execute_program(&program).unwrap();

        assert_eq!(result, RuntimeValue::Text("Ahmed".to_string()));
    }

    #[test]
    fn lowers_async_task_symbol_and_await_call_in_llvm_ir() {
        let source = r#"async task fetch_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

func main() -> i32 {
    const user: Json = await fetch_user(1);
    print(json.get_text(user, "name"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define ptr @__loz_async__fetch_user(i32 %0)"));
        assert!(ir.contains("call ptr @__loz_async__fetch_user"));
    }

    #[test]
    fn lowers_async_task_using_tool_in_llvm_ir() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

async task fetch_user(id: i32) -> Json {
    return get_user(id);
}

func main() -> i32 {
    const user: Json = await fetch_user(1);
    print(json.get_text(user, "name"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define ptr @__loz_async__fetch_user(i32 %0)"));
        assert!(ir.contains("call ptr @__loz_tool__get_user"));
    }

    #[test]
    fn lowers_tool_to_mangled_function_name_in_llvm_ir() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

func main() -> i32 {
    const user: Json = get_user(1);
    print(json.get_text(user, "name"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("define ptr @__loz_tool__get_user(i32 %0)"));
    }

    #[test]
    fn lowers_tool_call_to_mangled_function_call_in_llvm_ir() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

func main() -> i32 {
    const user: Json = get_user(1);
    print(json.get_text(user, "name"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("call ptr @__loz_tool__get_user"));
        assert!(!ir.contains("call ptr @get_user"));
    }

    #[test]
    fn allows_json_returning_tool_in_llvm_ir() {
        let source = r#"tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}

func main() -> i32 {
    const user: Json = get_user(1);
    print(json.get_text(user, "name"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("declare ptr @loz_json_parse(ptr)"));
        assert!(ir.contains("call ptr @loz_json_parse"));
        assert!(ir.contains("call ptr @loz_json_get_text"));
    }

    #[test]
    fn allows_schema_require_inside_tool_in_llvm_ir() {
        let source = r#"schema User {
    id: i32,
    name: Text
}

tool get_user(id: i32) -> Json {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
    return schema.require("User", user);
}

func main() -> i32 {
    const user: Json = get_user(1);
    print(json.get_text(user, "name"));
    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("call ptr @loz_schema_require"));
        assert!(ir.contains("@schema_User_descriptor = private constant"));
    }

    #[test]
    fn allows_for_loop_in_llvm_ir() {
        let source = r#"func main() -> Int {
    const nums: Array<Int> = [10, 20, 30];
    mut sum: Int = 0;
    for mut x in nums {
        x = x + 1;
        sum = sum + x;
    }
    return sum;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("forcond:"));
        assert!(ir.contains("forbody:"));
        assert!(ir.contains("forexit:"));
        assert!(ir.contains("alloca [3 x i64]"));
        assert!(ir.contains("icmp ult i64"));
        assert!(ir.contains("getelementptr inbounds [3 x i64], ptr"));
        assert!(ir.contains("store i64 %for_idx_next, ptr"));
    }

    #[test]
    fn allows_break_and_continue_in_llvm_ir() {
        let source = r#"func main() -> i32 {
    const nums: Array<i32> = [10, 20, 30];

    for x in nums {
        if x == 20 {
            continue;
        }

        print(x);

        if x == 30 {
            break;
        }
    }

    mut y: i32 = 0;
    while y < 5 {
        y = y + 1;

        if y == 2 {
            continue;
        }

        if y == 3 {
            break;
        }
    }

    return 0;
}
"#;

        let program = prepare_program(source);
        let ir = generate_llvm_ir(&program).unwrap();

        assert!(ir.contains("forstep:"));
        assert!(ir.contains("forexit:"));
        assert!(ir.contains("br label %forstep"));
        assert!(ir.contains("br label %forexit"));
        assert!(ir.contains("loopcond:"));
        assert!(ir.contains("loopexit:"));
        assert!(ir.contains("br label %loopcond"));
        assert!(ir.contains("br label %loopexit"));
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

        assert!(error.message.contains(
            "top-level const primitive globals, struct declarations, schema declarations, function declarations, tool declarations, agent declarations, workflow declarations, and impl blocks"
        ));
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
