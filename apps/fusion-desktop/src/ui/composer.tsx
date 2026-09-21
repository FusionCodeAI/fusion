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
        paddingLeft: 48,
        paddingRight: 48,
        backgroundColor: "#ffffff",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
      }}
    >
      <div
        style={{
          width: "100%",
          maxWidth: 760,
          display: "flex",
          flexDirection: "column",
          gap: 6,
        }}
      >
        {/* Upper Pill: Create Branch & Commit */}
        <div
          style={{
            alignSelf: "flex-start",
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            gap: 6,
            paddingTop: 4,
            paddingBottom: 4,
            paddingLeft: 10,
            paddingRight: 10,
            borderRadius: 14,
            backgroundColor: "#f4f4f5",
            borderWidth: 1,
            borderColor: "#e5e5e8",
            cursor: "pointer",
          }}
        >
          <text style={{ fontSize: 11, color: "#3f3f46", fontWeight: "500" }}>
            Create Branch & Commit
          </text>
          <svg source={icons.chevronDown} style={{ width: 9, height: 9, color: "#71717a" }} />
        </div>

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

        {/* Rounded Pill Composer Container */}
        <div
          style={{
            backgroundColor: "#ffffff",
            borderWidth: 1,
            borderColor: "#e5e5e8",
            borderRadius: 22,
            paddingTop: 6,
            paddingBottom: 6,
            paddingLeft: 12,
            paddingRight: 8,
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
          }}
        >
          {/* Left: Plus in soft circle */}
          <div
            style={{
              width: 22,
              height: 22,
              borderRadius: 11,
              backgroundColor: "#f4f4f5",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              cursor: "pointer",
              marginRight: 10,
            }}
          >
            <svg source={icons.plus} style={{ width: 11, height: 11, color: "#71717a" }} />
          </div>

          {/* Center: Input */}
          <input
            value={text}
            onChange={(e: { value?: string }) => setText(e.value ?? "")}
            onSubmit={handleSend}
            placeholder="Send follow-up"
            style={{
              flexGrow: 1,
              fontSize: 13,
              color: "#18181b",
              backgroundColor: "transparent",
              borderWidth: 0,
              padding: 0,
            }}
          />

          {/* Right Toolbar: Model Selector and Mic Button */}
          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 10 }}>
            {/* Model Pill */}
            <div
              onClick={() => setIsModelPickerOpen(!isModelPickerOpen)}
              style={{
                display: "flex",
                flexDirection: "row",
                alignItems: "center",
                gap: 4,
                cursor: "pointer",
                paddingTop: 2,
                paddingBottom: 2,
                paddingLeft: 6,
                paddingRight: 6,
                borderRadius: 6,
                backgroundColor: isModelPickerOpen ? "#f4f4f5" : "transparent",
                hover: { backgroundColor: "#f4f4f5" },
              }}
            >
              <text style={{ fontSize: 12, color: "#52525b", fontWeight: "500" }}>
                {activeModelObj.shortName}
              </text>
              <svg source={icons.chevronDown} style={{ width: 9, height: 9, color: "#71717a" }} />
            </div>

            {/* Mic / Action Button (Circular black button with white icon) */}
            {isGenerating ? (
              <div
                onClick={onCancel}
                style={{
                  width: 28,
                  height: 28,
                  borderRadius: 14,
                  backgroundColor: "#ef4444",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  cursor: "pointer",
                }}
              >
                <div style={{ width: 10, height: 10, borderRadius: 2, backgroundColor: "#ffffff" }} />
              </div>
            ) : (
              <div
                onClick={text.trim() ? handleSend : undefined}
                style={{
                  width: 28,
                  height: 28,
                  borderRadius: 14,
                  backgroundColor: "#000000",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  cursor: "pointer",
                }}
              >
                {text.trim() ? (
                  <svg source={icons.arrowRight} style={{ width: 13, height: 13, color: "#ffffff" }} />
                ) : (
                  <svg source={icons.mic} style={{ width: 13, height: 13, color: "#ffffff" }} />
                )}
              </div>
            )}
          </div>
        </div>

        {/* Sub-row below Composer: Branch, This Mac, Refresh */}
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            justifyContent: "space-between",
            paddingLeft: 8,
            paddingRight: 4,
            paddingTop: 4,
          }}
        >
          <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 14 }}>
            {/* Branch */}
            <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 4, cursor: "pointer" }}>
              <svg source={icons.branch} style={{ width: 11, height: 11, color: "#71717a" }} />
              <text style={{ fontSize: 11, color: "#71717a" }}>main</text>
              <svg source={icons.chevronDown} style={{ width: 8, height: 8, color: "#71717a" }} />
            </div>

            {/* This Mac */}
            <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 4, cursor: "pointer" }}>
              <svg source={icons.laptop} style={{ width: 12, height: 12, color: "#71717a" }} />
              <text style={{ fontSize: 11, color: "#71717a" }}>This Mac</text>
              <svg source={icons.chevronDown} style={{ width: 8, height: 8, color: "#71717a" }} />
            </div>
          </div>

          {/* Refresh icon */}
          <div style={{ cursor: "pointer", display: "flex", alignItems: "center" }}>
            <svg source={icons.refresh} style={{ width: 12, height: 12, color: "#71717a" }} />
          </div>
        </div>
      </div>
    </div>
  );
}
