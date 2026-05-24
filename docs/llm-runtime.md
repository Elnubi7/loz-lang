# LLM Runtime

Loz currently exposes one LLM entrypoint:

```loz
llm.ask(prompt: Text) -> Text
```

## Example

```loz
func main() -> Text {
    return llm.ask("hello");
}
```

## Providers

- `mock`
- `ollama`
- `github`

## Configuration Examples

### Mock

```bash
LOZ_LLM_PROVIDER=mock
```

### Ollama

```bash
LOZ_LLM_PROVIDER=ollama
LOZ_OLLAMA_BASE_URL=http://localhost:11434
LOZ_MODEL=qwen2.5:0.5b
```

### GitHub Models

```bash
LOZ_LLM_PROVIDER=github
GITHUB_TOKEN=...
LOZ_MODEL=gpt-4.1-mini
```

## Current Limitations

- No streaming yet
- No tool-calling loop yet
- No conversation memory yet
- Provider support is runtime-oriented, not an orchestration framework
