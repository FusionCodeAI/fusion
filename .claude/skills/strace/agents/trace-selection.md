# Trace Selection Agent

Statistically analyze a trace collection and select the most failure-prone components and representative traces for deeper investigation.

## Inputs

- **traces_dir**: directory containing execution trace files
- **system_structure**: path to `output/dependency_prior_analysis.md`, or a description of the system structure provided inline
- **output_dir**: where to write results (default: `output/`)

## Process

Knowledge of the agent system structure — specifically:
  - What components (agents/tools) exist and their roles (decision-maker vs. executor)
  - Data dependencies: which components consume outputs produced by others
  - Control dependencies: which components decide whether/how others run

  This structural knowledge can come from any source: a prior environment modeling step, a README,
  source code inspection, or the user describing the system. If none is available, infer it by
  sampling a few traces to identify component names and rough sequencing.

## Step 1: Statistical Failure Analysis

### 1a. Understand the trace structure

Before writing any code, sample 2-3 trace files to understand their JSON structure. Identify:
- How success/failure is recorded
- The sequence of agent/tool calls and their outcomes
- How "state changes" are signaled (acceptance criteria, score changes, flags)
- The names used for each component

### 1b. Write (or improve) `trace_analyzer.py`

If `trace_analyzer.py` already exists, read it first and assess whether it handles the current trace structure correctly. If not, write it from scratch.

The script must produce two output files:

**`output/trace_summaries.json`** — per-trace execution sequences:
```json
{
  "trace_0001.json": [
    {"position": 1, "subagent": "router", "outcome": "selected: executor_A", "state_changed": false},
    {"position": 2, "subagent": "executor_A", "outcome": "failed: <reason>", "state_changed": false},
    ...
  ]
}
```
For decision/router nodes, `outcome` must include which downstream component was selected. Include a `state_changed` boolean indicating whether this position updated shared state.

**`output/trace_analysis_report.md`** — essential statistical findings (max 500 words):
- Total traces, success rate
- Per-component: appearance count, failure count, failure rate
- Most common failure patterns
- Any notable cascade patterns

### 1c. Execute the script

Run `python trace_analyzer.py` (or `python3`). If it errors, fix and rerun.

### Output Constraints

- DO NOT write lengthy explanations or verbose commentary
- DO NOT repeat or paraphrase trace contents in your responses
- KEEP analysis concise — focus on key statistics and patterns
- All output files go in the `output/` directory

## Step 2: High-Priority Component Selection

Using `output/trace_summaries.json` and `output/trace_analysis_report.md`:

### 2a. List all active components

Use the system structure knowledge from the Input — include all decision-makers and executors.
If structural knowledge is incomplete, infer component roles from the trace data itself (e.g., a component
that always appears before others and determines which runs next is likely a decision-maker).

### 2b. Score each component

For each component, compute:
- **Failure frequency**: how often it appears in failed traces vs. total appearances
- **Cascade impact**: using the dependency map, how many downstream components depend on it? A high-fan-out upstream component that fails frequently causes more total damage than a leaf component with the same failure rate.

Weight: `priority_score = failure_frequency × (1 + downstream_dependency_count)`

### 2c. Select 1-5 high-priority components

Choose the top scorers. Use judgment — don't mechanically pick the top 5 if #4 and #5 have negligible scores. Brief justification for each selection is sufficient (1-2 sentences).

### 2d. Select representative traces per component

For each high-priority component, select ≤5 traces that best represent its distinct failure patterns.

**Criteria:**
- Each trace should represent a **distinct failure mode** — don't pick 3 traces with identical errors
- Prioritize traces that reveal **significant, generalizable issues** over edge cases
- If 2-3 traces already cover all major failure patterns for a component, stop — don't fill the quota artificially

Use the trace summaries to group traces by error type or failure pattern for each component, then pick one
representative per group.

### Output

Save `output/high_priority_components.json`:
```json
{
  "selected_components": ["component_A", "component_B", "component_C"],
  "rationale": {
    "component_A": "Appears in 73% of failed traces; controls routing to all downstream executors",
    "component_B": "42% failure rate; produces the shared state that 4 other components consume"
  },
  "representative_traces": {
    "component_A": ["trace_0001.json", "trace_0023.json"],
    "component_B": ["trace_0005.json", "trace_0012.json", "trace_0044.json"]
  }
}
```

Also write a ranked list in your response (brief, no need to repeat what's in the JSON).

## Notes

- Create `output/` directory if it doesn't exist
- If traces are very large (>1000 files), sample a representative subset rather than reading all of them — but note the sampling in the report
- The `state_changed` field in `trace_summaries.json` is critical for downstream causal slicing — get this right. Use the system structure knowledge to determine what "state change" means in this system (e.g., accepted outputs that update shared state vs. rejected attempts that do not)
