# Project Config

Loz project mode is driven by `loz.toml` and optional `.env`.

## `loz.toml`

Current sections:

- `[project]`
- `[llm]`
- `[ollama]`
- `[github]`
- `[python]`
- `[dependencies]`

## Example

```toml
[project]
name = "support-agent"
version = "0.1.0"
main = "src/main.loz"

[dependencies]
text_utils = { path = "./packages/text_utils" }

[llm]
provider = "mock"
model = "qwen2.5:0.5b"

[ollama]
base_url = "http://localhost:11434"

[github]
token_env = "GITHUB_TOKEN"
models_base_url = "https://models.github.ai/inference"

[python]
path = "python3"
```

## Environment Variables

- `LOZ_LLM_PROVIDER`
- `LOZ_MODEL`
- `LOZ_OLLAMA_BASE_URL`
- `LOZ_PYTHON_PATH`
- `GITHUB_TOKEN`
- `LOZ_GITHUB_TOKEN_ENV`

## Precedence

Current precedence is:

1. process environment
2. `.env`
3. `loz.toml`
4. built-in defaults

## Notes

- `LOZ_GITHUB_TOKEN_ENV` controls which environment variable name Loz reads for GitHub Models authentication
- package dependencies are local path only in `v0.1.0-alpha`
