# Clipboard Image Paste and Multimodal Vision Support

**Date:** 2026-09-10  
**Status:** Approved  
**Topic:** Support pasting images from system clipboard into Fusion CLI prompt with inline placeholders and multimodal LLM vision dispatch.

---

## 1. Overview & Goals

Fusion CLI currently processes prompt input as pure text. When users take a screenshot (e.g. `Cmd+Shift+4` on macOS, `Win+Shift+S` on Windows, or screenshot tools on Linux) and attempt to paste it with `Cmd+V` or `Ctrl+V`, no image is captured, no visual feedback is displayed, and vision-capable LLMs cannot inspect the image.

This design enables:
1. **Clipboard Image Ingestion**: Intercepting clipboard image data on `Ctrl+V`, terminal paste events, or `/image` / `/paste` slash commands using `arboard`.
2. **Inline Placeholder Rendering**: Inserting a clean, editable placeholder (e.g., `[Image #1: 1280x720]`) into the interactive prompt buffer, which can be edited or deleted with standard Backspace.
3. **Local Image Caching**: Saving raw clipboard RGBA pixels as compressed PNG files into `.fusion/cache/images/`.
4. **Multimodal LLM Dispatch**: Passing image attachments to Anthropic (Claude), OpenAI (GPT-4o), OpenRouter, and Ollama vision models using their native multimodal payloads.

---

## 2. Architecture & Component Changes

