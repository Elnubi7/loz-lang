# VS Code Extension

The Loz VS Code extension lives in:

```text
vscode-loz/
```

## Current Features

- Syntax highlighting
- Loz snippets
- Command Palette entries for `check`, `run`, `build`, and `llvm-ir`
- Agent/workflow helper commands
- Loz file icon theme

## Commands

- `Loz: Check Current File`
- `Loz: Run Current File`
- `Loz: Build Current File`
- `Loz: Generate LLVM IR`
- `Loz: Agent List`
- `Loz: Workflow List`

These commands assume `loz` is on `PATH`.

## Local Install

If you have a VSIX:

```bash
code --install-extension ./vscode-loz-<version>.vsix
```

## Packaging

The normal packaging command is:

```bash
cd vscode-loz
vsce package
```

At the moment, VSIX regeneration may require a compatible `Node` and `vsce` setup.
