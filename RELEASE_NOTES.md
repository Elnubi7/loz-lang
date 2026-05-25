# Loz v0.1.0-alpha Release Notes

Status: Alpha / experimental

Loz `v0.1.0-alpha` is the first public GitHub alpha release for the project. This release is intended for evaluation, source builds, documentation review, and early feedback. Syntax, CLI details, editor support, and native build behavior may still change during the alpha cycle.

## Key Features

- Loz CLI for checking, running, and building `.loz` programs
- Lexer, parser, and semantic checker pipeline
- Optimizer pass integrated into the compilation flow
- LLVM IR generation and native executable output
- Linux native build workflow
- Runtime support for current interpreter/native scenarios
- Example programs covering core syntax and automation-oriented features
- Native examples validation script in `scripts/test_native_examples.sh`
- VS Code extension in `vscode-loz/`
- Project documentation in `README.md` and `docs/`

## Installation From Source

Prerequisites:

- Rust toolchain with Cargo
- `clang`
- `llc`
- Linux native-build toolchain packages as described in `README.md` and `docs/native-build.md`

Build from source:

```bash
git clone https://github.com/Elnubi7/loz-lang.git
cd loz-lang
cargo build --workspace
./target/debug/loz --version
```

## VS Code Extension Local VSIX Install

Package the extension locally:

```bash
cd vscode-loz
vsce package
```

Install the generated VSIX locally:

```bash
code --install-extension ./vscode-loz-0.1.0-alpha.vsix
```

## Known Limitations

- Alpha / experimental quality; compatibility is not guaranteed yet
- Native executable validation is Linux-first
- Packaging/publishing metadata is being prepared for GitHub release use, not automated marketplace or crates.io publication
- Tooling around packages, runtime breadth, and cross-platform coverage remains limited

## Validation Commands

Run these commands from the repository root:

```bash
cargo fmt
cargo check --workspace
cargo test --workspace
cargo build --workspace
./scripts/test_native_examples.sh
cd vscode-loz
vsce package
```

## Next Steps

- Upload the release notes and VSIX asset to the GitHub release page
- Collect feedback on CLI ergonomics, native build reliability, docs clarity, and VS Code support
- Continue hardening native build coverage and alpha documentation
