# Clipboard Image Paste and Multimodal Vision Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable pasting images from the system clipboard into the Fusion CLI prompt with inline `[Image #N: WxH]` placeholders and dispatching them as multimodal vision attachments to LLM providers (Anthropic, OpenAI, OpenRouter, Ollama).

**Architecture:** Integrate `arboard` and `image` to detect and encode OS clipboard image pixels into local PNG cache files. Hook `Ctrl+V`, terminal paste events, and `/image` in `Prompt` to insert non-destructive placeholders. Extend `Message` with `ImageAttachment` and serialize native vision content blocks across all LLM providers.

**Tech Stack:** Rust (edition 2021), `arboard`, `image`, `crossterm`, `tokio`, `serde_json`, `base64`.

---

### Task 1: Add Dependencies & Clipboard Image Ingestion Helper

**Files:**
- Modify: `Cargo.toml:420-435`
- Create: `src/ui/clipboard_image.rs`
- Modify: `src/ui/mod.rs`
- Test: `src/ui/clipboard_image.rs` (inline module tests)

- [ ] **Step 1: Update Cargo.toml dependencies**

In `Cargo.toml` under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`, add `arboard` and `image`:
```toml
arboard = { workspace = true }
image = { workspace = true }
```

- [ ] **Step 2: Write unit tests in `src/ui/clipboard_image.rs`**

Write tests for image encoding, dimension extraction, and cache directory creation:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_rgba_to_png() {
        // 2x2 RGBA test image: red, green, blue, white
        let rgba_data = vec![
            255, 0, 0, 255,   0, 255, 0, 255,
            0, 0, 255, 255,   255, 255, 255, 255,
        ];
        let png_bytes = encode_rgba_png(2, 2, &rgba_data).expect("must encode png");
        assert!(!png_bytes.is_empty());
        assert_eq!(&png_bytes[0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn test_format_placeholder() {
        let placeholder = format_image_placeholder(1, 1920, 1080);
        assert_eq!(placeholder, "[Image #1: 1920x1080]");
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --bin fusion ui::clipboard_image::tests`
Expected: FAIL (module not defined)

- [ ] **Step 4: Implement `src/ui/clipboard_image.rs`**

Implement RGBA to PNG encoding, clipboard querying via `arboard`, and cache management:
```rust
//! System clipboard image extraction and caching for Fusion CLI.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PastedImage {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub bytes_len: usize,
    pub media_type: String,
}

/// Encodes raw RGBA8 pixels into compressed PNG byte buffer.
pub fn encode_rgba_png(width: u32, height: u32, rgba_data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let img_buf = image::RgbaImage::from_raw(width, height, rgba_data.to_vec())
        .ok_or_else(|| anyhow::anyhow!("Invalid RGBA image buffer dimensions"))?;
    let mut png_bytes = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
    image::ImageEncoder::write_image(
        encoder,
        img_buf.as_raw(),
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(png_bytes)
}

/// Formats the inline text placeholder for an attached image.
pub fn format_image_placeholder(index: usize, width: u32, height: u32) -> String {
    format!("[Image #{}: {}x{}]", index, width, height)
}

/// Returns the cache directory for pasted images (`.fusion/cache/images/`).
pub fn get_image_cache_dir() -> PathBuf {
    let dir = PathBuf::from(".fusion/cache/images");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Queries the OS clipboard for image data. If an image is found,
/// it is encoded as a PNG and saved to the local image cache.
pub fn read_clipboard_image() -> Option<PastedImage> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut clipboard = arboard::Clipboard::new().ok()?;
        let img = clipboard.get_image().ok()?;
        let width = img.width as u32;
        let height = img.height as u32;
        let png_bytes = encode_rgba_png(width, height, &img.bytes).ok()?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let cache_dir = get_image_cache_dir();
        let filename = format!("clip_{}_{}x{}.png", now, width, height);
        let path = cache_dir.join(filename);

        fs::write(&path, &png_bytes).ok()?;

        Some(PastedImage {
            path,
            width,
            height,
            bytes_len: png_bytes.len(),
            media_type: "image/png".to_string(),
        })
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}
```
Expose `pub mod clipboard_image;` in `src/ui/mod.rs`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --bin fusion ui::clipboard_image::tests`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/ui/clipboard_image.rs src/ui/mod.rs
git commit -m "feat(ui): add clipboard image detection and PNG caching helper"
```

---

### Task 2: Multimodal Image Support in Provider Types

**Files:**
- Modify: `src/provider/types.rs`
- Test: `src/provider/types.rs`

- [ ] **Step 1: Write failing tests for `ImageAttachment` and `Message` serialization**

