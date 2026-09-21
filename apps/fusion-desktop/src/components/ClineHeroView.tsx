import React from "react";
import { ClineWatermark } from "./ClineWatermark";
import { WorkspacePill } from "./WorkspacePill";
import { ConnectModelBanner } from "./ConnectModelBanner";
import { Composer } from "./Composer";

export interface ClineHeroViewProps {
  workspaceName?: string;
  onConnectModel?: () => void;
  onModelSettings?: () => void;
  effort?: "Low" | "Medium" | "High";
  onSelectEffort?: (effort: "Low" | "Medium" | "High") => void;
  onSend: (text: string) => void;
  selectedModel?: string;
  onSelectModel?: (modelId: string) => void;
  billingProfile?: string;
  onAttachFile?: () => void;
  onPickWorkspaceFolder?: () => void;
  className?: string;
}

export function ClineHeroView({
  workspaceName = "workspace",
  onConnectModel,
  onModelSettings,
  onSend,
  selectedModel,
  onSelectModel,
  billingProfile,
  effort,
  onSelectEffort,
  onAttachFile,
  onPickWorkspaceFolder,
  className = "",
}: ClineHeroViewProps) {
  return (
    <div
      data-testid="cline-hero-view"
      className={`relative w-full h-full flex flex-col items-center justify-center px-4 overflow-hidden ${className}`}
    >
      {/* Background Watermark */}
      <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
        <ClineWatermark className="w-56 h-56 -mt-16" />
      </div>

      {/* Main Foreground Container */}
      <div className="relative z-10 w-full max-w-[760px] flex flex-col items-center space-y-4">
        {/* Workspace Pill */}
        <div className="flex justify-center">
          <WorkspacePill name={workspaceName} onClick={onPickWorkspaceFolder} />
        </div>

        {/* Connect Model Banner */}
        <ConnectModelBanner
          onConnect={onConnectModel}
          onSettings={onModelSettings}
        />

        {/* Floating Composer Box */}
        <div className="w-full">
          <Composer
            onSend={onSend}
            selectedModel={selectedModel}
            onSelectModel={onSelectModel}
            billingProfile={billingProfile}
            effort={effort}
            onSelectEffort={onSelectEffort}
            onAttachFile={onAttachFile}
            autoFocus
          />
        </div>
      </div>
    </div>
  );
}

export default ClineHeroView;
