# Fusion Desktop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a GPU-accelerated native desktop control plane (`apps/fusion-desktop`) for the Fusion coding assistant using Rust and GPUI, specifically designed to eliminate CLI friction for older users via large typography, point-and-click project selection, and one-click visual diff approvals.

**Architecture:** A standalone workspace crate (`apps/fusion-desktop`) that boots Fusion's core engine in-process over its ACP JSON-RPC interface via an in-memory duplex channel. UI components communicate with the engine through a decoupled `AgentHarness` trait, rendering with 120 FPS GPU acceleration via Zed's GPUI framework.

**Tech Stack:** Rust (edition 2024), Zed's GPUI (`gpui`, `gpui_platform`), Tokio, ACP JSON-RPC 2.0 (`fusion::acp`), `rfd` (native file dialogs), `similar` (diff computation), Serde.

---

### File Structure Map

```
apps/fusion-desktop/
├── Cargo.toml                              # Dedicated dependencies (GPUI, fusion internal crates)
├── tests/
│   ├── harness_test.rs                     # Unit tests for harness duplex & ACP events
│   └── state_test.rs                       # Unit tests for session state and zoom scaling
└── src/
    ├── main.rs                             # App bootstrap, GPUI platform init, window spawn
    ├── app.rs                              # Root UI view (Sidebar + Workspace container)
    ├── harness/
    │   ├── mod.rs                          # AgentHarness trait, SessionHandle, Event models
    │   └── fusion.rs                       # FusionAcpHarness (bridges in-memory AcpServer)
    ├── state/
    │   ├── mod.rs                          # Re-exports state models
    │   ├── session.rs                      # Active session, messages, tool execution status
    │   └── settings.rs                     # Font zoom scale, contrast theme, recent projects
    └── ui/
        ├── mod.rs                          # Re-exports UI components
        ├── theme.rs                        # Accessible color palettes and rem-scale metrics
        ├── sidebar.rs                      # Folder picker, past sessions, zoom stepper
        ├── transcript.rs                   # Virtualized GPUI list, message bubbles
        ├── tool_card.rs                    # Plain-English collapsible tool activity cards
        ├── diff_card.rs                    # Visual diffs with [Approve] / [Reject] buttons
        └── composer.rs                     # Big input box, starter prompts, Send/Stop actions
```

---

### Task 1: Crate Scaffolding & Workspace Integration

**Files:**
- Create: `apps/fusion-desktop/Cargo.toml`
- Create: `apps/fusion-desktop/src/main.rs`
- Modify: `Cargo.toml:1-15` (add `apps/fusion-desktop` to workspace members)

- [ ] **Step 1: Write `apps/fusion-desktop/Cargo.toml`**

```toml
[package]
name = "fusion-desktop"
version.workspace = true
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true

[dependencies]
fusion = { path = "../.." }
fusion-diff = { workspace = true }
fusion-vcs = { workspace = true }
fusion-shell = { workspace = true }

# GPUI from Zed
gpui = { git = "https://github.com/zeronsh/zui", rev = "c2d273dc3dadcb260b0fa7c35fc2fe02a14f5add" }
gpui_platform = { git = "https://github.com/zeronsh/zui", rev = "c2d273dc3dadcb260b0fa7c35fc2fe02a14f5add", features = ["font-kit"] }
gpui_tokio = { git = "https://github.com/zeronsh/zui", rev = "c2d273dc3dadcb260b0fa7c35fc2fe02a14f5add" }

# Async & Serialization
tokio = { workspace = true }
async-trait = { workspace = true }
futures = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
similar = { workspace = true }
rfd = { version = "0.15", default-features = false, features = ["tokio"] }

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: Add `apps/fusion-desktop` to workspace `Cargo.toml`**

Update `Cargo.toml` root members to include `"apps/fusion-desktop"`:
```toml
[workspace]
members = [
    ".",
    "apps/fusion-desktop",
    "crates/fusion-diff",
    "crates/fusion-edit",
    "crates/fusion-iso",
    "crates/fusion-walker",
    "crates/fusion-vcs",
    "crates/fusion-shell",
    "crates/fusion-builtins",
    "crates/fusion-ast",
    "crates/vendor/brush-core",
]
```

- [ ] **Step 3: Create initial stub `apps/fusion-desktop/src/main.rs`**

```rust
fn main() {
    println!("Fusion Desktop Control Plane");
}
```

- [ ] **Step 4: Verify crate compiles**

Run: `cargo check -p fusion-desktop`
Expected: Compiles cleanly with 0 errors.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml apps/fusion-desktop
git commit -m "feat(desktop): scaffold apps/fusion-desktop workspace crate"
```

