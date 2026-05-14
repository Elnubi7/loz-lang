# Loz

Loz is a new compiled, statically typed, safe-by-default programming language project.

The repository is currently in Phase 1: Rust workspace setup. The workspace structure exists, but compiler, lexer, parser, semantic analysis, and code generation logic have not been implemented yet.

## Why Loz Exists

Loz is intended to sit between highly dynamic languages and highly complex systems languages.

It aims to keep the readability and day-to-day productivity that make JavaScript and Python attractive, while avoiding runtime type surprises, hidden `null` values, and exception-heavy control flow. At the same time, it aims to preserve safety and performance-oriented design without exposing the full complexity that many developers experience when first approaching Rust.

## Loz v1 Summary

- Compiled
- Statically typed
- Safe by default
- Implemented in Rust
- Planned backend later: LLVM
- Source file extension: `.loz`
- CLI name: `loz`
- Required semicolons
- `const` for immutable values
- `mut` for mutable values
- Primitive types: `Int`, `Float`, `Bool`, `Text`, `Char`, `Void`
- Planned collections: `Array<T>`, `Map<K, V>`, `Set<T>`
- No `null`; use `Maybe<T>`
- No exceptions; use `Result<T, E>`
- Structs supported
- References supported with `ref T` and `mut ref T`
- Raw pointers not supported in v1

## Small Example

```loz
func add(a: Int, b: Int) -> Int {
    return a + b;
}

func main() -> Void {
    const total: Int = add(2, 3);
    print(total);
}
```

## Repository Contents

- `Cargo.toml`: workspace manifest
- `README.md`: project introduction
- `docs/`: language and project documentation
- `crates/loz_cli`: compiler CLI binary crate
- `crates/loz_lexer`: lexer crate placeholder
- `crates/loz_ast`: AST crate placeholder
- `crates/loz_parser`: parser crate placeholder
- `crates/loz_semantic`: semantic analysis crate placeholder
- `crates/loz_codegen`: code generation crate placeholder
- `examples/`: future `.loz` examples
- `tests/`: future integration and compiler tests

## Current Status

The workspace now builds successfully, and the `loz` CLI entrypoint exists as a minimal placeholder. Language implementation work has not started yet.