Add test in `src/provider/types.rs`:
```rust
#[test]
fn test_message_with_image_attachments_serialization() {
    let attachment = ImageAttachment {
        media_type: "image/png".to_string(),
        data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".to_string(),
        path: Some(".fusion/cache/images/clip_1.png".to_string()),
        width: Some(1),
        height: Some(1),
    };
    let msg = Message::user_with_images("Explain this image", vec![attachment.clone()]);
    let json = serde_json::to_string(&msg).expect("must serialize");
    assert!(json.contains("Explain this image"));
    assert!(json.contains("image/png"));

    let deserialized: Message = serde_json::from_str(&json).expect("must deserialize");
    assert_eq!(deserialized.content, "Explain this image");
    assert!(deserialized.images.is_some());
    assert_eq!(deserialized.images.unwrap()[0].width, Some(1));
}

#[test]
fn test_message_without_images_omits_field() {
    let msg = Message::user("plain text prompt");
    let json = serde_json::to_string(&msg).expect("must serialize");
    assert!(!json.contains("\"images\""));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin fusion test_message_with_image_attachments_serialization`
Expected: FAIL (unresolved types/constructors)

- [ ] **Step 3: Implement `ImageAttachment` and update `Message`**

In `src/provider/types.rs`:
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageAttachment {
    pub media_type: String,
    pub data: String, // base64 encoded
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageAttachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn user_with_images(content: impl Into<String>, images: Vec<ImageAttachment>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            images: if images.is_empty() { None } else { Some(images) },
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }
}
```
Update `impl PartialEq for Message` to compare `self.images == other.images`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --bin fusion test_message_with_image_attachments_serialization`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/provider/types.rs
git commit -m "feat(provider): add ImageAttachment and multimodal message fields"
```

---

### Task 3: Vision Payload Adapters for LLM Providers

**Files:**
- Modify: `src/provider/anthropic.rs`
- Modify: `src/provider/openai.rs`
- Modify: `src/provider/openrouter.rs`
- Modify: `src/provider/ollama.rs`
- Test: `src/provider/anthropic.rs`, `src/provider/openai.rs`

- [ ] **Step 1: Write failing tests for provider vision payloads**

In `src/provider/anthropic.rs`:
```rust
#[test]
fn test_anthropic_payload_with_image_blocks() {
    let img = ImageAttachment {
        media_type: "image/png".to_string(),
        data: "aGVsbG8=".to_string(),
        path: None,
        width: None,
        height: None,
    };
    let msg = Message::user_with_images("What is this?", vec![img]);
    let anthropic_msgs = convert_messages_for_anthropic(&[msg]);
    assert_eq!(anthropic_msgs.len(), 1);
    match &anthropic_msgs[0].content {
        AnthropicContent::Blocks(blocks) => {
            assert_eq!(blocks.len(), 2);
            assert!(matches!(&blocks[0], AnthropicContentBlock::Text { text, .. } if text == "What is this?"));
            assert!(matches!(&blocks[1], AnthropicContentBlock::Image { .. }));
        }
        _ => panic!("Expected Content::Blocks"),
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin fusion test_anthropic_payload_with_image_blocks`
Expected: FAIL

- [ ] **Step 3: Implement provider image payloads**

1. In `src/provider/anthropic.rs`:
Update message conversion where `Role::User` is processed:
```rust
Role::User => {
    if let Some(images) = &msg.images {
        let mut blocks = Vec::new();
        if !msg.content.is_empty() {
            blocks.push(AnthropicContentBlock::Text {
                text: msg.content.clone(),
                cache_control: None,
            });
        }
        for img in images {
            blocks.push(AnthropicContentBlock::Image {
                source: AnthropicImageSource {
                    source_type: "base64".to_string(),
                    media_type: img.media_type.clone(),
                    data: img.data.clone(),
                },
            });
        }
        anthropic_messages.push(AnthropicMessage {
            role: AnthropicRole::User,
            content: AnthropicContent::Blocks(blocks),
        });
    } else {
        anthropic_messages.push(AnthropicMessage {
            role: AnthropicRole::User,
            content: AnthropicContent::Text(msg.content.clone()),
        });
    }
}
```

2. In `src/provider/openai.rs` and `src/provider/openrouter.rs`:
When formatting user messages for `/chat/completions`:
```rust
if let Some(images) = &msg.images {
    let mut parts = Vec::new();
    if !msg.content.is_empty() {
        parts.push(serde_json::json!({
            "type": "text",
            "text": msg.content
        }));
    }
    for img in images {
        parts.push(serde_json::json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{};base64,{}", img.media_type, img.data)
            }
        }));
    }
    // send content as parts array
}
```

3. In `src/provider/ollama.rs`:
Map `msg.images` to `images: Some(images.iter().map(|i| i.data.clone()).collect())`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --bin fusion test_anthropic_payload_with_image_blocks`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/provider/anthropic.rs src/provider/openai.rs src/provider/openrouter.rs src/provider/ollama.rs
git commit -m "feat(provider): format multimodal image attachments across all LLM providers"
```

---

### Task 4: Interactive Prompt Integration (`Ctrl+V`, `Event::Paste`, `/image`)

**Files:**
- Modify: `src/ui/prompt.rs`
- Modify: `src/ui/keys.rs`
- Test: `src/ui/prompt.rs`

- [ ] **Step 1: Write failing tests for image placeholder insertion in prompt**

In `src/ui/prompt.rs`:
```rust
#[test]
fn test_prompt_insert_image_attachment() {
    let mut prompt = Prompt::new();
    prompt.buffer = "Look at this ".chars().collect();
    prompt.cursor_pos = prompt.buffer.len();

    let path = std::path::PathBuf::from(".fusion/cache/images/test.png");
    prompt.attach_image(path, 800, 600);

    let text: String = prompt.buffer.iter().collect();
    assert_eq!(text, "Look at this [Image #1: 800x600]");
    assert_eq!(prompt.pending_images.len(), 1);
    assert_eq!(prompt.pending_images[0].width, 800);
}

