# Loz Project Structure

This project phase creates documentation only. No Rust workspace, compiler crates, or executable code should be created yet.

## Current Repository Structure

At this stage, the repository should stay minimal:

```text
loz-lang/
├── docs/
│   ├── 00-overview.md
│   ├── 01-language-goals.md
│   ├── 02-v1-scope.md
│   ├── 03-syntax-preview.md
│   ├── 04-compiler-architecture.md
│   ├── 05-project-structure.md
│   └── 06-roadmap.md
└── README.md
```

## Planned Future Repository Layout

Once implementation starts, a Rust-based workspace will likely be introduced. A plausible future layout is:

```text
loz-lang/
├── crates/
│   ├── loz_cli/
│   ├── loz_lexer/
│   ├── loz_parser/
│   ├── loz_ast/
│   ├── loz_semantic/
│   └── loz_codegen/
├── examples/
├── tests/
├── docs/
└── README.md
```

## Directory Intent

- `docs/` contains the language definition and planning documents.
- `crates/` would contain the Rust implementation once work begins.
- `examples/` would contain small `.loz` sample programs.
- `tests/` would contain compiler and language behavior tests.

## Structure Principles

- keep the current repository small until design decisions are stable
- separate compiler phases into focused Rust crates later
- keep language documentation versioned with implementation work
- avoid creating placeholder code directories before they have a concrete purpose
