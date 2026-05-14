# Loz v1 Scope

This document defines the initial boundary for Loz v1. Anything not listed here should be treated as out of scope until the language core is stable.

## Included In v1

### Basic Source Model

- source files use the `.loz` extension
- the compiler CLI is named `loz`
- semicolons are required
- blocks use braces

### Variables And Mutability

- `const` declares immutable values
- `mut` declares mutable values

```loz
const name: Text = "Loz";
mut count: Int = 0;

count = count + 1;
```

### Primitive Types

- `Int`
- `Float`
- `Bool`
- `Text`
- `Char`
- `Void`

### Planned Collection Types

- `Array<T>`
- `Map<K, V>`
- `Set<T>`

### Functions

Functions use explicit parameter types and explicit return types.

```loz
func add(a: Int, b: Int) -> Int {
    return a + b;
}
```

### Control Flow

- `if`
- `else`
- `while`
- `for item in collection`

```loz
if ready {
    print("start");
} else {
    print("wait");
}
```

### Structs

```loz
struct Point {
    x: Float;
    y: Float;
}
```

### References

- `ref T` for read-only references
- `mut ref T` for mutable references

### Optional Values And Errors

- `Maybe<T>` is used instead of `null`
- `Result<T, E>` is used instead of exceptions

## Not Included In v1

- classes
- inheritance
- raw pointers
- async
- agents
- tools
- workflows
- package manager
- macros

## Boundary Notes

Loz v1 is meant to be small enough to implement cleanly. That means the first version should optimize for consistency and safety rather than breadth. New features should only be added after they fit the existing model without weakening it.
