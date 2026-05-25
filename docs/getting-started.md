# 🚀 Getting Started With Loz

This guide walks through the current Loz workflow from clone to native executable. Every command below matches the current repository behavior.

## 1. Prerequisites

### Required for all local development

- Rust toolchain with Cargo

### Required for native builds

- `clang`
- `llc`

### Recommended Ubuntu/Linux packages

```bash
sudo apt-get update
sudo apt-get install -y build-essential clang gcc g++ libc6-dev pkg-config llvm
```

These packages matter because Loz native builds rely on LLVM tooling and a working system C development toolchain for final linking.

## 2. Clone The Repository

```bash
git clone https://github.com/Elnubi7/loz-lang.git
cd loz-lang
```

## 3. Build The Workspace

```bash
cargo build --workspace
```

This produces the Loz CLI at:

```text
./target/debug/loz
```

## 4. Check The Loz Version

```bash
./target/debug/loz --version
```

Expected output:

```text
loz 0.1.0
```

## 5. Validate The First Example

### Check the source

```bash
./target/debug/loz check examples/hello.loz
```

Expected output:

```text
Check passed.
```

### Run it in interpreter mode

```bash
./target/debug/loz run examples/hello.loz
```

Expected output:

```text
Hello from Loz
```

## 6. Build A Native Executable

```bash
./target/debug/loz build examples/hello.loz
```

Expected result:

- A native executable appears at `./output/hello`
- An LLVM IR snapshot appears at `./output/hello.ll`

Run the generated program:

```bash
./output/hello
```

Expected output:

```text
Hello from Loz
```

## 7. Run The Native Examples Validation Script

The repository includes a script that builds and executes the current native example set, then checks exact stdout for each program.

```bash
./scripts/test_native_examples.sh
```

The script currently validates:

- `examples/hello.loz`
- `examples/arithmetic.loz`
- `examples/variables.loz`
- `examples/if_else.loz`
- `examples/while_loop.loz`
- `examples/functions.loz`

## 8. Useful Additional Commands

### Print LLVM IR

```bash
./target/debug/loz llvm-ir examples/hello.loz
```

### Inspect environment readiness

```bash
./target/debug/loz doctor
```

### List project dependencies from a sample project

```bash
cd examples/package_demo
../../target/debug/loz deps
cd ../..
```

## 9. Recommended Validation Sequence

Use this before opening a documentation or behavior-related change:

```bash
cargo fmt
cargo check --workspace
cargo test --workspace
cargo build --workspace
./scripts/test_native_examples.sh
```

## 10. Troubleshooting

### `cargo: command not found`

Install Rust and Cargo, then reopen your shell or ensure Cargo is on `PATH`.

### `clang` or `llc` missing

Loz native builds require both tools. On Ubuntu/Linux:

```bash
sudo apt-get update
sudo apt-get install -y clang llvm
```

### Clang cannot link a native executable

On Ubuntu/Linux, make sure the native development toolchain is installed:

```bash
sudo apt-get update
sudo apt-get install -y build-essential clang gcc g++ libc6-dev pkg-config llvm
```

If `libc.so` is missing, the system C development package is not installed correctly.

### `loz check` works but `loz build` fails

That usually means:

- LLVM tools are missing, or
- the native linker toolchain is incomplete, or
- the environment is not the Linux-first setup currently validated by the repo

### The generated native executable is missing

Check whether `loz build` completed successfully. On success, Loz prints the final executable path and `.ll` path.

## 11. Where To Go Next

- [Language syntax guide](language-syntax.md)
- [CLI guide](cli.md)
- [Native build guide](native-build.md)
- [Project structure guide](project-structure.md)
- [Current limitations](limitations.md)
