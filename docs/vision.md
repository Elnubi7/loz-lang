# Vision

Loz is an Egyptian-built, compiled-first, hybrid execution, agent-native programming language aimed at typed AI tools, schemas, workflows, and automation.

## What Loz Is

- Egyptian-built
- Compiled-first
- Hybrid execution: interpreter for fast iteration, LLVM for native builds
- Agent-native: tools, agent tasks, workflows, schemas, and LLM calls are part of the current programming model
- Focused on typed automation rather than only general-purpose scripting

## What Loz Is Not

Loz is not trying to replace Python, Rust, or C directly.

- Python remains valuable for the AI and data ecosystem
- Rust remains valuable for low-level systems work and runtime implementation
- C-level interop is a later goal, not the center of `v0.1.0-alpha`

## Why This Direction

Loz targets programs where a developer wants:

- stronger typing than typical glue code
- native compilation when needed
- a direct path to tools, schemas, agent tasks, workflows, and LLM-backed automation
- practical integration with Python instead of pretending the Python ecosystem does not exist

## v0.1.0-alpha Position

Loz `v0.1.0-alpha` is an experimental milestone.

- It already supports real end-to-end programs
- It is suitable for exploration, examples, and early feedback
- It is not production-stable yet

## Design Direction

The long-term direction is:

1. keep the language small enough to reason about
2. keep agent/workflow programming explicit rather than hidden behind framework magic
3. preserve a native executable path through LLVM
4. preserve access to the Python AI ecosystem through interop
