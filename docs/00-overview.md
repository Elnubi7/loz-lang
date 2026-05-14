# Loz Overview

Loz is a compiled, statically typed, safe-by-default programming language designed for developers who want predictable behavior, clear syntax, and explicit error handling without the rougher edges of low-level programming.

Loz v1 is intentionally narrow. The first version focuses on a dependable core language rather than a large feature surface.

## What Loz Is Trying To Be

Loz is intended to be:

- easy to read in code review
- explicit about mutation and failure
- predictable for both users and compiler authors
- safe without exposing raw memory operations
- practical for application and tooling code

## Why Loz Exists

Many modern languages force a tradeoff:

- JavaScript is flexible, but that flexibility often creates runtime surprises.
- Python is productive, but many mistakes are only caught after code runs.
- Rust is powerful and safe, but its learning curve can be high for teams that want a simpler surface area.

Loz exists to explore a middle path: a language that keeps the code straightforward while enforcing stronger guarantees at compile time.

## Core v1 Identity

Loz v1 makes a few strong choices early:

- semicolons are required
- mutation is explicit through `mut`
- immutable values use `const`
- `null` is not part of the language model
- exceptions are not part of the error model
- raw pointers are not part of safe v1 programming

## Example

```loz
struct User {
    name: Text;
    age: Int;
}

func describe(user: ref User) -> Text {
    return user.name;
}
```

## Non-Goals For v1

Loz v1 does not attempt to solve everything. It deliberately excludes classes, inheritance, async features, macros, package management, and the AI-oriented concepts that may be explored later in future versions.
