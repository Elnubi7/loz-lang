# Python Interop

Loz supports Python interop through:

```loz
python.call("module.function", payload)
```

The Loz type is:

```loz
python.call("module.function", payload: Json) -> Json
```

## Example Loz Code

```loz
func main() -> i32 {
    const payload: Json = json.parse("{\"text\":\"hello\"}");
    const result: Json = python.call("tools.analyze_text", payload);
    print(json.get_i32(result, "length"));
    return 0;
}
```

## Example Python Code

```python
def analyze_text(payload):
    text = payload["text"]
    return {
        "length": len(text),
        "label": "ok",
    }
```

## Contract

- Loz passes a Json value
- Python receives a normal Python structure derived from that Json payload
- Python should return a JSON-serializable object
- Loz receives it back as `Json`

## Configuration

Python executable resolution can come from:

- `LOZ_PYTHON_PATH`
- `.env`
- `[python].path` in `loz.toml`

## Runtime Model

- Uses a subprocess bridge
- Resolves Python modules from the Loz project root
- Does not embed Python yet
