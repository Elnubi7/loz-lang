# Loz Roadmap

This roadmap describes the intended progression of the Loz project. Phase 0 is the only active phase right now.

## Phase 0: Language Definition

- write the initial project overview
- define language goals and tradeoffs
- define the Loz v1 scope
- publish an early syntax preview
- document the planned compiler architecture
- document the expected repository structure
- outline the first implementation roadmap

## Phase 1: Rust Workspace Setup

- create the Rust workspace
- add an initial `loz` CLI crate
- add placeholder front-end crates for lexer, parser, AST, and semantic analysis
- make the workspace build cleanly

## Phase 2: Lexing

- define token categories
- lex keywords, identifiers, literals, operators, and punctuation
- track source locations for diagnostics
- add lexer-focused tests

## Phase 3: Parsing And AST

- parse top-level declarations
- parse functions, structs, variable declarations, and expressions
- build a stable AST representation
- add parser tests and syntax fixtures

## Phase 4: Semantic Analysis

- resolve names and scopes
- check types
- enforce `const` versus `mut`
- validate function calls and returns
- validate struct and reference rules

## Phase 5: Initial Build Pipeline

- choose the first internal IR shape
- produce a minimal end-to-end pipeline from source file to checked program
- decide whether a temporary execution path is useful before LLVM integration

## Phase 6: LLVM Backend

- lower the checked program into LLVM-friendly IR
- generate native code
- support core primitives, functions, structs, and basic control flow

## Phase 7: Developer Experience

- improve diagnostics and error formatting
- add formatter support
- add editor tooling such as syntax highlighting

## Explicitly Deferred Beyond v1

- classes
- inheritance
- async
- agents
- tools
- workflows
- package manager
- macros
- raw pointers
