import React, { useState } from "react";
import type { ChatMessage, TurnStep } from "../types";
import { icons } from "./icons";

export interface ChatViewProps {
  sessionTitle: string;
  messages: readonly ChatMessage[];
  isGenerating?: boolean;
  onToggleSidebar?: () => void;
}

export function ChatView({
  sessionTitle,
  messages,
  isGenerating = false,
  onToggleSidebar,
}: ChatViewProps) {
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
        minHeight: 0,
        height: "100%",
        display: "flex",
        flexDirection: "column",
        backgroundColor: "#ffffff",
        overflow: "hidden",
      }}
    >
      {/* Top Header matching reference image: Title on Left, IDE / ... / Sidebar toggle on Right */}
      <div
        style={{
          flexShrink: 0,
          height: 44,
          paddingLeft: 20,
          paddingRight: 20,
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
          borderBottomWidth: 1,
          borderColor: "#f0f0f2",
          backgroundColor: "#ffffff",
        }}
      >
        {/* Title and Drawer Icon */}
        <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
          <text style={{ fontSize: 13, fontWeight: "500", color: "#18181b" }}>
            {sessionTitle || "General chat conversation"}
          </text>
          <svg source={icons.fileDrawer} style={{ width: 13, height: 13, color: "#71717a" }} />
        </div>

        {/* Right Header Items: IDE link, dots, and sidebar toggle */}
        <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 14 }}>
          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 4, cursor: "pointer" }}>
            <text style={{ fontSize: 12, color: "#71717a" }}>IDE</text>
            <svg source={icons.externalLink} style={{ width: 11, height: 11, color: "#71717a" }} />
          </div>

          <div style={{ cursor: "pointer", display: "flex", alignItems: "center" }}>
            <svg source={icons.dotsHorizontal} style={{ width: 14, height: 14, color: "#71717a" }} />
          </div>

          <div
            role="button"
            onClick={onToggleSidebar}
            style={{
              cursor: "pointer",
              display: "flex",
              alignItems: "center",
              padding: 4,
              borderRadius: 4,
              hover: { backgroundColor: "#f0f0f2" },
            }}
          >
            <svg source={icons.sidebarToggle} style={{ width: 14, height: 14, color: "#71717a" }} />
          </div>
        </div>
      </div>

      {/* Main Stream Area (Strictly vertical scrolling via virtual-list, ZERO 2D panning!) */}
      <div
        style={{
          flexGrow: 1,
          minHeight: 0,
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          paddingLeft: 36,
          paddingRight: 36,
          paddingTop: 16,
          paddingBottom: 24,
          overflow: "hidden",
          width: "100%",
        }}
      >
        <div
          style={{
            width: "100%",
            maxWidth: 720,
            height: "100%",
            display: "flex",
            flexDirection: "column",
          }}
        >
          <virtual-list
            alignment="top"
            followTail
            estimatedItemHeight={160}
            style={{ flexGrow: 1, minHeight: 0, width: "100%" }}
          >
            {messages.map((msg) => {
              if (msg.role === "user") {
                return (
                  <div
                    key={msg.id}
                    style={{
                      width: "100%",
                      paddingBottom: 16,
                      display: "flex",
                      flexDirection: "column",
                    }}
                  >
                    <div
                      style={{
                        width: "100%",
                        backgroundColor: "#ffffff",
                        borderWidth: 1,
                        borderColor: "#e5e5e8",
                        borderRadius: 14,
                        paddingTop: 12,
                        paddingBottom: 12,
                        paddingLeft: 18,
                        paddingRight: 18,
                      }}
                    >
                      <text style={{ fontSize: 14, lineHeight: 22, color: "#18181b" }}>
                        {msg.content}
                      </text>
                    </div>
                  </div>
                );
              }

              // Assistant message
              const isExpanded = !!expandedThoughts[msg.id];
              const visibleSteps = (msg.steps || []).filter(
                (s) =>
                  !s.title.toLowerCase().includes("waiting") &&
                  !s.title.toLowerCase().includes("think")
              );

              return (
                <div
                  key={msg.id}
                  style={{
                    width: "100%",
                    paddingBottom: 20,
                    display: "flex",
                    flexDirection: "column",
                    gap: 8,
                  }}
                >
                  {/* Thought briefly (collapsed by default) */}
                  {msg.thought && msg.thought.trim().length > 0 ? (
                    <div style={{ display: "flex", flexDirection: "column", alignSelf: "flex-start" }}>
                      <div
                        onClick={() => toggleThought(msg.id)}
                        style={{
                          display: "flex",
                          flexDirection: "row",
                          alignItems: "center",
                          gap: 6,
                          cursor: "pointer",
                          paddingTop: 2,
                          paddingBottom: 2,
                        }}
                      >
                        <text style={{ fontSize: 13, color: "#8e8e93" }}>
                          Thought briefly
                        </text>
                        <svg
                          source={isExpanded ? icons.chevronDown : icons.chevronRight}
                          style={{ width: 10, height: 10, color: "#8e8e93" }}
                        />
                      </div>

                      {isExpanded && (
                        <div
                          style={{
                            borderLeftWidth: 2,
                            borderColor: "#e5e5e8",
                            paddingLeft: 12,
                            paddingTop: 4,
                            paddingBottom: 4,
                            marginTop: 4,
                            width: "100%",
                            maxWidth: 680,
                          }}
                        >
                          <text
                            style={{
                              fontSize: 12,
                              lineHeight: 18,
                              color: "#71717a",
                              fontFamily: "monospace",
                            }}
                          >
                            {msg.thought}
                          </text>
                        </div>
                      )}
                    </div>
                  ) : null}

                  {/* Real Tool Steps if any */}
                  {visibleSteps.length > 0 && (
                    <div style={{ display: "flex", flexDirection: "column", gap: 4, alignSelf: "flex-start" }}>
                      {visibleSteps.map((step: TurnStep) => (
                        <div
                          key={step.id}
                          style={{
                            paddingTop: 3,
                            paddingBottom: 3,
                            paddingLeft: 8,
                            paddingRight: 8,
                            borderRadius: 6,
                            backgroundColor: "#f4f4f5",
                            display: "flex",
                            flexDirection: "row",
                            alignItems: "center",
                            gap: 6,
                          }}
                        >
                          <text style={{ fontSize: 11, color: step.status === "completed" ? "#16a34a" : "#d97706" }}>
                            {step.status === "completed" ? "✓" : "⚡"}
                          </text>
                          <text style={{ fontSize: 12, color: "#3f3f46" }}>{step.title}</text>
                          {step.details && (
                            <text style={{ fontSize: 11, color: "#71717a", paddingLeft: 4 }}>
                              {step.details}
                            </text>
                          )}
                        </div>
                      ))}
                    </div>
                  )}

                  {/* Assistant Markdown Content */}
                  {msg.content ? (
                    <div style={{ paddingTop: 2, paddingBottom: 2 }}>
                      <markdown
                        source={msg.content}
                        theme={{
                          appearance: "light",
                          accent: "#2563eb",
                        }}
                        style={{ color: "#18181b", fontSize: 14, lineHeight: 24 }}
                      />
                    </div>
                  ) : isGenerating ? (
                    <div style={{ paddingTop: 4, paddingBottom: 4 }}>
                      <text style={{ fontSize: 13, color: "#8e8e93" }}>Thinking...</text>
                    </div>
                  ) : null}

                  {/* Action Icons: Thumbs up, Thumbs down, Copy, Branch, Just now */}
                  <div
                    style={{
                      display: "flex",
                      flexDirection: "row",
                      alignItems: "center",
                      gap: 12,
                      paddingTop: 2,
                    }}
                  >
                    <div style={{ cursor: "pointer", display: "flex", alignItems: "center" }}>
                      <svg source={icons.thumbsUp} style={{ width: 13, height: 13, color: "#a1a1aa" }} />
                    </div>
                    <div style={{ cursor: "pointer", display: "flex", alignItems: "center" }}>
                      <svg source={icons.thumbsDown} style={{ width: 13, height: 13, color: "#a1a1aa" }} />
                    </div>
                    <div style={{ cursor: "pointer", display: "flex", alignItems: "center" }}>
                      <svg source={icons.copy} style={{ width: 13, height: 13, color: "#a1a1aa" }} />
                    </div>
                    <div style={{ cursor: "pointer", display: "flex", alignItems: "center" }}>
                      <svg source={icons.branch} style={{ width: 13, height: 13, color: "#a1a1aa" }} />
                    </div>
                    <text style={{ fontSize: 11, color: "#a1a1aa", marginLeft: 4 }}>Just now</text>
                  </div>
                </div>
              );
            })}
          </virtual-list>
        </div>
      </div>
    </div>
  );
}