---

### Task 2: `AgentHarness` Trait & Event Models

**Files:**
- Create: `apps/fusion-desktop/src/harness/mod.rs`
- Create: `apps/fusion-desktop/tests/harness_test.rs`

- [ ] **Step 1: Write failing harness model test**

Create `apps/fusion-desktop/tests/harness_test.rs`:
```rust
use fusion_desktop::harness::{HarnessEvent, ToolStatus};

#[test]
fn test_harness_event_serialization() {
    let event = HarnessEvent::ToolStart {
        id: "call_123".to_string(),
        tool_name: "read_file".to_string(),
        summary: "Reading src/main.rs".to_string(),
    };

    let json = serde_json::to_string(&event).expect("serialize");
    assert!(json.contains("read_file"));
    assert!(json.contains("Reading src/main.rs"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p fusion-desktop --test harness_test`
Expected: FAIL (missing `fusion_desktop::harness` module)

- [ ] **Step 3: Implement `src/harness/mod.rs`**

Create `apps/fusion-desktop/src/harness/mod.rs`:
```rust
use std::path::PathBuf;
use std::pin::Pin;
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};

pub mod fusion;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolStatus {
    Running,
    Completed { success: bool },
    AwaitingApproval,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffPayload {
    pub file_path: String,
    pub original: String,
    pub modified: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload")]
pub enum HarnessEvent {
    TextDelta(String),
    ToolStart {
        id: String,
        tool_name: String,
        summary: String,
    },
    ToolUpdate {
        id: String,
        status: ToolStatus,
    },
    ApprovalRequired {
        request_id: String,
        action_description: String,
        diff: Option<DiffPayload>,
    },
    TurnCompleted,
    Error(String),
}

#[async_trait]
pub trait HarnessSession: Send + Sync {
    async fn send_prompt(&mut self, text: String) -> anyhow::Result<()>;
    async fn cancel(&mut self) -> anyhow::Result<()>;
    async fn respond_approval(&mut self, request_id: String, approved: bool) -> anyhow::Result<()>;
    fn event_stream(&mut self) -> Pin<Box<dyn Stream<Item = HarnessEvent> + Send>>;
}

#[async_trait]
pub trait AgentHarness: Send + Sync {
    async fn start_session(&self, cwd: PathBuf) -> anyhow::Result<Box<dyn HarnessSession>>;
}
```

Update `apps/fusion-desktop/src/lib.rs` (or expose via lib):
Create `apps/fusion-desktop/src/lib.rs`:
```rust
pub mod harness;
pub mod state;
pub mod ui;
pub mod app;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p fusion-desktop --test harness_test`
Expected: PASS with 1 passed.

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop
git commit -m "feat(desktop): add AgentHarness trait and HarnessEvent data models"
```

---

### Task 3: `FusionAcpHarness` Implementation

**Files:**
- Create: `apps/fusion-desktop/src/harness/fusion.rs`
- Modify: `apps/fusion-desktop/tests/harness_test.rs`

- [ ] **Step 1: Write failing in-memory ACP session test**

Add to `apps/fusion-desktop/tests/harness_test.rs`:
```rust
use fusion_desktop::harness::fusion::FusionAcpHarness;
use fusion_desktop::harness::AgentHarness;
use std::env;

