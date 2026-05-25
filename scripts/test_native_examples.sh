#!/usr/bin/env bash
set -euo pipefail

assert_stdout() {
    local expected="$1"
    local actual="$2"
    local label="$3"

    if [[ "$actual" != "$expected" ]]; then
        printf 'unexpected stdout for %s\nexpected: %s\nactual: %s\n' "$label" "$expected" "$actual" >&2
        exit 1
    fi
}

cargo build --workspace

./target/debug/loz build examples/hello.loz
hello_output="$(./output/hello)"
assert_stdout "Hello from Loz" "$hello_output" "examples/hello.loz"

./target/debug/loz build examples/arithmetic.loz
arithmetic_output="$(./output/arithmetic)"
assert_stdout "30" "$arithmetic_output" "examples/arithmetic.loz"
