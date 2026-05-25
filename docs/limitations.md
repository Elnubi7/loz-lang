# ⚠️ Current Loz Limitations

Loz is promising, but it is still an Alpha-stage language project. This document is intentionally direct so users and contributors know what to expect.

## Release Status

- Loz is Alpha.
- Loz is experimental.
- Syntax may change.
- Strong backward compatibility is not guaranteed yet.

## Language And Tooling Boundaries

- The core language is usable for examples and experimentation, but it is not yet positioned as production-stable.
- Documentation and examples focus on the currently validated behavior rather than a frozen specification.
- Some syntax and runtime areas are still evolving and should be treated as subject to change.

## Native Build Scope

- Native build is currently Linux-first in terms of validation and project confidence.
- Native output depends on an installed LLVM and native linker toolchain.
- If the system C development toolchain is incomplete, native linking can fail even when `loz check` and `loz run` work.

## Standard Library And Runtime

- The standard library surface is still minimal.
- Runtime-oriented features such as JSON, schemas, Python interop, workflows, agents, and LLM support exist, but they should be treated as evolving rather than fully hardened.
- External runtime integrations depend on local tools, environment variables, or external services.

## Packaging And Project Workflows

- Local path dependencies exist, but packaging is not stable yet.
- The project configuration flow is still early and subject to revision.
- Release packaging and distribution workflows are not finalized.

## Editor And Ecosystem

- The VS Code extension is intentionally lightweight.
- The project ecosystem around Loz is still small.
- Documentation is improving quickly, but some older docs may lag implementation if they are not actively maintained.

## Compatibility Expectations

At this stage you should assume:

- example programs are the most reliable reference points,
- CLI behavior documented in the current repo is the source of truth,
- public Alpha polish is still underway.

## Recommended Mindset For Alpha Users

- Use Loz for exploration, experimentation, and feedback.
- Validate native builds on your target environment instead of assuming cross-platform parity.
- Run the repository validation commands before relying on behavior:

```bash
cargo fmt
cargo check --workspace
cargo test --workspace
cargo build --workspace
./scripts/test_native_examples.sh
```
