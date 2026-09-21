import React, { useState } from "react";
import { icons } from "./icons";
import { FUSION_MODELS, DEFAULT_FUSION_MODEL, type FusionModel } from "../models";

export interface ComposerProps {
  onSend: (text: string) => void;
  onCancel?: () => void;
  isGenerating?: boolean;
  selectedModel?: string;
  onSelectModel?: (modelId: string) => void;
}

export function Composer({
  onSend,
  onCancel,
  isGenerating = false,
  selectedModel = DEFAULT_FUSION_MODEL.id,
  onSelectModel,
}: ComposerProps) {
  const [text, setText] = useState("");
  const [isModelPickerOpen, setIsModelPickerOpen] = useState(false);

  const activeModelObj =
    FUSION_MODELS.find((m) => m.id === selectedModel || m.name === selectedModel) ||
    DEFAULT_FUSION_MODEL;

  const handleSend = () => {
    const trimmed = text.trim();
    if (trimmed && !isGenerating) {
      onSend(trimmed);
      setText("");
      setIsModelPickerOpen(false);
    }
  };

  const handleTextChange = (val: string) => {
    setText(val);
    if (isModelPickerOpen) {
      setIsModelPickerOpen(false);
    }
  };

  const handleSelectModel = (model: FusionModel) => {
    onSelectModel?.(model.id);
    setIsModelPickerOpen(false);
  };

  return (
    <div
      style={{
        width: "100%",
        flexShrink: 0,
        paddingTop: 8,
        paddingBottom: 16,
        paddingLeft: 32,
        paddingRight: 32,
        backgroundColor: "#ffffff",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
      }}
    >
      <div
        style={{
          width: "100%",
          maxWidth: 680,
          position: "relative",
          display: "flex",
          flexDirection: "column",
          gap: 6,
        }}
      >
        {/* Model Picker Popup Dropdown if open */}
        {isModelPickerOpen && (
          <div
            style={{
              backgroundColor: "#ffffff",
              borderWidth: 1,
              borderColor: "#e5e5e8",
              borderRadius: 12,
              padding: 6,
              display: "flex",
              flexDirection: "column",
              gap: 2,
              marginBottom: 6,
            }}
          >
            <div style={{ paddingLeft: 8, paddingTop: 4, paddingBottom: 4 }}>
              <text style={{ fontSize: 11, fontWeight: "600", color: "#8e8e93" }}>
                FUSION AI MODELS
              </text>
            </div>

            {FUSION_MODELS.map((m) => {
              const isSelected = m.id === activeModelObj.id;
              return (
                <div
                  key={m.id}
                  onClick={() => handleSelectModel(m)}
                  style={{
                    display: "flex",
                    flexDirection: "row",
                    alignItems: "center",
                    justifyContent: "space-between",
                    paddingTop: 6,
                    paddingBottom: 6,
                    paddingLeft: 10,
                    paddingRight: 10,
                    borderRadius: 6,
                    backgroundColor: isSelected ? "#f4f4f5" : "transparent",
                    cursor: "pointer",
                    hover: { backgroundColor: "#f4f4f5" },
                  }}
                >
                  <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
                    <text
                      style={{
                        fontSize: 13,
                        fontWeight: isSelected ? "600" : "400",
                        color: "#18181b",
                      }}
                    >
                      {m.name}
                    </text>
                    <div
                      style={{
                        paddingTop: 1,
                        paddingBottom: 1,
                        paddingLeft: 6,
                        paddingRight: 6,
                        borderRadius: 4,
                        backgroundColor: isSelected ? "#e4e4e7" : "#f4f4f5",
                      }}
                    >
                      <text style={{ fontSize: 10, color: "#71717a" }}>{m.badge}</text>
                    </div>
                  </div>

                  <text style={{ fontSize: 11, color: "#a1a1aa" }}>{m.contextWindow}</text>
                </div>
              );
            })}
          </div>
        )}

        {/* Elevated Multi-Row Composer Card (NOT a capsule pill!) */}
        <div
          style={{
            backgroundColor: "#ffffff",
            borderWidth: 1,
            borderColor: "#e5e5e8",
            borderRadius: 12,
            paddingTop: 12,
            paddingBottom: 10,
            paddingLeft: 14,
            paddingRight: 12,
            display: "flex",
            flexDirection: "column",
          }}
        >
          {/* Multi-line input area */}
          <textarea
            value={text}
            onChange={(e: { value?: string }) => handleTextChange(e.value ?? "")}
            onSubmit={handleSend}
            placeholder="Send follow-up"
            minRows={2}
            maxRows={8}
            theme={{
              caret: "#18181b",
            }}
            style={{
              width: "100%",
              fontSize: 14,
              lineHeight: 22,
              color: "#18181b",
              backgroundColor: "transparent",
              borderWidth: 0,
              paddingTop: 4,
              paddingBottom: 8,
              paddingLeft: 2,
              paddingRight: 2,
            }}
          />

          {/* Bottom Toolbar inside the card */}
          <div
            style={{
              display: "flex",
              flexDirection: "row",
              alignItems: "center",
              justifyContent: "space-between",
              paddingTop: 8,
              marginTop: 4,
            }}
          >
            {/* Left: Plus context & Model selector pill */}
            <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
              <div
                style={{
                  width: 22,
                  height: 22,
                  borderRadius: 6,
                  backgroundColor: "#f4f4f5",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  cursor: "pointer",
                  hover: { backgroundColor: "#e4e4e7" },
                }}
              >
                <svg source={icons.plus} style={{ width: 12, height: 12, color: "#71717a" }} />
              </div>

              <div
                onClick={() => setIsModelPickerOpen(!isModelPickerOpen)}
                style={{
                  display: "flex",
                  flexDirection: "row",
                  alignItems: "center",
                  gap: 5,
                  cursor: "pointer",
                  paddingTop: 3,
                  paddingBottom: 3,
                  paddingLeft: 8,
                  paddingRight: 8,
                  borderRadius: 6,
                  backgroundColor: isModelPickerOpen ? "#ebebec" : "#f4f4f5",
                  hover: { backgroundColor: "#e4e4e7" },
                }}
              >
                <text style={{ fontSize: 12, color: "#3f3f46", fontWeight: "500" }}>
                  {activeModelObj.shortName}
                </text>
                <svg source={icons.chevronDown} style={{ width: 9, height: 9, color: "#71717a" }} />
              </div>
            </div>

            {/* Right: Mic & Send/Stop action */}
            <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
              <div
                style={{
                  cursor: "pointer",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  width: 24,
                  height: 24,
                  borderRadius: 6,
                  hover: { backgroundColor: "#f4f4f5" },
                }}
              >
                <svg source={icons.mic} style={{ width: 14, height: 14, color: "#71717a" }} />
              </div>

              {isGenerating ? (
                <div
                  onClick={onCancel}
                  style={{
                    width: 26,
                    height: 26,
                    borderRadius: 13,
                    backgroundColor: "#ef4444",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    cursor: "pointer",
                  }}
                >
                  <div style={{ width: 9, height: 9, borderRadius: 2, backgroundColor: "#ffffff" }} />
                </div>
              ) : (
                <div
                  onClick={text.trim() ? handleSend : undefined}
                  style={{
                    width: 26,
                    height: 26,
                    borderRadius: 13,
                    backgroundColor: text.trim() ? "#18181b" : "#f4f4f5",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    cursor: text.trim() ? "pointer" : "default",
                    hover: text.trim() ? { backgroundColor: "#27272a" } : {},
                  }}
                >
                  <svg
                    source={icons.arrowRightSubmit}
                    style={{
                      width: 12,
                      height: 12,
                      color: text.trim() ? "#ffffff" : "#a1a1aa",
                    }}
                  />
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Sub-row below Composer: Branch on Left, Context Meter on Right (NO This Mac!) */}
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            paddingLeft: 6,
            paddingRight: 4,
            paddingTop: 2,
          }}
        >
          {/* Branch indicator */}
          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 4, cursor: "pointer" }}>
            <svg source={icons.branch} style={{ width: 11, height: 11, color: "#71717a" }} />
            <text style={{ fontSize: 11, color: "#71717a" }}>main</text>
            <svg source={icons.chevronDown} style={{ width: 8, height: 8, color: "#71717a" }} />
          </div>

          {/* Context Limit Usage Meter on the far right */}
          <div style={{ cursor: "pointer", display: "flex", alignItems: "center" }}>
            <svg source={icons.contextMeter} style={{ width: 13, height: 13, color: "#71717a" }} />
          </div>
        </div>
      </div>
    </div>
  );
}
