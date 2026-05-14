use loz_codegen::{execute, generate_llvm_ir};
use loz_lexer::tokenize;
use loz_parser::parse_program;
use loz_semantic::analyze;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let use_llvm_ir = args.get(1).map(|arg| arg.as_str()) == Some("llvm-ir");

    let source = if use_llvm_ir {
        r#"func main() -> Int {
    print(30);
    return 0;
}
"#
    } else {
        r#"const x: Int = 10;

func add(a: Int, b: Int) -> Int {
    return a + b;
}

func main() -> Void {
    print(add(x, 20));
}
"#
    };

    let tokens = tokenize(source);
    match parse_program(tokens) {
        Ok(program) => match analyze(&program) {
            Ok(()) => {
                if use_llvm_ir {
                    match generate_llvm_ir(&program) {
                        Ok(ir) => println!("{ir}"),
                        Err(error) => eprintln!("{error}"),
                    }
                } else if let Err(error) = execute(&program) {
                    eprintln!("{error}");
                }
            }
            Err(error) => eprintln!("{error}"),
        },
        Err(error) => eprintln!("{error}"),
    }
}
