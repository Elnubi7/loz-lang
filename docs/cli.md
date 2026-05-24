# CLI

## Overview

Loz supports both explicit source-file mode and project mode through `loz.toml`.

## Commands

### `loz --help`

Usage:

```bash
loz --help
```

Shows the command overview.

### `loz --version`

```bash
loz --version
```

Prints the current Loz CLI version.

### `loz init`

```bash
loz init my-agent
```

Creates a new Loz project with:

- `loz.toml`
- `src/main.loz`
- `tools/tools.py`
- `packages/README.md`
- `.env.example`

### `loz check`

```bash
loz check examples/hello.loz
loz check
```

Runs parsing, import resolution, package loading, semantic analysis, and optimizer checks.

Project mode uses `[project].main` from `loz.toml`.

Expected success output:

```text
Check passed.
```

### `loz run`

```bash
loz run examples/hello.loz
loz run
```

Runs the interpreter path after analysis and optimization.

### `loz llvm-ir`

```bash
loz llvm-ir examples/hello.loz
loz llvm-ir
```

Prints the generated LLVM IR for the loaded program graph.

### `loz build`

```bash
loz build examples/hello.loz
loz build
```

Builds:

- `output/<name>`
- `output/<name>.ll`

### `loz doctor`

```bash
loz doctor
```

Checks:

- toolchain commands such as `cargo`, `rustc`, `clang`, and `llc`
- project config presence
- runtime configuration such as Python path and LLM provider

### `loz deps`

```bash
loz deps
```

Lists local path dependencies for the current project.

Example output:

```text
Dependencies:

text_utils
  path: ./packages/text_utils
  main: src/lib.loz
```

### `loz agent list`

```bash
loz agent list examples/agent_support.loz
loz agent list
```

Lists discovered agents, models, tools, and task signatures.

### `loz agent run`

```bash
loz agent run examples/agent_support.loz SupportAgent answer "hello"
LOZ_LLM_PROVIDER=mock loz agent run examples/agent_support.loz "hello"
```

Supports explicit selection and shortcut mode when a single agent/task can be auto-selected.

### `loz workflow list`

```bash
loz workflow list examples/workflow_onboarding.loz
loz workflow list
```

Lists workflows and their steps.

### `loz workflow run`

```bash
loz workflow run examples/workflow_onboarding.loz Onboarding
loz workflow run examples/workflow_onboarding.loz
```

Supports explicit selection and shortcut mode when a single workflow exists.
