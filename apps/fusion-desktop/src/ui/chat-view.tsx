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
  const [collapsedThoughts, setCollapsedThoughts] = useState<Record<string, boolean>>({});

  const toggleThought = (id: string) => {
    setCollapsedThoughts((prev) => ({
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
      {/* Top Header matching Image #1: Title on Left, IDE / ... / Sidebar toggle on Right */}
      <div
        style={{
          flexShrink: 0,
          height: 40,
          paddingLeft: 16,
          paddingRight: 16,
          display: "flex",
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
          borderBottomWidth: 1,
          borderColor: "#f0f0f2",
          backgroundColor: "#ffffff",
        }}
      >
        {/* Left: General chat conversation [drawer icon] */}
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

      {/* Main Content: Conversation Stream on Left/Center, Info Drawer on Right */}
      <div
        style={{
          flexGrow: 1,
          minHeight: 0,
          display: "flex",
          flexDirection: "row",
          overflow: "hidden",
        }}
      >
        {/* Center Stream Area (Strictly vertical scrolling via virtual-list) */}
        <div
          style={{
            flexGrow: 1,
            minHeight: 0,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            paddingLeft: 32,
            paddingRight: 32,
            paddingTop: 16,
            paddingBottom: 24,
            overflow: "hidden",
            width: "100%",
          }}
        >
          <div
            style={{
              width: "100%",
              maxWidth: 680,
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
                const isCollapsed = !!collapsedThoughts[msg.id];
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
                    {/* Thought briefly (matching Image #1 exactly: no boxes, clean typography) */}
                    {msg.thought && msg.thought.trim().length > 0 ? (
                      <div style={{ display: "flex", flexDirection: "column", alignSelf: "flex-start", gap: 6 }}>
                        <div
                          onClick={() => toggleThought(msg.id)}
                          style={{
                            display: "flex",
                            flexDirection: "row",
                            alignItems: "center",
                            gap: 4,
                            cursor: "pointer",
                            paddingTop: 2,
                            paddingBottom: 2,
                          }}
                        >
                          <text style={{ fontSize: 13, color: "#71717a" }}>Thought </text>
                          <text style={{ fontSize: 13, color: "#8e8e93" }}>briefly</text>
                          <svg
                            source={isCollapsed ? icons.chevronRight : icons.chevronDown}
                            style={{ width: 10, height: 10, color: "#8e8e93", marginLeft: 2 }}
                          />
                        </div>

                        {/* Expanded thought text (clean unboxed muted gray markdown) */}
                        {!isCollapsed && (
                          <div
                            style={{
                              display: "flex",
                              flexDirection: "column",
                              gap: 6,
                              paddingTop: 2,
                              paddingBottom: 2,
                              width: "100%",
                              maxWidth: 680,
                            }}
                          >
                            <markdown
                              source={msg.thought}
                              theme={{
                                appearance: "light",
                                accent: "#2563eb",
                                codeText: "#18181b",
                                codeWash: "#f4f4f5",
                              }}
                              style={{
                                fontSize: 13,
                                lineHeight: 22,
                                color: "#52525b",
                              }}
                            />
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

                    {/* Assistant Markdown Content (crisp black text) */}
                    {msg.content ? (
                      <div style={{ paddingTop: 2, paddingBottom: 2 }}>
                        <markdown
                          source={msg.content}
                          theme={{
                            appearance: "light",
                            accent: "#2563eb",
                            codeText: "#18181b",
                            codeWash: "#f4f4f5",
                          }}
                          style={{ color: "#18181b", fontSize: 14, lineHeight: 24 }}
                        />
                      </div>
                    ) : isGenerating ? (
                      <div style={{ paddingTop: 4, paddingBottom: 4 }}>
                        <text style={{ fontSize: 13, color: "#8e8e93" }}>Thinking...</text>
                      </div>
                    ) : null}

                    {/* Native GPUIX Diff Viewer */}
                    {msg.diffPatch && (
                      <div style={{ marginTop: 8, width: "100%", borderRadius: 8, overflow: "hidden" }}>
                        <diff
                          patch={msg.diffPatch}
                          wordDiff
                          theme={{
                            appearance: "light",
                            diffAdd: "#16a34a",
                            diffDel: "#dc2626",
                            diffHunkBg: "#f4f4f5",
                          }}
                          style={{ borderRadius: 8 }}
                        />
                      </div>
                    )}

                    {/* Action Icons: Thumbs up, Thumbs down, Copy, Branch, Timestamp */}
                    <div
                      style={{
                        display: "flex",
                        flexDirection: "row",
                        alignItems: "center",
                        gap: 12,
                        paddingTop: 4,
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
                      <text style={{ fontSize: 11, color: "#a1a1aa", marginLeft: 4 }}>Just now</text>
                    </div>
                  </div>
                );
              })}
            </virtual-list>
          </div>
        </div>

        {/* Right Info Drawer (matching Image #1) */}
        <div
          style={{
            width: 130,
            flexShrink: 0,
            paddingTop: 20,
            paddingRight: 16,
            display: "flex",
            flexDirection: "column",
            gap: 10,
          }}
        >
          <text style={{ fontSize: 11, color: "#8e8e93", fontWeight: "500" }}>Open Tabs</text>
          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 6, cursor: "pointer" }}>
            <text style={{ fontSize: 12, color: "#3f3f46" }}>&gt;_ zsh</text>
          </div>

          <div style={{ height: 1, backgroundColor: "#f0f0f2", marginTop: 4, marginBottom: 4 }} />

          <text style={{ fontSize: 11, color: "#8e8e93", fontWeight: "500" }}>On kbtc-event</text>

          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 4 }}>
            <text style={{ fontSize: 12, color: "#71717a" }}>± Changes</text>
            <text style={{ fontSize: 12, color: "#16a34a", fontWeight: "500" }}>+4290</text>
            <text style={{ fontSize: 12, color: "#ef4444", fontWeight: "500" }}>-3</text>
          </div>

          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8, cursor: "pointer", paddingTop: 2 }}>
            <svg source={icons.browser} style={{ width: 13, height: 13, color: "#71717a" }} />
            <text style={{ fontSize: 12, color: "#3f3f46" }}>Browser</text>
          </div>

          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8, cursor: "pointer" }}>
            <svg source={icons.terminal} style={{ width: 13, height: 13, color: "#71717a" }} />
            <text style={{ fontSize: 12, color: "#3f3f46" }}>1 Terminal &gt;</text>
          </div>

          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8, cursor: "pointer" }}>
            <svg source={icons.files} style={{ width: 13, height: 13, color: "#71717a" }} />
            <text style={{ fontSize: 12, color: "#3f3f46" }}>Files</text>
          </div>
        </div>
      </div>
    </div>
  );
}