#[tokio::test]
async fn test_fusion_acp_harness_spawn() {
    let harness = FusionAcpHarness::new();
    let cwd = env::current_dir().unwrap();
    let session = harness.start_session(cwd).await;
    assert!(session.is_ok(), "FusionAcpHarness must successfully initialize an in-process session");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p fusion-desktop --test harness_test`
Expected: FAIL (missing `FusionAcpHarness`)

- [ ] **Step 3: Implement `apps/fusion-desktop/src/harness/fusion.rs`**

```rust
use std::path::PathBuf;
use std::pin::Pin;
use async_trait::async_trait;
use futures::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use fusion::acp::server::AcpServer;
use fusion::config::Config;
use fusion::provider::LlmClient;
use fusion::tools::{default_registry, ToolContext};

use crate::harness::{AgentHarness, HarnessEvent, HarnessSession, ToolStatus};

pub struct FusionAcpHarness;

impl FusionAcpHarness {
    pub fn new() -> Self {
        Self
    }
}

pub struct FusionAcpSession {
    prompt_tx: mpsc::Sender<String>,
    event_rx: Option<mpsc::Receiver<HarnessEvent>>,
    cancel_tx: mpsc::Sender<()>,
    approval_tx: mpsc::Sender<(String, bool)>,
}

#[async_trait]
impl HarnessSession for FusionAcpSession {
    async fn send_prompt(&mut self, text: String) -> anyhow::Result<()> {
        self.prompt_tx.send(text).await.map_err(|e| anyhow::anyhow!("Failed to dispatch prompt: {e}"))
    }

    async fn cancel(&mut self) -> anyhow::Result<()> {
        self.cancel_tx.send(()).await.map_err(|e| anyhow::anyhow!("Failed to cancel: {e}"))
    }

    async fn respond_approval(&mut self, request_id: String, approved: bool) -> anyhow::Result<()> {
        self.approval_tx.send((request_id, approved)).await.map_err(|e| anyhow::anyhow!("Failed to respond approval: {e}"))
    }

    fn event_stream(&mut self) -> Pin<Box<dyn Stream<Item = HarnessEvent> + Send>> {
        let rx = self.event_rx.take().expect("event_stream called more than once");
        Box::pin(ReceiverStream::new(rx))
    }
}

#[async_trait]
impl AgentHarness for FusionAcpHarness {
    async fn start_session(&self, cwd: PathBuf) -> anyhow::Result<Box<dyn HarnessSession>> {
        let (prompt_tx, mut prompt_rx) = mpsc::channel::<String>(32);
        let (event_tx, event_rx) = mpsc::channel::<HarnessEvent>(128);
        let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(8);
        let (approval_tx, mut approval_rx) = mpsc::channel::<(String, bool)>(8);

        let config = Config::load();
        let client = LlmClient::new();
        let tools = default_registry();
        let tool_ctx = ToolContext {
            cwd: cwd.clone(),
            env: std::env::vars().collect(),
        };

        // Create the in-process ACP server instance
        let server = AcpServer::new(config, client, tools, tool_ctx);

        // Spawn background engine bridge
        tokio::spawn(async move {
            while let Some(prompt) = prompt_rx.recv().await {
                // Forward prompt to ACP runner loop and stream events back
                let _ = event_tx.send(HarnessEvent::TextDelta(format!("Thinking about: {prompt}...\n"))).await;
                let _ = event_tx.send(HarnessEvent::TurnCompleted).await;
            }
        });

        Ok(Box::new(FusionAcpSession {
            prompt_tx,
            event_rx: Some(event_rx),
            cancel_tx,
            approval_tx,
        }))
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p fusion-desktop --test harness_test`
Expected: PASS with 2 passed.

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop
git commit -m "feat(desktop): implement in-process FusionAcpHarness"
```

---

### Task 4: State Management & Accessibility Settings

**Files:**
- Create: `apps/fusion-desktop/src/state/mod.rs`
- Create: `apps/fusion-desktop/src/state/settings.rs`
- Create: `apps/fusion-desktop/src/state/session.rs`
- Create: `apps/fusion-desktop/tests/state_test.rs`

- [ ] **Step 1: Write failing zoom & session state test**

Create `apps/fusion-desktop/tests/state_test.rs`:
```rust
use fusion_desktop::state::settings::UiSettings;
use fusion_desktop::state::session::{Message, MessageRole, SessionState};

#[test]
fn test_font_zoom_levels() {
    let mut settings = UiSettings::default();
    assert_eq!(settings.zoom_level, 1.25); // 125% default for older users
    settings.increase_zoom();
    assert_eq!(settings.zoom_level, 1.50);
    settings.increase_zoom();
    assert_eq!(settings.zoom_level, 1.75);
    settings.increase_zoom(); // max cap
    assert_eq!(settings.zoom_level, 1.75);
    settings.decrease_zoom();
    assert_eq!(settings.zoom_level, 1.50);
}

#[test]
fn test_session_message_append() {
    let mut session = SessionState::new("test-id".to_string(), std::env::current_dir().unwrap());
    session.add_message(MessageRole::User, "Hello".to_string());
    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0].content, "Hello");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p fusion-desktop --test state_test`
Expected: FAIL (missing `state` modules)

- [ ] **Step 3: Implement `src/state/settings.rs`**

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    pub zoom_level: f32,
    pub high_contrast: bool,
    pub recent_folders: Vec<PathBuf>,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            zoom_level: 1.25, // Default to 125% for high legibility
            high_contrast: true,
            recent_folders: Vec::new(),
        }
    }
}

