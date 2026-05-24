# Loz VS Code Extension

VS Code language support for Loz v0.1.0-alpha.

## What It Supports

- `.loz` file association
- TextMate syntax highlighting for current Loz syntax
- Line and block comments
- Bracket matching and auto-closing pairs
- Loz snippets for common program, tool, agent, workflow, and async task patterns
- Command Palette commands that run the Loz CLI in an integrated terminal
- Optional `loz.toml` association with the built-in TOML language
- Optional Loz file icon theme for `.loz` files

## Syntax Highlighting

The extension highlights:

- Core keywords such as `module`, `import`, `using`, `extern`, `global`, `const`, `mut`, `let`, `var`, `func`, `return`, `if`, `else`, `while`, `for`, `break`, and `continue`
- Type names such as `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `Int`, `Text`, `Bool`, `Json`, `f32`, `f64`, `void`, and `Void`
- Agent-native constructs such as `schema`, `tool`, `agent`, `task`, `workflow`, `step`, `async`, and `await`
- Built-in namespaces such as `io`, `json`, `schema`, `python`, and `llm`
- Function, tool, task, agent, workflow, schema, and struct names where they can be identified from simple declarations
- Strings, numbers, `true`, `false`, and `null`
- `//` line comments and `/* ... */` block comments

## Snippets

Included Loz snippets:

- `main`
- `func`
- `tool`
- `schema`
- `agent`
- `workflow`
- `async-task`
- `json-parse`
- `python-call`
- `llm-ask`

## Commands

Open the Command Palette and run:

- `Loz: Check Current File`
- `Loz: Run Current File`
- `Loz: Build Current File`
- `Loz: Generate LLVM IR`
- `Loz: Agent List`
- `Loz: Workflow List`

These commands run the Loz CLI in a VS Code terminal against the current `.loz` file:

- `loz check <current file>`
- `loz run <current file>`
- `loz build <current file>`
- `loz llvm-ir <current file>`
- `loz agent list <current file>`
- `loz workflow list <current file>`

The commands assume the `loz` CLI is available on your shell `PATH`.

## Installing Locally

If you already have a packaged VSIX artifact from a local package step:

```bash
code --install-extension ./vscode-loz-<version>.vsix
```

If you need to build a VSIX locally and `vsce` is installed:

```bash
cd vscode-loz
vsce package
code --install-extension ./vscode-loz-<version>.vsix
```

If `vsce` is not installed yet:

```bash
npm install -g @vscode/vsce
cd vscode-loz
vsce package
```

## File Icons

The extension contributes a `Loz File Icons` icon theme. To enable it in VS Code:

1. Open `Preferences: File Icon Theme`
2. Select `Loz File Icons`

This gives `.loz` files a Loz-specific icon.

## Manual Verification

Recommended verification steps:

1. Open a `.loz` file in VS Code and confirm the language mode is `Loz`.
2. Check that `schema`, `tool`, `agent`, `task`, `workflow`, `step`, `async`, and `await` highlight correctly.
3. Trigger snippets such as `main`, `tool`, `agent`, `workflow`, and `async-task`.
4. Run `Loz: Check Current File`, `Loz: Run Current File`, and `Loz: Build Current File` from the Command Palette.
5. Confirm the commands open or reuse a `Loz` terminal and execute the expected CLI commands.

## Development

Open `vscode-loz/` in VS Code and launch it in an Extension Development Host to test highlighting, snippets, commands, and file icons locally.
