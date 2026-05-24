# Runtime

Loz has two main execution paths:

- interpreter execution through `loz run`
- native execution through LLVM + `loz_runtime`

## Runtime Features

- Text values
- `io.read_line()` and numeric/bool input helpers
- Json parsing, stringifying, and typed accessors
- Schema validation and `schema.require()`
- Python subprocess bridge via `python.call()`
- LLM providers via `llm.ask()`

## Native Runtime Responsibilities

The native runtime currently provides:

- Json helpers
- Schema validation helpers
- Python bridge support
- LLM runtime entrypoints
- text/input glue used by generated LLVM code

## Known Limitations

- Some runtime strings and Json allocations may leak in alpha
- Async is sequential MVP
- There is no real scheduler yet
- Local packages only in the current package system
- No sandboxed permissions model yet
- No HTTP or filesystem stdlib modules yet