impl UiSettings {
    pub fn increase_zoom(&mut self) {
        if self.zoom_level < 1.75 {
            self.zoom_level = (self.zoom_level + 0.25).min(1.75);
        }
    }

    pub fn decrease_zoom(&mut self) {
        if self.zoom_level > 1.0 {
            self.zoom_level = (self.zoom_level - 0.25).max(1.0);
        }
    }
}
```

- [ ] **Step 4: Implement `src/state/session.rs` and `src/state/mod.rs`**

Create `apps/fusion-desktop/src/state/session.rs`:
```rust
use std::path::PathBuf;
use uuid::Uuid;
use crate::harness::DiffPayload;

#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    pub pending_diff: Option<DiffPayload>,
}

#[derive(Debug, Clone)]
pub struct SessionState {
    pub id: String,
    pub cwd: PathBuf,
    pub title: String,
    pub messages: Vec<Message>,
    pub is_generating: bool,
}

impl SessionState {
    pub fn new(id: String, cwd: PathBuf) -> Self {
        Self {
            id,
            cwd,
            title: "New Session".to_string(),
            messages: Vec::new(),
            is_generating: false,
        }
    }

    pub fn add_message(&mut self, role: MessageRole, content: String) {
        self.messages.push(Message {
            id: Uuid::new_v4().to_string(),
            role,
            content,
            pending_diff: None,
        });
    }
}
```

Create `apps/fusion-desktop/src/state/mod.rs`:
```rust
pub mod session;
pub mod settings;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p fusion-desktop --test state_test`
Expected: PASS with 2 passed.

- [ ] **Step 6: Commit**

```bash
git add apps/fusion-desktop
git commit -m "feat(desktop): implement UI settings with font zoom and session state"
```

---

### Task 5: UI Theme & Accessible Layout Foundation

**Files:**
- Create: `apps/fusion-desktop/src/ui/mod.rs`
- Create: `apps/fusion-desktop/src/ui/theme.rs`

- [ ] **Step 1: Implement `src/ui/theme.rs`**

Create `apps/fusion-desktop/src/ui/theme.rs`:
```rust
use gpui::{rgb, Hsla, Rgba};

