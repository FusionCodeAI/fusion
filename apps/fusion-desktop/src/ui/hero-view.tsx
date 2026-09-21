import React, { useState } from "react";

export interface HeroViewProps {
  onSubmit: (prompt: string) => void;
  onOpenFolder?: () => void;
  selectedModel?: string;
}

export function HeroView({ onSubmit, onOpenFolder, selectedModel = "Auto" }: HeroViewProps) {
  const [prompt, setPrompt] = useState("");

  const handleSubmit = () => {
    const trimmed = prompt.trim();
    if (trimmed) {
      onSubmit(trimmed);
      setPrompt("");
    }
  };

  const handleQuickPrompt = (text: string) => {
    onSubmit(text);
  };

  return (
    <div
      style={{
        flexGrow: 1,
        height: "100%",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        padding: 40,
        backgroundColor: "#0d1117",
      }}
    >
      {/* Title */}
      <text
        style={{
          fontSize: 32,
          fontWeight: "600",
          color: "#f0f6fc",
          marginBottom: 8,
          textAlign: "center",
        }}
      >
        What should we build?
      </text>

      <text
        style={{
          fontSize: 14,
          color: "#8b949e",
          marginBottom: 32,
          textAlign: "center",
        }}
      >
        Autonomous agent control plane. Type a prompt to begin.
      </text>

      {/* Hero Composer Card */}
      <div
        style={{
          width: "100%",
          maxWidth: 680,
          backgroundColor: "#161b22",
          borderWidth: 1,
          borderColor: "#30363d",
          borderRadius: 16,
          padding: 16,
          display: "flex",
          flexDirection: "column",
          marginBottom: 20,
        }}
      >
        <input
          value={prompt}
          onChange={(e: { value?: string }) => setPrompt(e.value ?? "")}
          onSubmit={handleSubmit}
          placeholder="Describe a task, ask a question, or edit code..."
          style={{
            minHeight: 64,
            width: "100%",
            fontSize: 16,
            lineHeight: 24,
            color: "#f0f6fc",
            backgroundColor: "transparent",
            borderWidth: 0,
            marginBottom: 16,
            padding: 8,
          }}
        />

        {/* Action bar inside input */}
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            paddingTop: 8,
            borderTopWidth: 1,
            borderColor: "#21262d",
          }}
        >
          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
            <div
              style={{
                display: "flex",
                flexDirection: "row",
                alignItems: "center",
                gap: 6,
                paddingTop: 6,
                paddingBottom: 6,
                paddingLeft: 12,
                paddingRight: 12,
                borderRadius: 8,
                backgroundColor: "#21262d",
                cursor: "pointer",
                hover: { backgroundColor: "#30363d" },
              }}
              onClick={onOpenFolder}
            >
              <text style={{ fontSize: 13, color: "#c9d1d9" }}>📁 Open Folder</text>
            </div>

            <div
              style={{
                display: "flex",
                flexDirection: "row",
                alignItems: "center",
                gap: 6,
                paddingTop: 6,
                paddingBottom: 6,
                paddingLeft: 12,
                paddingRight: 12,
                borderRadius: 8,
                backgroundColor: "#21262d",
              }}
            >
              <text style={{ fontSize: 13, color: "#8b949e" }}>Model: </text>
              <text style={{ fontSize: 13, color: "#58a6ff", fontWeight: "500" }}>
                {selectedModel} ▾
              </text>
            </div>
          </div>

          <div
            onClick={handleSubmit}
            style={{
              paddingTop: 8,
              paddingBottom: 8,
              paddingLeft: 18,
              paddingRight: 18,
              borderRadius: 8,
              backgroundColor: prompt.trim() ? "#238636" : "#21262d",
              cursor: prompt.trim() ? "pointer" : "default",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              hover: prompt.trim() ? { backgroundColor: "#2ea043" } : {},
            }}
          >
            <text
              style={{
                fontSize: 14,
                fontWeight: "600",
                color: prompt.trim() ? "#ffffff" : "#6e7681",
              }}
            >
              Send ↑
            </text>
          </div>
        </div>
      </div>

      {/* Suggestion Quick Action Pills */}
      <div
        style={{
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          gap: 12,
          justifyContent: "center",
        }}
      >
        <div
          onClick={() => handleQuickPrompt("Scan this repository and explain the project architecture")}
          style={{
            paddingTop: 8,
            paddingBottom: 8,
            paddingLeft: 16,
            paddingRight: 16,
            borderRadius: 20,
            backgroundColor: "#161b22",
            borderWidth: 1,
            borderColor: "#30363d",
            cursor: "pointer",
            hover: { borderColor: "#58a6ff", backgroundColor: "#1c2128" },
          }}
        >
          <text style={{ fontSize: 13, color: "#c9d1d9" }}>⚡ Explain Project Architecture</text>
        </div>

        <div
          onClick={() => handleQuickPrompt("Find and fix any compiler warnings or failing tests")}
          style={{
            paddingTop: 8,
            paddingBottom: 8,
            paddingLeft: 16,
            paddingRight: 16,
            borderRadius: 20,
            backgroundColor: "#161b22",
            borderWidth: 1,
            borderColor: "#30363d",
            cursor: "pointer",
            hover: { borderColor: "#58a6ff", backgroundColor: "#1c2128" },
          }}
        >
          <text style={{ fontSize: 13, color: "#c9d1d9" }}>🛠️ Fix Failing Tests</text>
        </div>

        <div
          onClick={() => handleQuickPrompt("Plan and implement a new feature")}
          style={{
            paddingTop: 8,
            paddingBottom: 8,
            paddingLeft: 16,
            paddingRight: 16,
            borderRadius: 20,
            backgroundColor: "#161b22",
            borderWidth: 1,
            borderColor: "#30363d",
            cursor: "pointer",
            hover: { borderColor: "#58a6ff", backgroundColor: "#1c2128" },
          }}
        >
          <text style={{ fontSize: 13, color: "#c9d1d9" }}>✨ Plan New Feature</text>
        </div>
      </div>
    </div>
  );
}
