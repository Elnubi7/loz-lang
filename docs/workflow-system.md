# Workflow System

Workflows provide a simple sequential execution surface for zero-argument steps.

## Declaration

```loz
tool get_data() -> Json {
    return json.parse("{\"name\":\"Ahmed\"}");
}

workflow Onboarding {
    step get_data;
}
```

## CLI

List workflows:

```bash
loz workflow list examples/workflow_onboarding.loz
```

Run workflows:

```bash
loz workflow run examples/workflow_onboarding.loz Onboarding
loz workflow run examples/workflow_onboarding.loz
```

Shortcut mode works when a single workflow exists.

## Native Lowering

Workflows are lowered to native functions similar to:

```text
__loz_workflow__Name
```

## Current Limitations

- Sequential only
- Zero-argument step targets only
- No branching
- No retry policy
- No parallel execution
- No output chaining yet