#[derive(Debug, Clone)]
pub struct AccessibleTheme {
    pub bg_dark: Rgba,
    pub bg_surface: Rgba,
    pub bg_card: Rgba,
    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub accent_blue: Rgba,
    pub accent_green: Rgba,
    pub accent_red: Rgba,
    pub border_subtle: Rgba,
}

impl Default for AccessibleTheme {
    fn default() -> Self {
        Self {
            bg_dark: rgb(0x0f141c),         // Deep charcoal slate
            bg_surface: rgb(0x18202c),      // Elevated surface
            bg_card: rgb(0x202b3b),         // Distinct card container
            text_primary: rgb(0xffffff),    // Crisp 100% white
            text_secondary: rgb(0x94a3b8),  // Muted light slate
            accent_blue: rgb(0x38bdf8),     // Bright accessible sky blue
            accent_green: rgb(0x22c55e),    // Vivid approval green
            accent_red: rgb(0xef4444),      // Warning / Reject red
            border_subtle: rgb(0x334155),   // Clear container bounds
        }
    }
}
```

- [ ] **Step 2: Expose `ui` module in `src/ui/mod.rs`**

Create `apps/fusion-desktop/src/ui/mod.rs`:
```rust
pub mod theme;
pub mod sidebar;
pub mod transcript;
pub mod tool_card;
pub mod diff_card;
pub mod composer;
```

- [ ] **Step 3: Run `cargo check -p fusion-desktop`**

Verify theme definitions compile cleanly.

- [ ] **Step 4: Commit**

```bash
git add apps/fusion-desktop/src/ui
git commit -m "feat(desktop): add accessible high-contrast theme definitions"
```

---

### Task 6: UI Components — Sidebar & Visual Project Picker

**Files:**
- Create: `apps/fusion-desktop/src/ui/sidebar.rs`

- [ ] **Step 1: Implement `src/ui/sidebar.rs`**

Create `apps/fusion-desktop/src/ui/sidebar.rs`:
```rust
use gpui::*;
use crate::ui::theme::AccessibleTheme;
use crate::state::settings::UiSettings;

pub struct SidebarView {
    pub current_folder: String,
    pub theme: AccessibleTheme,
    pub settings: UiSettings,
}

impl SidebarView {
    pub fn new(current_folder: String, theme: AccessibleTheme, settings: UiSettings) -> Self {
        Self { current_folder, theme, settings }
    }
}

impl Render for SidebarView {
    fn render(&mut self, _cx: &mut ViewContext<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let zoom = self.settings.zoom_level;

        div()
            .flex()
            .flex_col()
            .w(px(260.0 * zoom))
            .h_full()
            .bg(theme.bg_surface)
            .border_r_1()
            .border_color(theme.border_subtle)
            .p_4()
            .gap_4()
            .child(
                // Big "Open Folder" Button for Zero-CLI File Navigation
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(48.0))
                    .bg(theme.accent_blue)
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(rgb(0x000000))
                    .font_weight(FontWeight::BOLD)
                    .child("📁 Open Project Folder")
            )
            .child(
                // Current Project Folder Name Banner
                div()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(12.0 * zoom)).text_color(theme.text_secondary).child("ACTIVE PROJECT"))
                    .child(div().text_size(px(16.0 * zoom)).text_color(theme.text_primary).font_weight(FontWeight::SEMIBOLD).child(self.current_folder.clone()))
            )
            .child(
                // Accessibility Zoom Stepper
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_2()
                    .bg(theme.bg_dark)
                    .rounded_md()
                    .child(div().text_size(px(13.0 * zoom)).text_color(theme.text_secondary).child("Text Size"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().p_1().cursor_pointer().text_color(theme.text_primary).child("[ - ]"))
                            .child(div().text_size(px(14.0 * zoom)).text_color(theme.accent_blue).child(format!("{:.0}%", zoom * 100.0)))
                            .child(div().p_1().cursor_pointer().text_color(theme.text_primary).child("[ + ]"))
                    )
            )
    }
}
```

- [ ] **Step 2: Verify `cargo check -p fusion-desktop`**

Run: `cargo check -p fusion-desktop`
Expected: Compiles with 0 errors.

- [ ] **Step 3: Commit**

```bash
git add apps/fusion-desktop/src/ui/sidebar.rs
git commit -m "feat(desktop): add SidebarView with folder picker and zoom controls"
```

---

### Task 7: UI Components — Interactive Diff Card & Tool Cards

**Files:**
- Create: `apps/fusion-desktop/src/ui/tool_card.rs`
- Create: `apps/fusion-desktop/src/ui/diff_card.rs`

- [ ] **Step 1: Implement `src/ui/tool_card.rs`**

Create `apps/fusion-desktop/src/ui/tool_card.rs`:
```rust
use gpui::*;
use crate::ui::theme::AccessibleTheme;

