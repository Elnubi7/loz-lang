# 🧠 Loz Language Syntax

This guide focuses on the currently working beginner-level Loz syntax demonstrated by the repository examples. It does not invent syntax beyond what exists today.

## Program Structure

A minimal Loz program is a set of top-level declarations, typically ending with a `main` function:

```loz
func main() -> i32 {
    print("Hello from Loz");
    return 0;
}
```

Source example:

- `examples/hello.loz`

## `main` Function

Current examples use an `i32`-returning `main` function:

```loz
func main() -> i32 {
    return 0;
}
```

In example programs, `main` usually prints a result and then returns `0`.

## Functions

Functions are declared with `func`, a name, parameters, and a return type:

```loz
func square_plus_one(x: i32) -> i32 {
    return x * x;
}
```

Source example:

- `examples/functions.loz`

## Return Values

Loz uses explicit `return` statements:

```loz
func main() -> i32 {
    return 0;
}
```

Functions that return values also use `return` directly:

```loz
func square_plus_one(x: i32) -> i32 {
    return x * x;
}
```

## `print`

Examples use `print(...)` to write output:

```loz
print("Hello from Loz");
print(10 + 20);
print(value);
```

This is how the native examples surface their observable behavior.

## Integer Arithmetic

Basic arithmetic works in the current examples:

```loz
func main() -> i32 {
    print(10 + 20);
    return 0;
}
```

Source example:

- `examples/arithmetic.loz`

Expected stdout:

```text
30
```

## Variables

Mutable variables use `mut` with an explicit type annotation:

```loz
mut value: i32 = 40;
value = value + 2;
```

Source example:

- `examples/variables.loz`

Expected stdout:

```text
42
```

## Mutability

The example set currently demonstrates mutable local variables through `mut`:

```loz
mut count: i32 = 1;
mut total: i32 = 0;
```

Without `mut`, the beginner docs should assume the value is not intended for reassignment.

## `if` / `else`

Branching uses a direct condition followed by brace-delimited blocks:

```loz
if 10 > 5 {
    print("yes");
} else {
    print("no");
}
```

Source example:

- `examples/if_else.loz`

Expected stdout:

```text
yes
```

## `while` Loops

Loops use `while` with a condition and a block:

```loz
mut count: i32 = 1;
mut total: i32 = 0;
while count <= 5 {
    total = total + count;
    count = count + 1;
}
```

Source example:

- `examples/while_loop.loz`

Expected stdout:

```text
15
```

## Function Calls

Function calls use standard parenthesized argument syntax:

```loz
print(square_plus_one(5));
```

Source example:

- `examples/functions.loz`

Expected stdout:

```text
25
```

## Comments

The current lexer supports line comments with `//`:

```loz
// this is a comment
func main() -> i32 {
    return 0;
}
```

## Types Currently Supported

The broader language implementation supports more than the beginner examples use. Current built-in scalar types documented in the repo are:

| Type | Notes |
| --- | --- |
| `i8`, `i16`, `i32`, `i64` | Signed integers |
| `u8`, `u16`, `u32`, `u64` | Unsigned integers |
| `Int` | General integer type used in parts of the codebase and docs |
| `f32`, `f64` | Floating-point values |
| `Text` | String/text values |
| `Bool` | Boolean values |
| `Json` | JSON runtime values |
| `Void` / `void` | No-value return type |

For the examples in this guide, the most relevant types are:

- `i32`
- `Text`

## Example Set Used In This Guide

| Example | Purpose | Expected stdout |
| --- | --- | --- |
| `examples/hello.loz` | Minimal program and `print` | `Hello from Loz` |
| `examples/arithmetic.loz` | Integer arithmetic | `30` |
| `examples/variables.loz` | Variable declaration and mutation | `42` |
| `examples/if_else.loz` | Branching | `yes` |
| `examples/while_loop.loz` | Looping and mutation | `15` |
| `examples/functions.loz` | Function call and return value | `25` |

## Unsupported Or Partially Supported Syntax

These are important for accurate Alpha-stage expectations:

- Block comments are not currently part of the supported syntax guide.
- The language has more advanced areas than this beginner guide covers, but they should be treated as evolving.
- Syntax may change during Alpha.
- Native execution coverage is currently strongest on Linux.

For advanced language areas such as JSON, schemas, tools, workflows, async tasks, and packages, see the existing reference material in the `docs/` directory after you are comfortable with the basics above.
