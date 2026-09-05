#!/usr/bin/env python3
"""Search for target_text in a file and return surrounding context (configurable character window).

If --filter-positions is provided, only return matches that fall inside the
corresponding sub_agent_calling_N blocks.
"""

import argparse
import json
import os
import sys

def search_context(
    file_path: str,
    target_text: str,
    window_before: int = 400,
    window_after: int = 400,
    filter_positions: list[int] | None = None,
) -> str:
    window_before = max(0, min(window_before, 2000))
    window_after = max(0, min(window_after, 2000))

    if not file_path or not os.path.exists(file_path):
        raise ValueError(f"File not found: {file_path}")
    if target_text == "":
        raise ValueError("target_text must not be empty.")

    with open(file_path, "r", encoding="utf-8", errors="replace") as f:
        content = f.read()

    all_matches_output: list[str] = []
    match_count = 0

    def search_text(search_content: str, matched_location: int | None = None) -> bool:
        nonlocal match_count
        total_len = len(search_content)
        current_pos = 0

        while True:
            match_index = search_content.find(target_text, current_pos)
            if match_index == -1:
                return False

            match_count += 1
            start_idx = max(0, match_index - window_before)
            end_idx = min(total_len, match_index + len(target_text) + window_after)
            snippet = search_content[start_idx:end_idx]
            line_number = search_content.count("\n", 0, match_index) + 1
            clean_snippet = snippet.replace("\n", "↩")
            location_info = (
                f" [sub_agent_calling_{matched_location}]"
                if matched_location is not None
                else ""
            )
            location_scope = (
                f" within sub_agent_calling_{matched_location}"
                if matched_location is not None
                else ""
            )
            line_label = "Block Line" if matched_location is not None else "Line"
            char_label = "Block char index" if matched_location is not None else "Char index"
            block = (
                f"=== Match #{match_count} (Approx. {line_label} {line_number}){location_info} ===\n"
                f"Location: {char_label} {match_index} to {match_index + len(target_text)}{location_scope}\n"
                f"Content Context:\n"
                f"...{clean_snippet}..."
            )
            all_matches_output.append(block)
            if len(all_matches_output) >= 20:
                all_matches_output.append("\n(Stopping after 20 matches)")
                return True

            current_pos = match_index + len(target_text)

    if filter_positions:
        data = json.loads(content)
        if not isinstance(data, dict):
            raise ValueError("filter_positions requires a top-level JSON object.")

        filtered_blocks: list[tuple[int, str]] = []
        for loc in filter_positions:
            key = f"sub_agent_calling_{loc}"
            if key in data:
                filtered_blocks.append(
                    (loc, json.dumps(data[key], ensure_ascii=False, indent=2))
                )

        if not filtered_blocks:
            return f"No matching sub_agent_calling blocks found for filter_positions {filter_positions}"

        for loc, block_content in filtered_blocks:
            if search_text(block_content, loc):
                break
    else:
        search_text(content)

    if match_count == 0:
        filter_msg = (
            f" within sub_agent_calling {filter_positions}" if filter_positions else ""
        )
        return f"Not found: '{target_text}'{filter_msg}"

    filter_info = (
        f" (filtered by positions: {filter_positions})" if filter_positions else ""
    )
    final_output = "\n\n".join(all_matches_output)
    return f"Found {match_count} matches in '{file_path}'{filter_info}. Showing {window_before} chars before and {window_after} chars after each match:\n\n{final_output}"

def main() -> None:
    parser = argparse.ArgumentParser(
        description="Search for text in a file and return surrounding context."
    )
    parser.add_argument("file_path", help="Path to the file to search")
    parser.add_argument("target_text", help="Text to search for")
    parser.add_argument(
        "--window-before",
        type=int,
        default=400,
        help="Characters of context before each match (default: 400, max: 2000)",
    )
    parser.add_argument(
        "--window-after",
        type=int,
        default=400,
        help="Characters of context after each match (default: 400, max: 2000)",
    )
    parser.add_argument(
        "--filter-positions",
        type=int,
        nargs="+",
        default=None,
        help="Only return matches inside these sub_agent_calling_N blocks",
    )

    args = parser.parse_args()

    try:
        result = search_context(
            file_path=args.file_path,
            target_text=args.target_text,
            window_before=args.window_before,
            window_after=args.window_after,
            filter_positions=args.filter_positions,
        )
        print(result)
    except ValueError as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