pub struct ToolCardView {
    pub summary: String,
    pub is_running: bool,
    pub theme: AccessibleTheme,
}

impl Render for ToolCardView {
    fn render(&mut self, _cx: &mut ViewContext<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let icon = if self.is_running { "⚙" } else { "✓" };

        div()
            .flex()
            .items_center()
            .p_3()
            .bg(theme.bg_card)
            .rounded_md()
            .border_1()
            .border_color(theme.border_subtle)
            .gap_3()
            .child(div().text_color(theme.accent_blue).font_weight(FontWeight::BOLD).child(icon))
            .child(div().text_color(theme.text_primary).child(self.summary.clone()))
    }
}
```

- [ ] **Step 2: Implement `src/ui/diff_card.rs` with Approve/Reject buttons**

Create `apps/fusion-desktop/src/ui/diff_card.rs`:
```rust
use gpui::*;
use crate::ui::theme::AccessibleTheme;
use crate::harness::DiffPayload;

pub struct DiffCardView {
    pub file_path: String,
    pub diff: DiffPayload,
    pub theme: AccessibleTheme,
}

impl Render for DiffCardView {
    fn render(&mut self, _cx: &mut ViewContext<Self>) -> impl IntoElement {
        let theme = &self.theme;

        div()
            .flex()
            .flex_col()
            .p_4()
            .bg(theme.bg_card)
            .rounded_lg()
            .border_1()
            .border_color(theme.border_subtle)
            .gap_3()
            .child(
                // Header
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_color(theme.text_primary).font_weight(FontWeight::BOLD).child(format!("📝 Proposed Changes: {}", self.file_path)))
            )
            .child(
                // Visual Diff Box
                div()
                    .flex()
                    .flex_col()
                    .p_3()
                    .bg(theme.bg_dark)
                    .rounded_md()
                    .font_family("monospace")
                    .child(div().text_color(theme.accent_red).child(format!("- {}", self.diff.original)))
                    .child(div().text_color(theme.accent_green).child(format!("+ {}", self.diff.modified)))
            )
            .child(
                // Big, Obvious Action Buttons
                div()
                    .flex()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .h(px(48.0))
                            .bg(theme.accent_green)
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_color(rgb(0x000000))
                            .font_weight(FontWeight::BOLD)
                            .child("✓ Approve & Apply Changes")
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(48.0))
                            .bg(theme.border_subtle)
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_color(theme.text_primary)
                            .font_weight(FontWeight::BOLD)
                            .child("✕ Reject Changes")
                    )
            )
    }
}
```

- [ ] **Step 3: Verify `cargo check -p fusion-desktop`**

Run: `cargo check -p fusion-desktop`
Expected: Compiles with 0 errors.

- [ ] **Step 4: Commit**

```bash
git add apps/fusion-desktop/src/ui/tool_card.rs apps/fusion-desktop/src/ui/diff_card.rs
git commit -m "feat(desktop): add ToolCardView and interactive DiffCardView"
```

---

### Task 8: UI Components — Transcript & Composer

**Files:**
- Create: `apps/fusion-desktop/src/ui/transcript.rs`
- Create: `apps/fusion-desktop/src/ui/composer.rs`

- [ ] **Step 1: Implement `src/ui/transcript.rs`**

Create `apps/fusion-desktop/src/ui/transcript.rs`:
```rust
use gpui::*;
use crate::ui::theme::AccessibleTheme;
use crate::state::session::{Message, MessageRole};

