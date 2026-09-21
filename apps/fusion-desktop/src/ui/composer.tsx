import React, { useState } from "react";

export interface ComposerProps {
  onSend: (text: string) => void;
  onCancel?: () => void;
  isGenerating?: boolean;
  selectedModel?: string;
  onSelectModel?: (model: string) => void;
}

export function Composer({
  onSend,
  onCancel,
  isGenerating = false,
  selectedModel = "Auto",
  onSelectModel,
}: ComposerProps) {
  const [text, setText] = useState("");

  const handleSend = () => {
    const trimmed = text.trim();
    if (trimmed && !isGenerating) {
      onSend(trimmed);
      setText("");
    }
  };

  return (
    <div
      style={{
        width: "100%",
        paddingTop: 16,
        paddingBottom: 16,
        paddingLeft: 24,
        paddingRight: 24,
        backgroundColor: "#0d1117",
        borderTopWidth: 1,
        borderColor: "#21262d",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
      }}
    >
      <div
        style={{
          width: "100%",
          maxWidth: 780,
          backgroundColor: "#161b22",
          borderWidth: 1,
          borderColor: "#30363d",
          borderRadius: 14,
          paddingTop: 10,
          paddingBottom: 10,
          paddingLeft: 14,
          paddingRight: 14,
          display: "flex",
          flexDirection: "column",
        }}
      >
        <input
          value={text}
          onChange={(e: { value?: string }) => setText(e.value ?? "")}
          onSubmit={handleSend}
          placeholder="Send follow-up instructions..."
          style={{
            minHeight: 40,
            width: "100%",
            fontSize: 14,
            lineHeight: 20,
            color: "#f0f6fc",
            backgroundColor: "transparent",
            borderWidth: 0,
            padding: 4,
          }}
        />

        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            marginTop: 8,
            paddingTop: 8,
            borderTopWidth: 1,
            borderColor: "#21262d",
          }}
        >
          {/* Target and model chips */}
          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 10 }}>
            <div
              style={{
                display: "flex",
                flexDirection: "row",
                alignItems: "center",
                gap: 6,
                paddingTop: 4,
                paddingBottom: 4,
                paddingLeft: 10,
                paddingRight: 10,
                borderRadius: 6,
                backgroundColor: "#21262d",
              }}
            >
              <text style={{ fontSize: 12, color: "#8b949e" }}>💻 This Mac ▾</text>
            </div>

            <div
              onClick={() => onSelectModel?.(selectedModel === "Auto" ? "claude-3-7-sonnet" : "Auto")}
              style={{
                display: "flex",
                flexDirection: "row",
                alignItems: "center",
                gap: 6,
                paddingTop: 4,
                paddingBottom: 4,
                paddingLeft: 10,
                paddingRight: 10,
                borderRadius: 6,
                backgroundColor: "#21262d",
                cursor: "pointer",
                hover: { backgroundColor: "#30363d" },
              }}
            >
              <text style={{ fontSize: 12, color: "#58a6ff" }}>{selectedModel} ▾</text>
            </div>
          </div>

          {/* Action button: Send or Stop */}
          <div>
            {isGenerating ? (
              <div
                onClick={onCancel}
                style={{
                  paddingTop: 6,
                  paddingBottom: 6,
                  paddingLeft: 14,
                  paddingRight: 14,
                  borderRadius: 6,
                  backgroundColor: "#da3633",
                  cursor: "pointer",
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  hover: { backgroundColor: "#f85149" },
                }}
              >
                <text style={{ fontSize: 12, fontWeight: "600", color: "#ffffff" }}>■ Stop</text>
              </div>
            ) : (
              <div
                onClick={handleSend}
                style={{
                  paddingTop: 6,
                  paddingBottom: 6,
                  paddingLeft: 14,
                  paddingRight: 14,
                  borderRadius: 6,
                  backgroundColor: text.trim() ? "#238636" : "#21262d",
                  cursor: text.trim() ? "pointer" : "default",
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  hover: text.trim() ? { backgroundColor: "#2ea043" } : {},
                }}
              >
                <text
                  style={{
                    fontSize: 12,
                    fontWeight: "600",
                    color: text.trim() ? "#ffffff" : "#6e7681",
                  }}
                >
                  Send ↑
                </text>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
