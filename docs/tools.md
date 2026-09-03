# Built-in Tools

Fusion includes a sandboxed, cross-platform tool registry:

| Tool | Description |
| :--- | :--- |
| **`read` / `read_file`** | Surgical file reading with offset and line-limit selectors. |
| **`write` / `write_file`** | Safe file writing and creation. |
| **`edit` / `edit_file`** | Accurate text replacement and block patching with unambiguous anchor detection. |
| **`grep`** | High-speed regex and literal searching with `.gitignore` awareness and result filtering. |
| **`glob`** | Fast pattern-based directory and file scanning. |
| **`bash`** | Asynchronous command execution with timeouts, output truncation protection, and signal cancellation. |

Extended registry:

| Tool | Description |
| :--- | :--- |
| **`git` / `git_log` / `git_branch`** | Repository inspection, history, and branch queries. |
| **`fetch` / `web_search`** | HTTP fetching and web search. |
| **`sqlite`** | Embedded SQLite queries with typed cell values. |
| **`test_runner` / `regex_test`** | Targeted test execution and regex validation. |
| **`symbols` / `syntax` / `dep_graph` / `deps`** | Symbol indexing, syntax inspection, and dependency analysis. |
| **`diff_stats` / `patch` / `format` / `docgen` / `crate_docs`** | Diff analytics, patch application, code formatting, and documentation generation. |
| **`watch` / `process` / `ports` / `system` / `env_cleaner` / `profiler`** | File watching, process/port inspection, system diagnostics, environment hygiene, and profiling. |
| **`clipboard` / `hex` / `secret_scan` / `guardrails` / `mock_server`** | Clipboard access, hex dumps, secret detection, safety guardrails, and HTTP mocking. |
| **`mcp` / `mcp_bridge`** | Model Context Protocol JSON-RPC client and bridge for external tool servers. |
| **`compat`** | Legacy tool-name compatibility mapping. |