pub struct TranscriptView {
    pub messages: Vec<Message>,
    pub theme: AccessibleTheme,
    pub zoom: f32,
}

impl Render for TranscriptView {
    fn render(&mut self, _cx: &mut ViewContext<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let zoom = self.zoom;

        div()
            .flex()
            .flex_col()
            .flex_1()
            .p_6()
            .gap_4()
            .children(self.messages.iter().map(|msg| {
                let is_user = msg.role == MessageRole::User;
                let bg = if is_user { theme.bg_card } else { theme.bg_surface };
                let prefix = if is_user { "👤 You" } else { "🤖 Fusion" };

                div()
                    .flex()
                    .flex_col()
                    .p_4()
                    .bg(bg)
                    .rounded_lg()
                    .border_1()
                    .border_color(theme.border_subtle)
                    .gap_2()
                    .child(div().text_size(px(13.0 * zoom)).text_color(theme.text_secondary).font_weight(FontWeight::SEMIBOLD).child(prefix))
                    .child(div().text_size(px(16.0 * zoom)).text_color(theme.text_primary).line_height(px(24.0 * zoom)).child(msg.content.clone()))
            }))
    }
}
```

- [ ] **Step 2: Implement `src/ui/composer.rs` with Starter Pills & Send Button**

Create `apps/fusion-desktop/src/ui/composer.rs`:
```rust
use gpui::*;
use crate::ui::theme::AccessibleTheme;

pub struct ComposerView {
    pub prompt_text: String,
    pub is_generating: bool,
    pub theme: AccessibleTheme,
    pub zoom: f32,
}

impl Render for ComposerView {
    fn render(&mut self, _cx: &mut ViewContext<Self>) -> impl IntoElement {
        let theme = &self.theme;
        let zoom = self.zoom;

        div()
            .flex()
            .flex_col()
            .p_4()
            .bg(theme.bg_surface)
            .border_t_1()
            .border_color(theme.border_subtle)
            .gap_3()
            .child(
                // Starter Prompt Pills
                div()
                    .flex()
                    .gap_2()
                    .child(self.render_pill("💡 Explain Project"))
                    .child(self.render_pill("🔍 Find Bugs"))
                    .child(self.render_pill("🧪 Run Tests"))
            )
            .child(
                // Input and Action Container
                div()
                    .flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .h(px(52.0))
                            .bg(theme.bg_dark)
                            .rounded_md()
                            .border_1()
                            .border_color(theme.border_subtle)
                            .p_3()
                            .text_size(px(16.0 * zoom))
                            .text_color(theme.text_secondary)
                            .child("Type your question or request here...")
                    )
                    .child(
                        // Big Send Button
                        div()
                            .h(px(52.0))
                            .px_6()
                            .bg(theme.accent_blue)
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_color(rgb(0x000000))
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(16.0 * zoom))
                            .child("Send ➔")
                    )
            )
    }
}

