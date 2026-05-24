# Json And Schema

## Json

Supported Json helpers:

- `json.parse(text)`
- `json.stringify(value)`
- `json.get_text(value, key)`
- `json.get_i32(value, key)`
- `json.get_i64(value, key)`
- `json.get_f64(value, key)`
- `json.get_bool(value, key)`
- `json.has(value, key)`

Example:

```loz
func main() -> i32 {
    const user: Json = json.parse("{\"id\":1,\"name\":\"Ahmed\",\"active\":true}");
    print(json.get_text(user, "name"));
    print(json.get_bool(user, "active"));
    return 0;
}
```

## Schema

Define a schema:

```loz
schema User {
    id: i32,
    name: Text,
    active: Bool
}
```

Validate:

```loz
print(schema.validate("User", user));
```

Require:

```loz
const checked: Json = schema.require("User", user);
```

## Interpreter And Native

Both interpreter mode and LLVM/native builds support:

- `schema.validate()`
- `schema.require()`
- Json parsing/stringifying
- Json getter helpers

## Notes

- `schema.require()` fails at runtime if validation fails
- schema names are currently passed as string literals
