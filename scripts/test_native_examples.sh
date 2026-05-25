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

test_example() {
    local name="$1"
    local expected="$2"

    ./target/debug/loz build "examples/${name}.loz"
    local actual
    actual="$(./output/${name})"
    assert_stdout "$expected" "$actual" "examples/${name}.loz"
}

test_example "hello" "Hello from Loz"
test_example "arithmetic" "30"
test_example "variables" "42"
test_example "if_else" "yes"
test_example "while_loop" "15"
test_example "functions" "25"
