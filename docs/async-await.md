# Async / Await

Loz currently supports an MVP async surface:

```loz
async task fetch_user(id: i32) -> Json {
    return get_user(id);
}

func main() -> i32 {
    const user: Json = await fetch_user(1);
    print(json.get_text(user, "name"));
    return 0;
}
```

## Current Model

- `async task` declares an async callable
- `await` is required when using async task results
- execution is still sequential in `v0.1.0-alpha`

## Native Lowering

Async tasks lower to native functions with names similar to:

```text
__loz_async__name
```

## Current Limitations

- No scheduler
- No futures abstraction
- No parallel task execution
- No event loop
