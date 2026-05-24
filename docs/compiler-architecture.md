# Compiler Architecture

Loz currently follows this pipeline:

```text
Source
→ Lexer
→ Parser / AST
→ Semantic analysis
→ Optimizer
→ Interpreter or LLVM codegen
→ Native executable
```

## Workspace Crates

- `crates/loz_ast`
- `crates/loz_lexer`
- `crates/loz_parser`
- `crates/loz_semantic`
- `crates/loz_optimizer`
- `crates/loz_codegen`
- `crates/loz_runtime`
- `crates/loz_cli`

## Roles

### `loz_ast`

Defines the shared AST, spans, and diagnostics structures used across the compiler.

### `loz_lexer`

Turns source text into tokens with source spans.

### `loz_parser`

Builds the Loz AST from tokens and produces parser diagnostics with source locations.

### `loz_semantic`

Validates types, declarations, imports, tools, schemas, agent tasks, workflows, and async usage.

### `loz_optimizer`

Runs compile-time constant evaluation, const propagation, and dead branch cleanup.

### `loz_codegen`

Provides:

- interpreter execution for `loz run`
- LLVM IR generation for `loz llvm-ir`
- lowering for tools, agents, workflows, schema calls, Python calls, LLM calls, and async tasks

### `loz_runtime`

Contains runtime support used by native builds, including Json, schema, Python bridge, and LLM support.

### `loz_cli`

Implements project loading, CLI commands, package import resolution, diagnostics rendering, and native build orchestration.

## Execution Modes

### `loz run`

- loads project config and imports
- runs semantic analysis
- runs the optimizer
- executes the optimized program in the interpreter

### `loz llvm-ir`

- runs the same front-end pipeline
- lowers the program to LLVM IR
- prints the generated IR

### `loz build`

- generates LLVM IR
- invokes `llc`
- links against the native runtime
- writes an executable and `.ll` file into `output/`

## Current Architecture Notes

- Import resolution and local package loading currently live in `loz_cli`
- Built-in namespaces such as `json`, `schema`, `python`, and `llm` are recognized directly
- Async lowering exists, but execution is still sequential MVP rather than scheduler-based
