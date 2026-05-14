# Loz Syntax Preview

This document provides an early preview of the intended Loz v1 syntax. The examples are illustrative and stay within the current v1 scope.

## Function Syntax

```loz
func add(a: Int, b: Int) -> Int {
    return a + b;
}
```

## Variables

Immutable values use `const`. Mutable values use `mut`.

```loz
const language: Text = "Loz";
mut version: Int = 1;

version = version + 1;
```

## Conditions

```loz
func label(score: Int) -> Text {
    if score >= 50 {
        return "pass";
    } else {
        return "fail";
    }
}
```

## Loops

```loz
mut index: Int = 0;

while index < 3 {
    print(index);
    index = index + 1;
}
```

```loz
const items: Array<Text> = ["a", "b", "c"];

for item in items {
    print(item);
}
```

## Structs

```loz
struct User {
    name: Text;
    active: Bool;
}

const user: User = User {
    name: "Mina",
    active: true
};
```

## References

```loz
func print_name(user: ref User) -> Void {
    print(user.name);
}
```

```loz
func activate(user: mut ref User) -> Void {
    user.active = true;
}
```

## Maybe And Result

Loz does not use `null` or exceptions in v1. APIs should express absence and failure directly in the type system.

```loz
func find_name(id: Int) -> Maybe<Text> {
    return none;
}
```

```loz
func divide(a: Int, b: Int) -> Result<Int, Text> {
    if b == 0 {
        return error("division by zero");
    } else {
        return ok(a / b);
    }
}
```

The exact helper syntax around constructing and consuming `Maybe<T>` and `Result<T, E>` may evolve during implementation, but the v1 rule is fixed: absence is explicit, and failure is explicit.
