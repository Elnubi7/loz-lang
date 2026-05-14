# Loz Compiler Architecture

Loz will be implemented in Rust. Native code generation is expected to target LLVM later, but the current project phase is language design only.

## Planned Compiler Pipeline

```text
.loz source
  -> lexer
  -> parser
  -> abstract syntax tree
  -> semantic analysis
  -> intermediate representation
  -> LLVM backend
  -> executable or build artifact
```

## Stage Responsibilities

### Lexer

The lexer converts source text into tokens such as keywords, identifiers, literals, operators, and punctuation.

Source:

```loz
const value: Int = 10;
```

Representative tokens:

```text
CONST
IDENTIFIER(value)
COLON
IDENTIFIER(Int)
EQUAL
INTEGER(10)
SEMICOLON
```

### Parser

The parser turns the token stream into structured syntax nodes.

Example:

```loz
func add(a: Int, b: Int) -> Int {
    return a + b;
}
```

This becomes a function declaration with typed parameters, a return type, and a block body.

### Abstract Syntax Tree

The AST should represent the stable shape of the program. Likely top-level categories include:

- declarations
- statements
- expressions
- types

### Semantic Analysis

The semantic phase checks meaning rather than raw syntax. It is responsible for rules such as:

- names must be declared before valid use
- assignments must respect mutability
- expressions must type-check
- function calls must match signatures
- struct field access must be valid
- reference usage must follow safe language rules

Example of a semantic error:

```loz
const total: Int = 10;
total = 12;
```

This should fail because `total` is immutable.

### Intermediate Representation

An internal IR will likely simplify later optimization and backend work. The exact IR design does not need to be fixed yet, but it should reflect Loz semantics clearly rather than mirror source text too closely.

### LLVM Backend

LLVM is the planned backend target, not the current implementation milestone. The project should not commit to backend details before the front-end model is stable.

## CLI Direction

The CLI name is `loz`. A likely long-term command shape is:

```bash
loz check app.loz
loz build app.loz
loz run app.loz
```

These commands are planning targets only. They are not being implemented in this phase.
