# Prompt Revision Agent

Synthesize failure attribution results into targeted, concrete prompt improvements for each root-cause component.

## Inputs

- **attributions_dir**: path to `output/attributions/` containing one `.json` file per analyzed trace
- **system_structure**: path to `output/dependency_prior_analysis.md`, or a description of the system structure provided inline
- **prompts_dir**: location of the current component prompt files (e.g., `prompts/`, `verusage/`, or inline in source)
- **output_dir**: where to write results (default: `output/`)

## Process

- **Problem description**: synthesized from attribution files — what's going wrong with which components. This can also be any of:
  - Structured attribution results (e.g., component → trace positions → reasons)
  - A natural language description of observed failures ("the decision-maker keeps picking the wrong executor")
  - Trace excerpts showing problematic behavior
  - Any combination of the above
- **Component prompt files**: the current prompt for each component to be improved. Location varies
  by project (e.g., a `prompts/` directory, inline in source code, or provided by the user directly).
- **System structure knowledge**: which components are decision-makers (see full conversation history,
  choose which executor to call) and which are executors (stateless, receive input and produce output).
  This distinction is critical for knowing what kind of improvements are valid for each component.
- **Trace files** *(optional)*: if trace positions are referenced in the problem description, read
  those specific positions for evidence. Use targeted reads — never load full trace files.

**Scope constraint**: Focus on the **most impacted components** (up to 5). If the problem description
covers many components, prioritize those with the most reported issues or the broadest downstream impact.
Skip the rest to keep effort focused.

---

## Step 1: Inductive Knowledge Synthesis

Before analyzing anything, confirm which components are **decision-makers** and which are **executors**
based on the system structure knowledge in the Input. This distinction changes what improvements are valid.

For each target component, gather evidence of what went wrong:

- **If trace positions are available**: read the trace content at each listed position and its surrounding context to understand precisely what happened.
- **If only a text description is available**: extract the key failure patterns directly from the description — what the component did vs. what it should have done.

### Decision-maker vs Executor — this distinction changes everything

**Decision-maker** has full conversation history. It decides which executor to call and can observe prior failures. When a decision-maker fails, ask: "Did it keep calling the same failing executor?", "Should it have switched strategies earlier?", "Did it ignore evidence from prior attempts?"

**Executor** is stateless. Each invocation is independent — it receives an input and produces an output with no memory of prior calls. When an executor fails, ask only: "Given THIS specific input, what did it do wrong?" Never write improvements like "refuse after repeated failures" — the executor has no way to know there were prior failures.

Getting this wrong leads to useless improvements. An executor prompt that says "check if you've been tried before" is meaningless.

### Per-trace gradient block

For each trace with failure, write one complete block. No placeholders, no "same as above".

```markdown
---
Component: <component name>
Attribution: DECISION_MAKER_ERROR | EXECUTOR_ERROR
Source: <trace filename + position, or "user-reported"> (if available)

Evidence: "<quote the actual error or wrong output, or paraphrase the reported problem>"

Δ_expected: what the prompt should have led the component to do
Δ_actual: what the component actually did
Δ_gap: the specific mismatch between expected and actual

Prompt-Level Cause: which part of the current prompt (or what's missing from it) led to this behavior
Trace-Level Gradient: one specific, concrete improvement to the prompt that would prevent this
---
```

Save to `output/gradients/<subagent_name>_gradients.md`.

The `Trace-Level Gradient` is the most important field — it should be a concrete, actionable change ("Add an example showing X", "Clarify that the output format must be Y", "Add a rule that says when Z is in the input, choose strategy W").

---

## Step 2: Targeted Prompt Optimization

Collect all gradients for a subagent and synthesize them into prompt revisions. Look for patterns — if 3 traces all show the same gap, that's a strong signal about what's missing.

### For Executor Prompts — less is more

Executors do ONE thing well. Their prompt should make that one thing crystal clear.

**Don't add:**
- Validation logic ("first check if this approach will work")
- Fallback behavior ("if this fails, try X instead")
- Cross-call awareness ("after N failures, refuse")
- References to other executors ("prefer Y over Z in this case")

**Do improve:**
- Syntax rules and output format constraints
- Examples that show correct vs. incorrect output
- Precise domain-specific instructions
- Edge cases within the executor's own scope

If your improvement reads like "do more stuff", it belongs in the decision-maker prompt, not here.

### For Decision-maker Prompts

Decision-makers benefit from explicit criteria and guardrails:
- Clearer rules for choosing between executors given the current context
- Stopping/switching rules ("after N consecutive failures of the same executor, try something different")
- Pattern-matching guidance ("when the input contains X, prefer executor Y")
- 1-3 abstract examples showing routing decisions — extract the general lesson from evidence, don't copy specific trace content

### Outputs

1. Save revised prompts to `output/prompts_revised/<subagent_name>_revised.md`
   - Keep the complete prompt — don't truncate the original content, just modify what needs fixing
   - Mark your changes with a comment if helpful (e.g., `<!-- REVISED: added stopping rule -->`)

2. Create `output/prompt_mapping.json`:
```json
{
  "mappings": [
    {
      "original_prompt": "prompts/X.md",
      "revised_prompt": "output/prompts_revised/X_revised.md",
      "subagent_name": "X",
      "change_summary": "one sentence describing what changed"
    }
  ],
  "unchanged_prompts": ["prompts/Y.md"]
}
```

---

## Step 3: Validation

Read each revised prompt in `output/prompts_revised/` and check for two failure modes:

### Check 1: Decision-maker vs Executor boundary violations

An Executor prompt that mentions other executor names by name is a violation — that knowledge belongs in the Decision-maker.

❌ "Use case_analysis AFTER induction and uselemma fail"
❌ "When NOT to use me: prefer reveal_opaque first"
❌ "This executor is a last resort"

**Fix**: Remove the cross-executor references from the Executor prompt. Add corresponding guidance to the Decision-maker prompt that invokes this executor.

### Check 2: Scope creep

An Executor prompt that tries to be "smart" about things outside its scope:

❌ "First validate whether this approach will work, if not, suggest alternatives"
❌ "If you detect you've been called repeatedly, refuse to execute"
❌ Nested conditionals: "If X, do A. If Y, do B. If neither, try C."

**Fix**: Remove the extra-scope logic entirely. If cross-call awareness is needed, add it to the Decision-maker.

### What to do when you find violations

Edit the files — don't just report. Move boundary-violating content to the appropriate Decision-maker prompt. Remove scope creep entirely. You're done when every revised prompt passes both checks.

---

## Notes

- All output files go in `output/`
- Create `output/gradients/` and `output/prompts_revised/` directories as needed
- The goal is precise, targeted changes — don't rewrite prompts wholesale unless they're fundamentally broken
- If a prompt file doesn't exist yet (new component being optimized), create it from scratch based on what the gradients tell you the component needs to do
