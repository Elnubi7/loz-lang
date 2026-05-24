# Loz Programming Language

Loz is an Egyptian-built, compiled-first agent-native programming language for AI tools, schemas, workflows, and automation.

## Status

Loz is in `v0.1.0-alpha` development.

- Experimental
- Not production-stable yet
- Focused on alpha usability, documentation, and native execution coverage

## Feature Highlights

- Compiled-first hybrid execution
- Interpreter mode with `loz run`
- LLVM native build with `loz build`
- Json + Schema support
- Tools
- Python interop
- LLM runtime with `mock`, `ollama`, and `github`
- Agents
- Workflows
- Async/Await MVP
- Local package dependencies
- `loz.toml` + `.env`
- VS Code extension

## Quick Start

```bash
loz init my-agent
cd my-agent
loz check
loz run
loz agent list
LOZ_LLM_PROVIDER=mock loz agent run "hello"
loz workflow run
```

## Basic Example

```loz
func main() -> i32 {
    print("Hello from Loz");
    return 0;
}
```

Run it with:

```bash
loz check examples/hello.loz
loz run examples/hello.loz
```

## Agent Example

```loz
agent SupportAgent {
    model: "mock";

    task answer(question: Text) -> Text {
        return llm.ask(question);
    }
}

func main() -> i32 {
    return 0;
}
```

Run it with:

```bash
LOZ_LLM_PROVIDER=mock loz agent run examples/agent_support.loz "hello"
```

## Local Package Example

`examples/package_demo/loz.toml`

```toml
[project]
name = "package-demo"
version = "0.1.0"
main = "src/main.loz"

[dependencies]
text_utils = { path = "./packages/text_utils" }
```

`examples/package_demo/src/main.loz`

```loz
import text_utils;

func main() -> i32 {
    print(text_utils.title());
    return 0;
}
```

## Build Example

```bash
loz llvm-ir examples/hello.loz
loz build examples/hello.loz
./output/hello
```

## CLI Command Overview

- `loz init <project-name>`: create a new Loz project
- `loz check [source.loz]`: parse, resolve imports, and run semantic checks
- `loz run [source.loz]`: interpret the program
- `loz llvm-ir [source.loz]`: print generated LLVM IR
- `loz build [source.loz]`: build a native executable and LLVM IR file
- `loz deps`: show local path dependencies for the current project
- `loz doctor`: inspect toolchain and runtime configuration
- `loz agent list [source.loz]`: list agents in a program
- `loz agent run [source.loz] [AgentName] [TaskName] [args...]`: run an agent task
- `loz workflow list [source.loz]`: list workflows in a program
- `loz workflow run [source.loz] [WorkflowName]`: run a workflow

## Project Structure

```text
.
├── crates/
│   ├── loz_ast
│   ├── loz_lexer
│   ├── loz_parser
│   ├── loz_semantic
│   ├── loz_optimizer
│   ├── loz_codegen
│   ├── loz_runtime
│   └── loz_cli
├── docs/
├── examples/
├── runtime/
└── vscode-loz/
```

## Roadmap Summary

Completed for alpha:

- Core language
- Interpreter
- LLVM/native build
- Json and Schema
- Tools
- Python interop
- LLM runtime
- Agents and Workflows
- Async/Await MVP
- Optimizer
- Better CLI diagnostics
- Local package dependencies
- VS Code extension

Remaining before broader release hardening:

- Cross-platform hardening
- GitHub Actions
- Release packaging
- VSIX regeneration on a compatible `Node`/`vsce` setup

See [docs/roadmap.md](docs/roadmap.md) and [docs/release-plan.md](docs/release-plan.md).

## Documentation

- [Vision](docs/vision.md)
- [Language Reference](docs/language-reference.md)
- [Compiler Architecture](docs/compiler-architecture.md)
- [CLI](docs/cli.md)
- [Project Config](docs/project-config.md)
- [Package System](docs/package-system.md)
- [VS Code Extension](docs/vscode-extension.md)

## License

License selection is still being finalized for the public alpha release.
