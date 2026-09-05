#!/usr/bin/env python3
"""Show the structure of a JSON file by replacing leaf values with their type names.

Groups dict keys that share the same value structure using fingerprinting,
showing one representative and listing the rest.
"""

import argparse
import json
import os
import sys

def get_fingerprint(v: object, depth: int = 0) -> str:
    if depth > 4:
        return type(v).__name__
    if v is None:
        return "null"
    elif isinstance(v, bool):
        return "bool"
    elif isinstance(v, int):
        return "int"
    elif isinstance(v, float):
        return "float"
    elif isinstance(v, str):
        return "string"
    elif isinstance(v, list):
        if len(v) == 0:
            return "list(0)[]"
        return f"list({len(v)})[{get_fingerprint(v[0], depth + 1)}]"
    elif isinstance(v, dict):
        sorted_keys = sorted(v.keys())
        inner = ", ".join(
            f"{k}: {get_fingerprint(v[k], depth + 1)}" for k in sorted_keys[:12]
        )
        suffix = ", ..." if len(v) > 12 else ""
        return "{" + inner + suffix + "}"
    return type(v).__name__

def to_skeleton(obj: object, max_depth: int = 20, depth: int = 0) -> object:
    if depth > max_depth:
        return "..."
    if obj is None:
        return "null"
    elif isinstance(obj, bool):
        return "bool"
    elif isinstance(obj, int):
        return "int"
    elif isinstance(obj, float):
        return "float"
    elif isinstance(obj, str):
        return "string"
    elif isinstance(obj, list):
        if len(obj) == 0:
            return "[] (0 items)"
        elem_skeleton = to_skeleton(obj[0], max_depth, depth + 1)
        return {f"[0..{len(obj)-1}] ({len(obj)} items, showing [0])": elem_skeleton}
    elif isinstance(obj, dict):
        if len(obj) == 0:
            return "{}"
        keys_list = list(obj.keys())
        groups: dict[str, list[str]] = {}
        for key in keys_list:
            fp = get_fingerprint(obj[key])
            groups.setdefault(fp, []).append(key)
        result = {}
        for fp, group_keys in groups.items():
            if len(group_keys) == 1:
                k = group_keys[0]
                result[k] = to_skeleton(obj[k], max_depth, depth + 1)
            else:
                rep = group_keys[0]
                if len(group_keys) <= 8:
                    others = group_keys[1:]
                else:
                    others = group_keys[1:4] + ["..."] + group_keys[-2:]
                label = f"{rep}  (+ {len(group_keys)-1} similar: {', '.join(str(o) for o in others)})"
                result[label] = to_skeleton(obj[rep], max_depth, depth + 1)
        return result
    return str(type(obj).__name__)

def get_json_structure(file_path: str) -> str:
    if not file_path or not os.path.exists(file_path):
        raise ValueError(f"File not found: {file_path}")

    with open(file_path, "r", encoding="utf-8") as f:
        data = json.load(f)

    skeleton = to_skeleton(data)
    formatted = json.dumps(skeleton, ensure_ascii=False, indent=2)
    return f"Structure of '{file_path}':\n\n{formatted}"

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Show the structure of a JSON file with type placeholders."
    )
    parser.add_argument("file_path", help="Path to the JSON file")

    args = parser.parse_args()

    try:
        result = get_json_structure(args.file_path)
        print(result)
    except (ValueError, json.JSONDecodeError) as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
