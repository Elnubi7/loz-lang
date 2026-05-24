# Agent System

Loz has an agent-native surface for defining typed agent tasks.

## Declaration

```loz
agent SupportAgent {
    model: "mock";
    tools: [];

    task answer(question: Text) -> Text {
        return llm.ask(question);
    }
}
```

## CLI

List agents:

```bash
loz agent list examples/agent_support.loz
```

Run an agent task explicitly:

```bash
loz agent run examples/agent_support.loz SupportAgent answer "hello"
```

Shortcut mode:

```bash
LOZ_LLM_PROVIDER=mock loz agent run examples/agent_support.loz "hello"
```

Shortcut mode works when a single agent and task can be auto-selected.

## Native Lowering

Agent tasks are lowered to native symbols similar to:

```text
__loz_agent__Agent__Task
```

## Current Limitations

- No memory layer
- No planner loop
- No autonomous tool selection yet
- No multi-agent orchestration yet
