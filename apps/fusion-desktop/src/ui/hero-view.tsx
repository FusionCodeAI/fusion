import React, { useState } from "react";
import { icons } from "./icons";
import { FUSION_MODELS, DEFAULT_FUSION_MODEL, type FusionModel } from "../models";

export interface HeroViewProps {
  onSubmit: (prompt: string) => void;
  onOpenFolder?: () => void;
  workspaceDir?: string;
  selectedModel?: string;
  onSelectModel?: (modelId: string) => void;
}

export function HeroView({
  onSubmit,
  onOpenFolder,
  workspaceDir = "fusion",
  selectedModel = DEFAULT_FUSION_MODEL.id,
  onSelectModel,
}: HeroViewProps) {
  const [prompt, setPrompt] = useState("");
  const [isModelPickerOpen, setIsModelPickerOpen] = useState(false);

  const activeModelObj =
    FUSION_MODELS.find((m) => m.id === selectedModel || m.name === selectedModel) ||
    DEFAULT_FUSION_MODEL;

  const handleSubmit = () => {
    const trimmed = prompt.trim();
    if (trimmed) {
      onSubmit(trimmed);
      setPrompt("");
    }
  };

  const handleSelectModel = (model: FusionModel) => {
    onSelectModel?.(model.id);
    setIsModelPickerOpen(false);
  };

  const folderName = workspaceDir.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || "fusion";

  return (
    <div
      style={{
        flexGrow: 1,
        minHeight: 0,
        height: "100%",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        paddingLeft: 40,
        paddingRight: 40,
        backgroundColor: "#ffffff",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          width: "100%",
          maxWidth: 680,
          position: "relative",
          display: "flex",
          flexDirection: "column",
        }}
      >
        {/* Workspace folder pill above composer */}
        <div
          onClick={onOpenFolder}
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            gap: 6,
            alignSelf: "flex-start",
            paddingTop: 4,
            paddingBottom: 6,
            paddingLeft: 4,
            cursor: "pointer",
          }}
        >
          <svg source={icons.folder} style={{ width: 14, height: 14, color: "#71717a" }} />
          <text style={{ fontSize: 13, color: "#3f3f46", fontWeight: "500" }}>
            {folderName}
          </text>
          <svg source={icons.chevronDown} style={{ width: 11, height: 11, color: "#71717a" }} />
        </div>

        {/* Model Picker Popup Dropdown if open */}
        {isModelPickerOpen && (
          <div
            style={{
              marginBottom: 8,
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

        {/* Floating Composer Card */}
        <div
          style={{
            backgroundColor: "#ffffff",
            borderWidth: 1,
            borderColor: "#e5e5e8",
            borderRadius: 14,
            paddingTop: 14,
            paddingBottom: 10,
            paddingLeft: 16,
            paddingRight: 14,
            display: "flex",
            flexDirection: "column",
          }}
        >
          <input
            value={prompt}
            onChange={(e: { value?: string }) => setPrompt(e.value ?? "")}
            onSubmit={handleSubmit}
            placeholder="Ask anything, @ to mention, / for actions"
            style={{
              minHeight: 44,
              width: "100%",
              fontSize: 14,
              lineHeight: 22,
              color: "#18181b",
              backgroundColor: "transparent",
              borderWidth: 0,
              padding: 0,
            }}
          />

          {/* Bottom Toolbar inside Card */}
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
            {/* Left: Plus Context & Model Dropdown */}
            <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 10 }}>
              <div
                style={{
                  cursor: "pointer",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  width: 22,
                  height: 22,
                  borderRadius: 6,
                  hover: { backgroundColor: "#f4f4f5" },
                }}
              >
                <svg source={icons.plus} style={{ width: 14, height: 14, color: "#71717a" }} />
              </div>

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
                  paddingLeft: 4,
                  paddingRight: 4,
                  borderRadius: 4,
                  backgroundColor: isModelPickerOpen ? "#ebebec" : "transparent",
                  hover: { backgroundColor: "#f4f4f5" },
                }}
              >
                <text style={{ fontSize: 12, color: "#52525b", fontWeight: "500" }}>
                  {activeModelObj.name}
                </text>
                <svg source={icons.chevronDown} style={{ width: 10, height: 10, color: "#71717a" }} />
              </div>
            </div>

            {/* Right: Mic & Circular Arrow Send */}
            <div style={{ display: "flex", flexDirection: "row", alignItems: "center", gap: 8 }}>
              <div
                style={{
                  cursor: "pointer",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  width: 26,
                  height: 26,
                  borderRadius: 6,
                  hover: { backgroundColor: "#f4f4f5" },
                }}
              >
                <svg source={icons.mic} style={{ width: 14, height: 14, color: "#71717a" }} />
              </div>

              <div
                onClick={handleSubmit}
                style={{
                  width: 28,
                  height: 28,
                  borderRadius: 14,
                  backgroundColor: prompt.trim() ? "#18181b" : "#f4f4f5",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  cursor: prompt.trim() ? "pointer" : "default",
                  hover: prompt.trim() ? { backgroundColor: "#27272a" } : {},
                }}
              >
                <svg
                  source={icons.arrowRight}
                  style={{
                    width: 13,
                    height: 13,
                    color: prompt.trim() ? "#ffffff" : "#a1a1aa",
                  }}
                />
              </div>
            </div>
          </div>
        </div>

        {/* Attached Sub-pill below Composer: Local */}
        <div
          style={{
            display: "flex",
            flexDirection: "row",
            alignItems: "center",
            gap: 6,
            alignSelf: "flex-start",
            marginTop: 6,
            marginLeft: 4,
            paddingTop: 4,
            paddingBottom: 4,
            paddingLeft: 8,
            paddingRight: 8,
            borderRadius: 6,
            backgroundColor: "#f4f4f5",
          }}
        >
          <svg source={icons.laptop} style={{ width: 12, height: 12, color: "#71717a" }} />
          <text style={{ fontSize: 11, color: "#71717a", fontWeight: "500" }}>Local</text>
          <svg source={icons.chevronDown} style={{ width: 9, height: 9, color: "#71717a" }} />
        </div>
      </div>
    </div>
  );
}
