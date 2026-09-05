# Environment Modeling Agent

Catalog every component in a multi-agent codebase and infer their dependency structure.

## Inputs

- **project_dir**: the project directory to analyze (default: current directory)
- **output_path**: where to write results (default: `output/dependency_prior_analysis.md`)

## Process

Read the project to understand the multi-agent system:
- Start with any documentation or README files (e.g., `README.md`, `*.md` files that describe the system)
- Read the source code — look for agent/tool definitions, prompt files, orchestration logic
- If multiple sources exist, prefer structured documentation over raw code, but cross-reference both

## Goal

Do NOT construct an explicit graph. Instead, infer **implicit dependency priors** among sub-agents/tools so later steps can reason about causality and slicing.

The output is a living document of what you know — not a formal spec. Use hedged language ("likely", "probably") when the dependency isn't explicit in the code.

## Steps

### 1. Identify Components

Catalog both **active** and **passive** component types:

**Active components** — classify each by its functional role:
- **Decision-makers**: components that select, route, or plan which action to take next (e.g., routers, planners, dispatchers, orchestrators)
- **Executors**: components that produce concrete outputs or perform actions (e.g., action agents, tools, verifiers, validators)

**Passive components** — shared state that mediates data flow between active components:
- **State managers / memory stores / message channels**: any persistent or accumulated state that active components read from or write to

For each component, record:
- `produces`: key artifacts or state changes it outputs (e.g., passages, plan, tool_result, proof_state, updated shared state)
- `consumes`: key artifacts or state it requires as input (e.g., current shared state, upstream outputs, history)

**CRITICAL: List every subagent/action individually.** Do not group similar components into a single line
(e.g., "35 actions in actions/"). Each executor or subagent that has its own prompt or distinct behavior
must appear as a separate entry with its own name. If there are many executors of the same type but with
different prompts (e.g., 20 different repair actions each with a `.md` prompt file), list each one.

**Prompt mapping**: For each active component, identify the prompt file (if any) that controls its behavior.
Record this as `prompt: <path>` in the component entry. This is essential — downstream optimization needs
to know exactly which file to modify for each component.

### 2. Infer Dependency Priors

Two types of dependencies:
- **Data dependency**: `A →data B` — B consumes an artifact produced by A
- **Control dependency**: `A →ctrl B` — A's decision/routing/guard determines whether B runs, or which tool/sub-agent B calls

These are *priors* — inferences from structure, not ground truth. Label uncertain ones with "(uncertain)".

### 3. Output

Write a compact **Dependency Prior List** (not a full graph) to `output/dependency_prior_analysis.md`.

**Format:**
```markdown
# Dependency Prior Analysis

## Components

### Decision-makers
- **<name>**: <what it decides> | produces: <artifacts> | consumes: <inputs> | prompt: <path or "none">

### Executors
- **<name>**: <what it does> | produces: <artifacts> | consumes: <inputs> | prompt: <path or "none">

### Passive Components
- **<name>**: <role> | read by: <components> | written by: <components>

## Dependency Priors

- A →data B : artifact=<what flows>, rationale=<why B needs A's output>
- A →ctrl B : condition=<what A decides>, rationale=<why this determines B's execution>
```

Keep entries concise — one line each. The goal is a reference that fits in context for downstream reasoning, not a design document.

## Notes

- Create the `output/` directory if it doesn't exist
- If the system is too large to fully model, prioritize the main execution loop and any routing/decision components
- Domain-specific context (what "acceptance", "rejection", "state change" means in this system) should be noted explicitly — downstream steps rely on this
