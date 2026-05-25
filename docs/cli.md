# 🛠 Loz CLI Guide

This guide documents the CLI commands that actually exist in the current Loz repository. The command surface below is based on the implementation in `crates/loz_cli/src/main.rs` and on direct local command checks.

## Command Summary

```text
loz run [source.loz]
loz check [source.loz]
loz llvm-ir [source.loz]
loz build [source.loz]
loz deps
loz agent list [source.loz]
loz agent run [source.loz] [AgentName] [TaskName] [args...]
loz workflow list [source.loz]
loz workflow run [source.loz] [WorkflowName]
loz doctor
loz init <project-name>
loz --version
```

## 📋 Quick Reference

| Command | Purpose | Example |
| --- | --- | --- |
| `loz --help` | Show usage | `./target/debug/loz --help` |
| `loz --version` | Print version | `./target/debug/loz --version` |
| `loz check [source.loz]` | Validate a program | `./target/debug/loz check examples/hello.loz` |
| `loz run [source.loz]` | Run a program in interpreter mode | `./target/debug/loz run examples/hello.loz` |
| `loz llvm-ir [source.loz]` | Print LLVM IR to stdout | `./target/debug/loz llvm-ir examples/hello.loz` |
| `loz build [source.loz]` | Build native executable + `.ll` file | `./target/debug/loz build examples/hello.loz` |
| `loz deps` | Show project path dependencies | `cd examples/package_demo && ../../target/debug/loz deps` |
| `loz doctor` | Inspect environment readiness | `./target/debug/loz doctor` |
| `loz init <project-name>` | Create a starter project | `./target/debug/loz init sample-app` |
| `loz agent list [source.loz]` | Show agents in a source file | `./target/debug/loz agent list examples/agent_support.loz` |
| `loz agent run ...` | Execute an agent task | `LOZ_LLM_PROVIDER=mock ./target/debug/loz agent run examples/agent_support.loz "hello"` |
| `loz workflow list [source.loz]` | Show workflows in a source file | `./target/debug/loz workflow list examples/workflow_onboarding.loz` |
| `loz workflow run [source.loz] [WorkflowName]` | Execute a workflow | `./target/debug/loz workflow run examples/workflow_onboarding.loz` |

## `--help`

### Purpose

Print the built-in usage summary.

### Syntax

```bash
./target/debug/loz --help
```

### Notes

- The help output lists the public CLI entry points.
- There is no separate `help` subcommand; use `--help` or `-h`.

## `--version`

### Purpose

Print the current Loz CLI version string.

### Syntax

```bash
./target/debug/loz --version
```

### Example output

```text
loz 0.1.0
```

## `check`

### Purpose

Parse, resolve, and semantically validate a Loz program.

### Syntax

```bash
./target/debug/loz check [source.loz]
```

### Example

```bash
./target/debug/loz check examples/hello.loz
```

### Example output

```text
Check passed.
```

### Notes

- `source.loz` is optional.
- When no source file is provided, Loz tries to resolve project context from the current directory.

## `run`

### Purpose

Execute a Loz program in interpreter mode.

### Syntax

```bash
./target/debug/loz run [source.loz]
```

### Example

```bash
./target/debug/loz run examples/hello.loz
```

### Example output

```text
Hello from Loz
```

### Notes

- `run` does not produce a native executable.
- It is the fastest way to exercise a small example during development.

## `llvm-ir`

### Purpose

Print generated LLVM IR to stdout for a checked Loz program.

### Syntax

```bash
./target/debug/loz llvm-ir [source.loz]
```

### Example

```bash
./target/debug/loz llvm-ir examples/hello.loz
```

### Output behavior

- LLVM IR is written directly to stdout.
- No executable is produced by this command.

## `build`

### Purpose

Generate LLVM IR, lower it to an object file, and link a native executable.

### Syntax

```bash
./target/debug/loz build [source.loz]
```

### Example

```bash
./target/debug/loz build examples/hello.loz
./output/hello
```

### Example output

```text
Built native executable: '/path/to/output/hello' and LLVM IR: '/path/to/output/hello.ll'
```

### Generated files

| File | Description |
| --- | --- |
| `output/<name>` | Native executable |
| `output/<name>.ll` | LLVM IR snapshot saved by Loz |

### Notes

- Native build is Linux-first in current validation coverage.
- On Ubuntu/Linux you need the native development toolchain installed for linking.

## `deps`

### Purpose

Show local path dependencies for the current Loz project.

### Syntax

```bash
./target/debug/loz deps
```

### Example

```bash
cd examples/package_demo
../../target/debug/loz deps
```

### Example output

```text
Dependencies:

text_utils
  path: ./packages/text_utils
  main: src/lib.loz
```

### Notes

- This command is project-oriented and expects a project layout with `loz.toml`.

## `doctor`

### Purpose

Inspect the current environment and report whether Loz is ready for interpreter and native workflows.

### Syntax

```bash
./target/debug/loz doctor
```

### Example output

```text
Loz doctor

Platform:
  ...
Toolchain:
  ...
Runtime:
  ...
Status:
  ...
```

### Notes

- `doctor` reports warnings for missing optional runtime integrations.
- Native toolchain readiness and runtime provider configuration are reported separately.

## `init`

### Purpose

Create a starter Loz project directory.

### Syntax

```bash
./target/debug/loz init <project-name>
```

### Example

```bash
./target/debug/loz init sample-app
```

### Generated files

```text
sample-app/.env.example
sample-app/README.md
sample-app/examples/hello.loz
sample-app/loz.toml
sample-app/packages/README.md
sample-app/src/main.loz
sample-app/tools/tools.py
```

### Notes

- `init` refuses to overwrite a non-empty existing directory.
- The generated project includes a starter example and Python tools stub.

## `agent list`

### Purpose

List agents declared in a Loz source file.

### Syntax

```bash
./target/debug/loz agent list [source.loz]
```

### Example

```bash
./target/debug/loz agent list examples/agent_support.loz
```

### Example output

```text
Agents found:

SupportAgent
  model: mock
  tools:
    (none)
  tasks:
    answer(question: Text) -> Text
```

## `agent run`

### Purpose

Execute an agent task.

### Syntax

```bash
./target/debug/loz agent run [source.loz] [AgentName] [TaskName] [args...]
```

### Example

```bash
LOZ_LLM_PROVIDER=mock ./target/debug/loz agent run examples/agent_support.loz "hello"
```

### Example output

```text
[mock] hello
```

### Notes

- Loz can auto-select the agent and task when the program exposes exactly one candidate.
- For public docs, prefer the explicit form shown in the usage line above.

## `workflow list`

### Purpose

List workflows declared in a Loz source file.

### Syntax

```bash
./target/debug/loz workflow list [source.loz]
```

### Example

```bash
./target/debug/loz workflow list examples/workflow_onboarding.loz
```

### Example output

```text
Workflows found:

Onboarding
  steps:
    1. get_data
```

## `workflow run`

### Purpose

Execute a workflow.

### Syntax

```bash
./target/debug/loz workflow run [source.loz] [WorkflowName]
```

### Example

```bash
./target/debug/loz workflow run examples/workflow_onboarding.loz
```

### Example output

```text
[1/1] get_data
{"name":"Ahmed"}
```

### Notes

- If exactly one workflow exists, Loz can auto-select it when the name is omitted.

## Practical Validation Set

For a quick end-to-end CLI sanity pass:

```bash
cargo build --workspace
./target/debug/loz --version
./target/debug/loz check examples/hello.loz
./target/debug/loz run examples/hello.loz
./target/debug/loz build examples/hello.loz
./output/hello
./scripts/test_native_examples.sh
```
