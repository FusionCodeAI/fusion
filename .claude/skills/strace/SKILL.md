---
name: strace
description: >
  Optimize a multi-agent system's prompts or other components using trajectory data, 
  improve performance while reducing analysis cost — even at large scale.
  Use when the user wants to improve agent performance, revise prompts, or diagnose failures,
  and has execution traces available for analysis. Works at any scale: from a handful of traces to
  large collections. Do NOT use for building new agents from scratch, or when no traces exist.
---

# strace — Multi-Agent System Trace Analysis & Repair Pipeline

strace is a skill with a four-stage pipeline for diagnosing and improving multi-agent systems. 
Each stage is run as an independent subagent using instructions from `agents/`. 
Stages pass context forward via files written to `output/`.

## Step 0 — Triage (always run this first)

Before spawning any subagents, assess the situation and decide which stages to run.

**Check these signals:**

| Signal | Implication |
|--------|-------------|
| `output/dependency_prior_analysis.md` exists | Skip Stage 1 |
| `output/high_priority_components.json` exists | Skip Stage 2 |
| Only 1 trace provided, or user points to a specific trace | Skip Stage 2 — go directly to Stage 3 |
| User already described which component is failing | Skip Stage 1 and Stage 2; treat user's description as system structure |
| User says "just fix the prompts" with attribution results available | Skip Stages 1–3, run Stage 4 only |
| User asks about manifestation node / "where does it fail?" | Note: this is answered as part of Stage 3 — no separate step needed |

**State your plan explicitly** before proceeding. Example:
> "You have 1 trace and no prior output. I'll run Stage 1 to map the system, then skip Stage 2 and
> go directly to Stage 3 to attribute the root cause. Stage 4 only if you want prompt fixes."

Then execute that plan. Do not enumerate possibilities — just commit to a path.

**Do NOT read agent files before completing triage.** Only load `agents/<stage>.md` immediately
before executing that stage — skip loading it entirely if the stage is not needed.

---

## Pipeline Overview

```
Stage 1: Environment-Modeling    →  output/dependency_prior_analysis.md
Stage 2: Trace-Selection         →  output/high_priority_components.json
                                    output/trace_summaries.json
Stage 3: Self-Debugging & Root Cause Attribution  →  output/attributions/<trace>.json  (one per trace)
Stage 4: Harness-Engineering         →  output/prompts_revised/<agent>_revised.md
```

Stages share context via files in `output/`. Each stage can be skipped if its output already exists
or the situation makes it unnecessary (see Step 0).

---

## Stage 1 — Environment-Modeling
*instructions: `agents/agent-env-modeling.md`*

Reads source code, prompts, and docs to catalog every component (decision-makers, executors, passive
state), infer data and control dependencies, and write a compact dependency prior list.

**Skip if**: `output/dependency_prior_analysis.md` already exists and reflects current codebase.  
**Output**: `output/dependency_prior_analysis.md`

---

## Stage 2 — Trace-Selection
*instructions: `agents/trace-selection.md`*

Statistically analyzes all traces to find the worst-performing components (failure rate × downstream
impact), then selects ≤5 representative traces per component covering distinct failure patterns.

**Skip if**: only small number of traces provided, user already knows which component to investigate, or
`output/high_priority_components.json` already exists.  
**Output**: `output/trace_summaries.json`, `output/high_priority_components.json`, `output/trace_analysis_report.md`

---

## Stage 3 — Self-Debugging & Root Cause Attribution
*instructions: `agents/trace-self-debug.md`*

For each trace, backtracks from the manifestation node to find the upstream component that actually
caused the failure. Spawn one subagent per trace; they can run in parallel.

**Skip if**: attribution results already exist in `output/attributions/`.  
**Output**: `output/attributions/<trace_filename>.json` per trace

After all subagents complete, summarize which components appear most as root causes — this is the
input to Stage 4.

---

## Stage 4 — Harness-Engineering
*instructions: `agents/harness-engineering.md`*

Synthesizes attribution results into targeted prompt edits. Produces gradient blocks per trace,
then revised prompt files. Validates that decision-maker / executor boundaries are not violated.

**Skip if**: user only wants diagnosis, not prompt fixes.  
**Output**: `output/prompts_revised/<name>_revised.md`, `output/prompt_mapping.json`

---

## Trace Scripts

Three utility scripts in `scripts/` for reading and searching trace files. Use these instead of
loading full trace files — they handle truncation, structure inspection, and targeted search.

When spawning subagents for Stages 2–4, include the relevant script usage in the subagent prompt
so it knows how to read traces efficiently.

### `scripts/get_json_structure.py` — Inspect trace structure

Show the schema of a JSON file with type placeholders. Use this first on an unfamiliar trace
to understand its format before reading specific positions.

```bash
python scripts/get_json_structure.py <file_path>
```

### `scripts/read_trace_positions.py` — Read specific positions

Read specific 1-based positions from a trace file. Supports dict-keyed traces
(`sub_agent_calling_N`, `step_N`, etc.) and list-based traces. Long strings are
automatically truncated.

```bash
python scripts/read_trace_positions.py <file_path> <pos1> <pos2> ...
python scripts/read_trace_positions.py trace.json 1 3 5 --max-string-length 3000
```

### `scripts/search_context_in_file.py` — Search with context

Search for text in a trace file and return surrounding context. Optionally filter
to specific `sub_agent_calling_N` blocks.

```bash
python scripts/search_context_in_file.py <file_path> <target_text>
python scripts/search_context_in_file.py trace.json "error" --window-before 600 --window-after 600
python scripts/search_context_in_file.py trace.json "failed" --filter-positions 2 5 8
```

---

## Notes

- All output files land in `output/` — create it if it doesn't exist
- Each subagent is stateless; pass context explicitly (file paths, relevant file contents) in the prompt
- For Stage 3, parallel subagents are preferred — spawn all trace attribution agents in the same turn
- After Stage 4, ask the user whether they want to apply the revised prompts to their codebase
  (copy `output/prompts_revised/` → original prompt locations)

## Agents

- `agents/agent-env-modeling.md` — Stage 1
- `agents/trace-selection.md` — Stage 2
- `agents/trace-self-debug.md` — Stage 3
- `agents/harness-engineering.md` — Stage 4
