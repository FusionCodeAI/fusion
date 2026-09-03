# Multi-Agent Mesh & Advisors

## Multi-Agent Mesh (Parallel Subagents)

Fusion delegates complex, multi-stage engineering tasks to specialized background subagents that execute concurrently without blocking the primary loop:

```text
                  +------------------------+
                  |   Lead Agent Runner    |
                  +-----------+------------+
                              |
            +-----------------+-----------------+
            v                 v                 v
   +----------------+ +----------------+ +------------------+
   | Scout Subagent | | Coder Subagent | | Tester Subagent  |
   | (Read / Grep)  | | (Edit / Write) | | (Bash / Verify)  |
   +----------------+ +----------------+ +------------------+
```

- **`Scout`**: Fast, read-only exploration specialist. Uses `grep`, `glob`, and `read` to map dependencies, inspect architecture, and index files without risk of accidental mutations.
- **`Coder`**: Surgical implementation specialist. Applies targeted diffs and replacements using the `edit` and `write` tools adhering strictly to idiomatic patterns.
- **`Tester`**: Verification and diagnosis specialist. Runs targeted tests via the `bash` tool, captures outputs, isolates failures, and verifies regression tests.
- **`Reviewer`**: In-depth static audit specialist. Examines diffs for logic bugs, memory safety, and cross-platform quirks.
- **`General / Custom`**: Configurable worker agents tailored dynamically for user-defined pipelines.

Each subagent runs in an isolated task context with role-restricted toolsets, progress reporting channels, and lifecycle metrics. Agents coordinate through a peer **Mesh** supporting broadcast messages, direct messages with reply channels, and peer queries.

## Advisory Committee (Concurrent Automated Review)

Before executing high-impact code modifications or risky shell commands, Fusion consults a concurrent committee of specialized advisors:

| Advisor | Domain & Responsibilities | Risk Triggers |
| :--- | :--- | :--- |
| **Architecture Advisor** | Modularity, separation of concerns, DRY/SOLID principles, cross-platform safety (preventing OS-specific path leaks). | Monolithic bloat, tight coupling, broken platform abstractions. |
| **Security Advisor** | Command injection defense, credential & secret protection (`.env`, private keys), prevention of destructive shell scripts (`rm -rf`, raw disk ops). | Shell injection, token leaks, privilege escalation. |
| **Code Review Advisor** | Rust idioms, error propagation (`anyhow`/`thiserror`), zero-allocation designs, asynchronous cancellation safety, test coverage. | `unwrap()` in production code, excessive cloning, unhandled edge cases. |

Advisors assess proposed plans with structured risk levels: **`LOW`**, **`MEDIUM`**, **`HIGH`**, or **`CRITICAL`**. If critical risks are detected, execution halts with actionable critique and remediation suggestions. A weighted **Consensus Engine** aggregates advisor votes, supports vetoes, and resolves conflicting critiques before approval.
