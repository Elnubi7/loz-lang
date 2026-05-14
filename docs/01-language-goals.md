# Loz Language Goals

## Primary Goal

Loz is meant to provide a small, coherent language core that is safe, statically checked, and easier to approach than many systems languages.

The project goal is not novelty for its own sake. The goal is to combine a few proven ideas into a language that stays readable, predictable, and teachable.

## Problems Loz Tries To Avoid

### JavaScript

Loz tries to avoid several issues that commonly make JavaScript codebases harder to reason about at scale:

- implicit coercion
- values that are missing in multiple ways
- weak compile-time guarantees
- surprising truthiness behavior
- bugs that are only discovered under specific runtime paths

Example of behavior Loz explicitly does not want:

```javascript
"5" + 5 // "55"
```

Loz prefers compile-time rejection over runtime guessing.

### Python

Loz tries to avoid common Python failure modes in larger systems:

- type mismatches discovered only when code executes
- interface assumptions hidden in documentation rather than enforced by the language
- inconsistent mutation patterns in loosely structured code
- performance ceilings that become visible late in a project

Loz keeps syntax direct, but it expects more information from the programmer up front so mistakes can be caught earlier.

### Rust

Loz is not trying to compete with Rust on systems-level control. It is trying to avoid some of the friction that Rust introduces for teams that want a simpler language surface:

- heavy ownership and lifetime concepts in everyday code
- exposure to low-level unsafe tools
- a learning curve that can slow down early experimentation

Loz still values safety, but it aims to present that safety through a smaller and more guided v1 model.

## Design Principles

- Favor explicitness over convenience when the two conflict.
- Make mutation visible in source code.
- Make absence explicit with `Maybe<T>`.
- Make failure explicit with `Result<T, E>`.
- Prefer a small set of predictable rules over many advanced features.
- Keep the language implementation realistic for a first compiler written in Rust.

## What Success Looks Like For v1

Loz v1 is successful if it can support small to medium programs with:

- clear function definitions
- predictable control flow
- static typing with a compact set of primitive and collection types
- simple user-defined structs
- safe references without raw pointers
- explicit error and optional-value handling
