# Tool System

Tools are top-level callable declarations intended for reusable automation steps.

## Declaration

```loz
tool get_user(id: i32) -> Json {
    return json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
}
```

## Calling A Tool

```loz
func main() -> i32 {
    const user: Json = get_user(1);
    print(json.get_text(user, "name"));
    return 0;
}
```

## Return Types

Tools can return normal Loz values, including:

- `i32`
- `Text`
- `Bool`
- `Json`
- named structs where supported by the rest of the type system

## Tools With Json And Schema

```loz
schema User {
    id: i32,
    name: Text
}

tool checked_user() -> Json {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\"}");
    return schema.require("User", user);
}
```

## Native Lowering

Tools are lowered into callable native functions during LLVM generation and can be used from:

- normal functions
- agent tasks
- workflows
- async tasks

## Current Limitations

- No permissions model yet
- No external HTTP stdlib yet
- No tool metadata export format yet
