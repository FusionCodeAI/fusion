#!/usr/bin/env python3
"""Read specific positions (entries) from a trace JSON file with smart truncation.

Works with any trace format: dict with numbered keys
(e.g., 'step_1', 'position_2', 'sub_agent_calling_3') or a top-level list.
Positions are 1-based.
"""

import argparse
import json
import os
import sys

def _truncate_strings(obj: object, max_len: int) -> object:
    """Recursively truncate long strings in a JSON object."""
    if isinstance(obj, str):
        if len(obj) > max_len:
            half = max_len // 2
            return (
                obj[:half]
                + f"\n... [{len(obj) - max_len} chars truncated] ...\n"
                + obj[-half:]
            )
        return obj
    elif isinstance(obj, dict):
        return {k: _truncate_strings(v, max_len) for k, v in obj.items()}
    elif isinstance(obj, list):
        return [_truncate_strings(item, max_len) for item in obj]
    return obj

def read_trace_positions(
    file_path: str,
    positions: list[int],
    max_string_length: int = 2000,
) -> str:
    if not file_path or not os.path.exists(file_path):
        raise ValueError(f"File does not exist: {file_path}")
    if not positions:
        raise ValueError("No positions provided.")
    if max_string_length < 2:
        raise ValueError("max_string_length must be at least 2.")

    max_len = max_string_length

    with open(file_path, "r", encoding="utf-8") as f:
        data = json.load(f)

    entries: dict[int, tuple[str | None, object]] = {}

    if isinstance(data, list):
        total = len(data)
        for pos in positions:
            idx = pos - 1
            if 0 <= idx < total:
                entries[pos] = (f"index {idx}", data[idx])
            else:
                entries[pos] = (None, None)

    elif isinstance(data, dict):
        keys_list = list(data.keys())
        total = len(keys_list)
        key_patterns = [
            lambda n: f"sub_agent_calling_{n}",
            lambda n: f"step_{n}",
            lambda n: f"position_{n}",
            lambda n: f"node_{n}",
            lambda n: f"segment_{n}",
            lambda n: str(n),
        ]
        for pos in positions:
            found = False
            for pattern_fn in key_patterns:
                key = pattern_fn(pos)
                if key in data:
                    entries[pos] = (key, data[key])
                    found = True
                    break
            if not found:
                idx = pos - 1
                if 0 <= idx < total:
                    key = keys_list[idx]
                    entries[pos] = (f"{key} (by order)", data[key])
                else:
                    entries[pos] = (None, None)
    else:
        raise ValueError(f"Top-level JSON is {type(data).__name__}, expected dict or list.")

    result_blocks: list[str] = []
    for pos in sorted(positions):
        key_label, value = entries.get(pos, (None, None))
        if key_label is None:
            result_blocks.append(
                f"=== Position {pos} ===\n⚠️ Not found (total entries: {total})"
            )
            continue
        truncated_value = _truncate_strings(value, max_len)
        formatted = json.dumps(truncated_value, ensure_ascii=False, indent=2)
        result_blocks.append(f"=== Position {pos} [{key_label}] ===\n{formatted}")

    return (
        f"File: {os.path.basename(file_path)} ({total} entries)\n\n"
        + "\n\n".join(result_blocks)
    )

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Read specific positions from a trace JSON file."
    )
    parser.add_argument("file_path", help="Path to the trace JSON file")
    parser.add_argument(
        "positions",
        type=int,
        nargs="+",
        help="1-based position numbers to read",
    )
    parser.add_argument(
        "--max-string-length",
        type=int,
        default=2000,
        help="Truncate strings longer than this (default: 2000, min: 2)",
    )

    args = parser.parse_args()

    try:
        result = read_trace_positions(
            file_path=args.file_path,
            positions=args.positions,
            max_string_length=args.max_string_length,
        )
        print(result)
    except (ValueError, json.JSONDecodeError) as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