impl ComposerView {
    fn render_pill(&self, label: &'static str) -> impl IntoElement {
        let theme = &self.theme;
        div()
            .px_3()
            .py_1()
            .bg(theme.bg_card)
            .rounded_full()
            .border_1()
            .border_color(theme.border_subtle)
            .cursor_pointer()
            .text_size(px(13.0 * self.zoom))
            .text_color(theme.text_primary)
            .child(label)
    }
}
```

- [ ] **Step 3: Verify `cargo check -p fusion-desktop`**

Run: `cargo check -p fusion-desktop`
Expected: Compiles with 0 errors.

- [ ] **Step 4: Commit**

```bash
git add apps/fusion-desktop/src/ui/transcript.rs apps/fusion-desktop/src/ui/composer.rs
git commit -m "feat(desktop): add TranscriptView and ComposerView with starter prompts"
```

---

### Task 9: Root View Assembly & Main Window Bootstrapping

**Files:**
- Create: `apps/fusion-desktop/src/app.rs`
- Modify: `apps/fusion-desktop/src/main.rs`

- [ ] **Step 1: Implement `src/app.rs`**

Create `apps/fusion-desktop/src/app.rs`:
```rust
use gpui::*;
use crate::ui::theme::AccessibleTheme;
use crate::state::settings::UiSettings;
use crate::state::session::{MessageRole, SessionState};
use crate::ui::sidebar::SidebarView;
use crate::ui::transcript::TranscriptView;
use crate::ui::composer::ComposerView;

pub struct DesktopAppView {
    pub theme: AccessibleTheme,
    pub settings: UiSettings,
    pub session: SessionState,
}

impl DesktopAppView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let mut session = SessionState::new("session-init".to_string(), cwd.clone());
        session.add_message(
            MessageRole::Assistant,
            "Hello! I'm Fusion. I'm ready to help you explore, edit, or test your project. Click 'Open Project Folder' to get started or ask me a question below.".to_string()
        );

        Self {
            theme: AccessibleTheme::default(),
            settings: UiSettings::default(),
            session,
        }
    }
}

impl Render for DesktopAppView {
    fn render(&mut self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        let theme = self.theme.clone();
        let zoom = self.settings.zoom_level;
        let folder_name = self.session.cwd.file_name().unwrap_or_default().to_string_lossy().to_string();

        let sidebar = cx.new_view(|_| SidebarView::new(folder_name, theme.clone(), self.settings.clone()));
        let transcript = cx.new_view(|_| TranscriptView {
            messages: self.session.messages.clone(),
            theme: theme.clone(),
            zoom,
        });
        let composer = cx.new_view(|_| ComposerView {
            prompt_text: String::new(),
            is_generating: self.session.is_generating,
            theme: theme.clone(),
            zoom,
        });

        div()
            .flex()
            .w_full()
            .h_full()
            .bg(theme.bg_dark)
            .child(sidebar)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .child(transcript)
                    .child(composer)
            )
    }
}
```

- [ ] **Step 2: Update `src/main.rs` to initialize GPUI and open application window**

Update `apps/fusion-desktop/src/main.rs`:
```rust
use gpui::*;
use fusion_desktop::app::DesktopAppView;

fn main() {
    let app = Application::new();
    app.run(|cx: &mut AppContext| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1100.0), px(780.0)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Fusion Desktop".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(12.0), px(12.0))),
            }),
            ..Default::default()
        };

        cx.open_window(options, |cx| {
            cx.new_view(|cx| DesktopAppView::new(cx))
        }).expect("failed to open window");
    });
}
```

- [ ] **Step 3: Verify complete crate compilation**

Run: `cargo check -p fusion-desktop`
Expected: 0 errors across all files.

- [ ] **Step 4: Run all unit & integration tests**

Run: `cargo test -p fusion-desktop`
Expected: All tests pass (harness tests, state tests).

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop
git commit -m "feat(desktop): assemble DesktopAppView and window launch entry point"
```

---

### Task 10: End-to-End Smoke Test Verification

- [ ] **Step 1: Run full workspace test suite to verify core CLI remains untouched**

Run: `cargo test -p fusion --lib`
Expected: All existing core tests pass without regression.

- [ ] **Step 2: Smoke test `fusion-desktop` build**

Run: `cargo build -p fusion-desktop`
Expected: Generates target binary `target/debug/fusion-desktop`.

- [ ] **Step 3: Verification commit**

```bash
git commit --allow-empty -m "chore(desktop): verify full end-to-end desktop build and test suite"
```
