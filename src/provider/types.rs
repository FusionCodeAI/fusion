use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl PartialEq for ToolCall {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.name == other.name && self.arguments == other.arguments
    }
}
impl Eq for ToolCall {}

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

impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        self.role == other.role
            && self.content == other.content
            && self.images == other.images
            && self.name == other.name
            && self.tool_calls == other.tool_calls
            && self.tool_call_id == other.tool_call_id
    }
}
impl Eq for Message {}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            images: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

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

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            images: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            images: None,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant_with_tools(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            images: None,
            name: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            images: None,
            name: None,
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum StreamChunk {
    ContentDelta(String),
    ThinkingDelta(String),
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: String,
    },
    Done {
        finish_reason: Option<String>,
        prompt_tokens: Option<u32>,
        completion_tokens: Option<u32>,
    },
    Error(String),
}

/// Deduplicates streaming tokens that may be delivered as cumulative snapshots
/// (e.g. MiniMax reasoning streams or certain local proxy engines) rather than incremental deltas.
#[derive(Debug, Default, Clone)]
pub struct CumulativeDeduplicator {
    last_snapshot: String,
}

impl CumulativeDeduplicator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a chunk and returns only the incremental new text delta.
    pub fn feed<'a>(&mut self, chunk: &'a str) -> &'a str {
        if chunk.is_empty() {
            return "";
        }

        if !self.last_snapshot.is_empty() && chunk.starts_with(&self.last_snapshot) {
            // Cumulative snapshot detected: slice only the newly appended text
            let new_chars = &chunk[self.last_snapshot.len()..];
            self.last_snapshot = chunk.to_string();
            new_chars
        } else {
            // Normal incremental delta or non-prefix break
            self.last_snapshot.push_str(chunk);
            chunk
        }
    }

    pub fn reset(&mut self) {
        self.last_snapshot.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cumulative_deduplicator_with_deltas() {
        let mut dedup = CumulativeDeduplicator::new();
        assert_eq!(dedup.feed("Hello "), "Hello ");
        assert_eq!(dedup.feed("world!"), "world!");
    }

    #[test]
    fn test_cumulative_deduplicator_with_cumulative_snapshots() {
        let mut dedup = CumulativeDeduplicator::new();
        assert_eq!(dedup.feed("Hello "), "Hello ");
        assert_eq!(dedup.feed("Hello world "), "world ");
        assert_eq!(dedup.feed("Hello world from "), "from ");
        assert_eq!(dedup.feed("Hello world from MiniMax"), "MiniMax");
    }

    #[test]
    fn test_cumulative_deduplicator_duplicate_chunk() {
        let mut dedup = CumulativeDeduplicator::new();
        assert_eq!(dedup.feed("Thinking..."), "Thinking...");
        assert_eq!(dedup.feed("Thinking..."), "");
        assert_eq!(dedup.feed("Thinking... Done"), " Done");
    }

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
}