#[test]
fn test_prompt_reconcile_deleted_placeholder() {
    let mut prompt = Prompt::new();
    let path = std::path::PathBuf::from(".fusion/cache/images/test.png");
    prompt.attach_image(path, 800, 600);

    // User deletes placeholder via Backspace
    prompt.buffer.clear();
    let remaining = prompt.reconcile_attached_images();
    assert!(remaining.is_empty(), "deleted placeholder must detach image");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --bin fusion test_prompt_insert_image_attachment`
Expected: FAIL

- [ ] **Step 3: Implement prompt image attachment tracking and key hooks**

1. In `src/ui/prompt.rs`:
Add fields to `Prompt`:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingImageAttachment {
    pub index: usize,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub tag: String,
}

pub struct Prompt {
    ...
    pub pending_images: Vec<PendingImageAttachment>,
}
```
Implement `attach_image` and `reconcile_attached_images`:
```rust
impl Prompt {
    pub fn attach_image(&mut self, path: PathBuf, width: u32, height: u32) {
        let index = self.pending_images.len() + 1;
        let tag = crate::ui::clipboard_image::format_image_placeholder(index, width, height);
        for c in tag.chars() {
            self.buffer.insert(self.cursor_pos, c);
            self.cursor_pos += 1;
        }
        self.pending_images.push(PendingImageAttachment {
            index,
            path,
            width,
            height,
            tag,
        });
    }

    pub fn reconcile_attached_images(&self) -> Vec<PendingImageAttachment> {
        let current_text: String = self.buffer.iter().collect();
        self.pending_images
            .iter()
            .filter(|img| current_text.contains(&img.tag))
            .cloned()
            .collect()
    }
}
```

2. Hook `Ctrl+V` in `src/ui/keys.rs` and `src/ui/prompt.rs`:
In `KeyHandler::handle_emacs` and default profile, map `(KeyCode::Char('v'), KeyModifiers::CONTROL)`:
```rust
(KeyCode::Char('v'), KeyModifiers::CONTROL) => KeyAction::Paste,
```
In `Prompt::handle_event`:
When handling paste (either via `Event::Paste` or `KeyAction::Paste`):
```rust
// First check if clipboard has an image
if let Some(pasted) = crate::ui::clipboard_image::read_clipboard_image() {
    self.attach_image(pasted.path, pasted.width, pasted.height);
    self.render_current()?;
    return Ok(None);
}
// Else fall back to standard text paste
```

3. Register `/image` in `COMMAND_PALETTE` in `src/ui/prompt.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --bin fusion test_prompt_insert_image_attachment`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/ui/prompt.rs src/ui/keys.rs
git commit -m "feat(ui): add image attachment insertion and placeholder reconciliation in prompt"
```

---

### Task 5: Agent Turn Wiring & End-to-End Verification

**Files:**
- Modify: `src/agent/loop_runner.rs`
- Modify: `src/ui/repl.rs`
- Test: `tests/clipboard_image_test.rs`

- [ ] **Step 1: Write integration test for image turn execution**

Create `tests/clipboard_image_test.rs`:
```rust
use fusion::provider::types::{ImageAttachment, Message, Role};

#[test]
fn test_loop_runner_message_with_image() {
    let img = ImageAttachment {
        media_type: "image/png".to_string(),
        data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=".to_string(),
        path: Some("clip_test.png".to_string()),
        width: Some(1),
        height: Some(1),
    };
    let msg = Message::user_with_images("Describe this screenshot", vec![img]);
    assert_eq!(msg.role, Role::User);
    assert!(msg.images.is_some());
    assert_eq!(msg.images.as_ref().unwrap().len(), 1);
}
```

- [ ] **Step 2: Run test to verify it passes**

Run: `cargo test --test clipboard_image_test`
Expected: PASS

- [ ] **Step 3: Connect prompt attachments into `run_turn_stream`**

In `src/ui/repl.rs`:
When `PromptResult::Submit(input)` is received:
- Collect active attachments via `prompt.reconcile_attached_images()`.
- Load and base64-encode image files into `Vec<ImageAttachment>`.
- Pass attachments into `runner.run_turn_stream_with_images(&mut session, &input, images, tx)`.
- If images are attached, print status confirmation: `📷 Attached: [Image #1: ...]`.
- Reset prompt attachments after turn starts.

- [ ] **Step 4: Full compilation check and test suite**

Run:
```bash
cargo check --bin fusion
cargo test --bin fusion
```
Expected: All tests pass, zero errors.

- [ ] **Step 5: Commit**

```bash
git add src/agent/loop_runner.rs src/ui/repl.rs tests/clipboard_image_test.rs
git commit -m "feat(agent): wire prompt image attachments into runner and LLM turn"
```