```
┌────────────────────────────────────────────────────────────────────────┐
│                        User Terminal / Prompt                          │
│   Ctrl+V / Event::Paste / /image -> Prompt::handle_paste_or_image()    │
└────────────────────────────────────┬───────────────────────────────────┘
                                     │ (extract image if present)
                                     ▼
┌────────────────────────────────────────────────────────────────────────┐
│                     Clipboard Manager (arboard)                        │
│   - Detects image in OS clipboard (RGBA8 pixels)                       │
│   - Encodes to PNG via `image` crate                                   │
│   - Writes to `.fusion/cache/images/clip_<timestamp>_<hash>.png`       │
└────────────────────────────────────┬───────────────────────────────────┘
                                     │ returns ImageAttachmentInfo
                                     ▼
┌────────────────────────────────────────────────────────────────────────┐
│                        Prompt Buffer State                             │
│   - Inserts placeholder `[Image #1: 1280x720]` into text buffer        │
│   - Stores pending attachment record in `Prompt::pending_images`       │
│   - User can delete placeholder with Backspace                         │
└────────────────────────────────────┬───────────────────────────────────┘
                                     │ Enter submitted
                                     ▼
┌────────────────────────────────────────────────────────────────────────┐
│                     Loop Runner & Context Injector                     │
│   - Reconciles `[Image #N]` placeholders with active attachments       │
│   - Builds `ImageAttachment` (media_type, base64 data, dimensions)     │
│   - Populates `Message.images` on user message                         │
└────────────────────────────────────┬───────────────────────────────────┘
                                     │
         ┌───────────────────────────┼───────────────────────────┐
         ▼                           ▼                           ▼
┌─────────────────┐         ┌─────────────────┐         ┌─────────────────┐
│ Anthropic API   │         │ OpenAI / ORouter│         │ Ollama API      │
│ content blocks  │         │ image_url URIs  │         │ images: [...]   │
└─────────────────┘         └─────────────────┘         └─────────────────┘
```

---

## 3. Detailed Specifications

### 3.1 Dependencies
In `Cargo.toml`:
- Add `arboard = { workspace = true }` to `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`.
- Add `image = { workspace = true }` to `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`.

### 3.2 System Clipboard Service
Create a dedicated clipboard image extraction helper in `src/tools/clipboard.rs` (or `src/ui/clipboard_image.rs`):
- `pub fn read_clipboard_image() -> Option<PastedImage>`
- **Logic**:
  1. Initialize `arboard::Clipboard::new()`.
  2. Call `clipboard.get_image()`.
  3. If found:
     - Get width, height, and raw RGBA bytes.
     - Encode to PNG using `image::RgbaImage::from_raw(w, h, bytes)` and `image::codecs::png::PngEncoder`.
     - Compute sha256 hash or timestamp ID.
     - Ensure `.fusion/cache/images/` exists in project root or `~/.fusion/cache/images/`.
     - Write PNG file to disk.
     - Return `PastedImage { path: PathBuf, width: u32, height: u32, bytes_len: usize }`.
  4. If `get_image()` returns an error or no image, return `None`.

### 3.3 Prompt & Keybinding Integration
In `src/ui/prompt.rs` and `src/ui/keys.rs`:
- **`Ctrl+V` Handling**:
  - In `KeyHandler::handle_key` (both Emacs and Default profiles): map `(KeyCode::Char('v'), KeyModifiers::CONTROL)` to `KeyAction::PasteOrClipboardImage`.
  - When triggered:
    1. Check `read_clipboard_image()`.
    2. If an image is returned:
       - Format tag: `let tag = format!("[Image #{}: {}x{}]", next_idx, img.width, img.height);`
       - Insert `tag` at current cursor position in `prompt.buffer`.
       - Record attachment in `prompt.pending_images: Vec<PendingImageAttachment>`.
       - Request render update.
    3. If no image is present:
       - Fall back to reading clipboard text via `arboard` (or existing text paste).
- **Terminal `Event::Paste(text)` Handling**:
  - If `text.trim()` is empty (common when pasting OS clipboard image in terminal emulators):
    - Attempt `read_clipboard_image()`. If found, insert `[Image #N: WxH]`.
  - If `text` is a path to an existing image file (e.g. `.png`, `.jpg`, `.jpeg`, `.webp`, `.gif`, `.bmp`):
    - Read image dimensions, register as pending image, and insert `[Image #N: path]`.
  - Otherwise, insert text as normal characters.
- **Slash Commands**:
  - Register `/image` and `/paste` in `COMMAND_PALETTE` (`src/ui/prompt.rs`).
  - Running `/image` without arguments triggers clipboard image capture.
  - Running `/image <path>` validates and attaches the local image file.
- **Detaching & Editing**:
  - Standard Backspace removes characters. When prompt submits, it scans the buffer text for remaining `[Image #N: ...]` tags. Any images whose placeholder tag was removed by the user are automatically discarded.
  - `Ctrl+D` remains strictly mapped to EOF / exit prompt when buffer is empty, and delete char forward when buffer is non-empty.

### 3.4 Multimodal Provider Data Types
In `src/provider/types.rs`:
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageAttachment {
    pub media_type: String, // e.g. "image/png", "image/jpeg"
    pub data: String,       // base64 encoded string
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}
```
Extend `Message`:
```rust
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
```
Add helper:
```rust
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

### 3.5 Provider Serialization
1. **Anthropic (`src/provider/anthropic.rs`)**:
   - For `Role::User`, if `msg.images` has attachments:
     - Emit `AnthropicContentBlock::Text` for the message prompt.
     - For each image, emit:
       ```rust
       AnthropicContentBlock::Image {
           source: AnthropicImageSource {
               source_type: "base64".to_string(),
               media_type: img.media_type.clone(),
               data: img.data.clone(),
           }
       }
       ```
2. **OpenAI & OpenRouter (`src/provider/openai.rs`, `src/provider/openrouter.rs`)**:
   - If `msg.images` has attachments:
     - Serialize user message content as an array of parts:
       - Text part: `{"type": "text", "text": msg.content}`
       - Image parts: `{"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", img.media_type, img.data)}}`
3. **Ollama (`src/provider/ollama.rs`)**:
   - Map `msg.images` into Ollama's `images: Some(vec![img.data.clone()])`.

---

## 4. Error Handling & Edge Cases

1. **Non-graphical / Headless / SSH sessions**:
   - If `arboard` cannot connect to a display server (e.g. Linux headless or SSH without X11 forwarding), `read_clipboard_image()` returns `None`.
   - The CLI logs a debug message and gracefully falls back to text paste without error or crash.
2. **Huge Images / Memory Limits**:
   - Clamp images exceeding 4096px on any dimension or 20MB in raw size by downscaling with `image::imageops::resize` to ensure API limits are respected.
3. **Cache Cleanup**:
   - Image files in `.fusion/cache/images/` are named deterministically or with timestamps and kept in `.gitignore` so they do not pollute version control.

---

## 5. Testing Plan

1. **Unit Tests**:
   - `test_image_attachment_serialization`: verify JSON roundtrip of `Message` with and without `images`.
   - `test_anthropic_payload_with_images`: verify conversion of `Message` to `AnthropicMessage` content blocks.
   - `test_openai_payload_with_images`: verify conversion of `Message` to OpenAI content parts.
   - `test_placeholder_insertion_and_deletion`: verify prompt buffer text manipulation with `[Image #N]`.
   - `test_png_encode_decode_roundtrip`: verify RGBA bytes converted to PNG can be loaded by `image_view`.
2. **Integration / Manual Verification**:
   - Copy screenshot into clipboard on macOS (`Cmd+Shift+4`).
   - Run `cargo run --bin fusion`.
   - Press `Ctrl+V` in prompt: verify `[Image #1: WxH]` appears.
   - Type prompt: "What is this image?" and submit.
   - Verify multimodal LLM successfully interprets the visual content.
