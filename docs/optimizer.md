# Optimizer

Loz includes an optimizer pass focused on compile-time simplification.

## Current Optimizations

- Constant folding
- Simple const propagation
- Dead branch cleanup

## Example

```loz
func main() -> i32 {
    const x: Int = 15 + 10;
    const y: Int = x * 2;

    if 10 > 5 {
        return y;
    } else {
        return 0;
    }
}
```

This can be simplified before interpretation or LLVM lowering.

## Boundaries

The optimizer does not try to evaluate runtime side effects such as:

- `io.*`
- `python.call()`
- `llm.ask()`
- tool calls with runtime behavior
