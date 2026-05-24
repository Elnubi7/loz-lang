# Package System

Loz currently ships a package manager MVP based on local path dependencies.

## `loz.toml`

```toml
[dependencies]
text_utils = { path = "./packages/text_utils" }
```

## Import

```loz
import text_utils;
```

## Use

```loz
func main() -> i32 {
    print(text_utils.title());
    return 0;
}
```

## CLI

```bash
loz deps
```

Example output:

```text
Dependencies:

text_utils
  path: ./packages/text_utils
  main: src/lib.loz
```

## Entry Resolution

For a dependency package, Loz currently prefers:

1. `[project].main`
2. `src/lib.loz`
3. `src/main.loz`

## Current Limitations

- Local path dependencies only
- No `loz add` yet
- No registry
- No `loz publish`
- No `loz.lock`
- No version solver
