#!/usr/bin/env python3

import importlib
import json
import os
import sys
import traceback


def main() -> int:
    if len(sys.argv) != 2:
        print(
            f"python bridge expects 1 function path argument, found {len(sys.argv) - 1}",
            file=sys.stderr,
        )
        return 1

    function_path = sys.argv[1]
    current_dir = os.getcwd()
    if current_dir and current_dir not in sys.path:
        sys.path.insert(0, current_dir)

    module_name, separator, function_name = function_path.rpartition(".")
    if not separator or not module_name or not function_name:
        print(
            f"invalid python.call path '{function_path}'; expected module.function",
            file=sys.stderr,
        )
        return 1

    try:
        module = importlib.import_module(module_name)
    except Exception as error:
        print(f"failed to import Python module '{module_name}': {error}", file=sys.stderr)
        return 1

    try:
        function = getattr(module, function_name)
    except AttributeError:
        print(
            f"Python module '{module_name}' has no function '{function_name}'",
            file=sys.stderr,
        )
        return 1

    if not callable(function):
        print(
            f"Python attribute '{function_path}' is not callable",
            file=sys.stderr,
        )
        return 1

    payload_text = sys.stdin.read()
    try:
        payload = json.loads(payload_text)
    except json.JSONDecodeError as error:
        print(f"failed to parse JSON payload: {error}", file=sys.stderr)
        return 1

    try:
        result = function(payload)
    except Exception as error:
        print(f"Python function '{function_path}' raised: {error}", file=sys.stderr)
        traceback.print_exc(file=sys.stderr)
        return 1

    try:
        json.dump(result, sys.stdout, separators=(",", ":"))
    except TypeError as error:
        print(
            f"Python function '{function_path}' returned a non-JSON-serializable value: {error}",
            file=sys.stderr,
        )
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
