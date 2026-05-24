# Language Reference

This document describes the current Loz `v0.1.0-alpha` syntax. It is intentionally practical rather than exhaustive.

## Comments

```loz
// line comment
/* block comment */
```

## Modules And Imports

```loz
module users;

func get_name() -> Text {
    return "Ahmed";
}
```

```loz
import users;

func main() -> i32 {
    print(users.get_name());
    return 0;
}
```

Built-in namespaces such as `json` and `llm` can be used without import. `import json;` is also accepted.

## Variables

```loz
const name: Text = "Ahmed";
mut count: i32 = 0;
```

`const` is immutable. `mut` allows reassignment and mutation where supported.

## Types

Current built-in scalar types:

- `i8`, `i16`, `i32`, `i64`
- `u8`, `u16`, `u32`, `u64`
- `Int`
- `Text`
- `Bool`
- `Json`
- `f32`, `f64`
- `Void` and `void`

Collection and composite types currently used in alpha:

- `Array<T>`
- `Map<K, V>`
- `Set<T>`
- named `struct` types
- references via `ref T` and `mut ref T`

## Functions

```loz
func add(a: i32, b: i32) -> i32 {
    return a + b;
}
```

## Structs

```loz
struct Point {
    x: i32,
    y: i32
}

func main() -> i32 {
    const p: Point = Point(10, 20);
    return p.x;
}
```

## impl And Methods

```loz
struct Point {
    x: i32,
    y: i32
}

impl Point {
    func sum(self) -> i32 {
        return self.x + self.y;
    }
}
```

## Arrays

```loz
const nums: Array<Int> = [10, 20, 30];
print(nums[1]);
```

Mutable arrays support:

- `nums.push(value)`
- `nums.pop()`
- `nums.len()`
- index assignment

## Maps

```loz
mut scores: Map<Text, i32> = Map();
scores.insert("ahmed", 100);
print(scores.contains("ahmed"));
print(scores.get("ahmed"));
```

## Sets

```loz
mut ids: Set<i32> = Set();
ids.add(10);
print(ids.contains(10));
```

## Control Flow

```loz
if score > 90 {
    print("high");
} else {
    print("normal");
}
```

## Loops

```loz
while count < 3 {
    count = count + 1;
}
```

```loz
for item in nums {
    print(item);
}
```

`break` and `continue` are supported inside loops.

## Casting

```loz
const x: f64 = as<f64>(10);
```

## References

```loz
mut x: i32 = 10;
const rx = ref x;
const rmut = mut ref x;
```

## Async Task / Await

```loz
async task get_name() -> Text {
    return "Ahmed";
}

func main() -> i32 {
    const name: Text = await get_name();
    print(name);
    return 0;
}
```

This is an MVP. Await is immediate and sequential in alpha.

## Schema

```loz
schema User {
    id: i32,
    name: Text
}
```

## Tool

```loz
tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}
```

## Agent

```loz
agent SupportAgent {
    model: "mock";

    task answer(question: Text) -> Text {
        return llm.ask(question);
    }
}
```

## Workflow

```loz
tool get_data() -> Json {
    return json.parse("{\"name\":\"Ahmed\"}");
}

workflow Onboarding {
    step get_data;
}
```

## Current Limitations

- No LSP yet
- No formatter yet
- No registry package manager
- Local packages only
- Workflow steps are sequential and zero-argument in this phase
- Async is sequential MVP, not a scheduler-backed runtime
