import React, { useState } from "react";
import type { ChatMessage, TurnStep } from "../types";

export interface ChatViewProps {
  sessionTitle: string;
  messages: readonly ChatMessage[];
  isGenerating?: boolean;
}

export function ChatView({ sessionTitle, messages, isGenerating = false }: ChatViewProps) {
  const [expandedThoughts, setExpandedThoughts] = useState<Record<string, boolean>>({});

  const toggleThought = (id: string) => {
    setExpandedThoughts((prev) => ({
      ...prev,
      [id]: !prev[id],
    }));
  };

  return (
    <div
      style={{
        flexGrow: 1,
        height: "100%",
        display: "flex",
        flexDirection: "column",
        backgroundColor: "#0d1117",
        overflow: "hidden",
      }}
    >
      {/* Chat Navigation Header */}
      <div
        style={{
          paddingTop: 12,
          paddingBottom: 12,
          paddingLeft: 24,
          paddingRight: 24,
          borderBottomWidth: 1,
          borderColor: "#21262d",
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
          backgroundColor: "#161b22",
        }}
      >
        <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
          <text style={{ fontSize: 15, fontWeight: "600", color: "#f0f6fc" }}>
            {sessionTitle || "New Conversation"}
          </text>
          <text style={{ fontSize: 13, color: "#8b949e" }}>🗄️</text>
        </div>

        <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 16 }}>
          <text style={{ fontSize: 12, color: "#3fb950" }}>● Ready</text>
          <text style={{ fontSize: 12, color: "#8b949e" }}>🌐 Browser</text>
          <text style={{ fontSize: 12, color: "#8b949e" }}>🖳 Terminal</text>
          <text style={{ fontSize: 12, color: "#8b949e" }}>📁 Files</text>
        </div>
      </div>

      {/* Messages Stream Container */}
      <div
        style={{
          flexGrow: 1,
          paddingTop: 24,
          paddingBottom: 24,
          paddingLeft: 40,
          paddingRight: 40,
          overflow: "scroll",
          display: "flex",
          flexDirection: "column",
          gap: 20,
        }}
      >
        {messages.map((msg) => {
          if (msg.role === "user") {
            return (
              <div
                key={msg.id}
                style={{
                  alignSelf: "flex-end",
                  maxWidth: 680,
                  backgroundColor: "#1f2937",
                  borderWidth: 1,
                  borderColor: "#374151",
                  borderRadius: 14,
                  paddingTop: 12,
                  paddingBottom: 12,
                  paddingLeft: 18,
                  paddingRight: 18,
                }}
              >
                <text style={{ fontSize: 14, lineHeight: 22, color: "#f3f4f6" }}>
                  {msg.content}
                </text>
              </div>
            );
          }

          // Assistant message
          const isExpanded = !!expandedThoughts[msg.id];
          return (
            <div
              key={msg.id}
              style={{
                alignSelf: "flex-start",
                width: "100%",
                maxWidth: 860,
                display: "flex",
                flexDirection: "column",
                gap: 12,
              }}
            >
              {/* Collapsible Thought Card if thought exists */}
              {msg.thought && (
                <div
                  style={{
                    backgroundColor: "#161b22",
                    borderWidth: 1,
                    borderColor: "#30363d",
                    borderRadius: 8,
                    overflow: "hidden",
                  }}
                >
                  <div
                    onClick={() => toggleThought(msg.id)}
                    style={{
                      paddingTop: 8,
                      paddingBottom: 8,
                      paddingLeft: 14,
                      paddingRight: 14,
                      display: "flex",
                      flexDirection: "row",
                      alignItems: "center",
                      justifyContent: "space-between",
                      cursor: "pointer",
                      backgroundColor: "#1c2128",
                      hover: { backgroundColor: "#21262d" },
                    }}
                  >
                    <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
                      <text style={{ fontSize: 12, color: "#8b949e" }}>🧠 Thought</text>
                      <text style={{ fontSize: 11, color: "#58a6ff" }}>
                        {isExpanded ? "▾ Hide reasoning" : "▸ Show reasoning"}
                      </text>
                    </div>
                  </div>

                  {isExpanded && (
                    <div
                      style={{
                        paddingTop: 10,
                        paddingBottom: 10,
                        paddingLeft: 14,
                        paddingRight: 14,
                        backgroundColor: "#0d1117",
                      }}
                    >
                      <text
                        style={{
                          fontSize: 12,
                          lineHeight: 18,
                          color: "#8b949e",
                          fontFamily: "monospace",
                        }}
                      >
                        {msg.thought}
                      </text>
                    </div>
                  )}
                </div>
              )}

              {/* Tool Execution Steps if any */}
              {msg.steps && msg.steps.length > 0 && (
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  {msg.steps.map((step: TurnStep) => (
                    <div
                      key={step.id}
                      style={{
                        paddingTop: 6,
                        paddingBottom: 6,
                        paddingLeft: 12,
                        paddingRight: 12,
                        borderRadius: 6,
                        backgroundColor: "#161b22",
                        borderWidth: 1,
                        borderColor: "#30363d",
                        display: "flex",
                        flexDirection: "row",
                        alignItems: "center",
                        gap: 8,
                      }}
                    >
                      <text style={{ fontSize: 12, color: step.status === "completed" ? "#3fb950" : "#d29922" }}>
                        {step.status === "completed" ? "✓" : "⚡"}
                      </text>
                      <text style={{ fontSize: 12, color: "#c9d1d9" }}>{step.title}</text>
                      {step.details && (
                        <text style={{ fontSize: 11, color: "#8b949e", paddingLeft: 8 }}>
                          {step.details}
                        </text>
                      )}
                    </div>
                  ))}
                </div>
              )}

              {/* Assistant Message Body */}
              <div
                style={{
                  backgroundColor: "#161b22",
                  borderWidth: 1,
                  borderColor: "#30363d",
                  borderRadius: 12,
                  paddingTop: 16,
                  paddingBottom: 16,
                  paddingLeft: 20,
                  paddingRight: 20,
                }}
              >
                <markdown
                  source={msg.content || (isGenerating ? "Thinking..." : "")}
                  style={{ color: "#f0f6fc", fontSize: 14, lineHeight: 22 }}
                />
              </div>

              {/* Optional Diff View */}
              {msg.diffPatch && (
                <div style={{ marginTop: 8 }}>
                  <diff
                    patch={msg.diffPatch}
                    wordDiff
                    style={{ borderRadius: 8, overflow: "hidden" }}
                  />
                </div>
              )}

              {/* Actions Footer */}
              <div
                style={{
                  display: "flex",
                  flexDirection: "row",
                  alignItems: "center",
                  gap: 12,
                  paddingLeft: 4,
                }}
              >
                <text style={{ fontSize: 12, color: "#8b949e" }}>👍</text>
                <text style={{ fontSize: 12, color: "#8b949e" }}>👎</text>
                <text style={{ fontSize: 12, color: "#8b949e" }}>📋 Copy</text>
                <text style={{ fontSize: 11, color: "#6e7681", marginLeft: 8 }}>Just now</text>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
