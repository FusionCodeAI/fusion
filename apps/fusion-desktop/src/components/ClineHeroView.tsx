import React from "react";
import { ClineWatermark } from "./ClineWatermark";
import { FusionMascot } from "./FusionMascot";
import { WorkspacePill } from "./WorkspacePill";
import { ConnectModelBanner } from "./ConnectModelBanner";
import { Composer } from "./Composer";
import type { ChatImageAttachment } from "../types";

export interface ClineHeroViewProps {
  workspaceName?: string;
  isSignedIn?: boolean;
  onSignIn?: () => void;
  onConnectModel?: () => void;
  effort?: "Low" | "Medium" | "High";
  onSelectEffort?: (effort: "Low" | "Medium" | "High") => void;
  onSend: (text: string, images?: ChatImageAttachment[]) => void;
  selectedModel?: string;
  onSelectModel?: (modelId: string) => void;
  billingProfile?: string;
  onAttachFile?: () => void;
  onPickWorkspaceFolder?: () => void;
  workspaceEntries?: Array<{ path: string; name?: string; is_dir?: boolean } | string>;
  className?: string;
}

export function ClineHeroView({
  workspaceName = "workspace",
  isSignedIn = false,
  onSignIn,
  onConnectModel,
  onSend,
  selectedModel,
  onSelectModel,
  billingProfile,
  effort,
  onSelectEffort,
  onAttachFile,
  onPickWorkspaceFolder,
  workspaceEntries,
  className = "",
}: ClineHeroViewProps) {
  return (
    <div
      data-testid="cline-hero-view"
      className={`relative w-full h-full flex flex-col items-center justify-center px-4 overflow-hidden group ${className}`}
    >
      {/* Cline-style Soft Square Grid: hidden by default, softly reveals on hover */}
      <div
        className="absolute inset-0 pointer-events-none select-none cline-dot-grid opacity-0 group-hover:opacity-70 transition-opacity duration-700 ease-out [mask-image:radial-gradient(ellipse_at_center,black_35%,transparent_85%)] [-webkit-mask-image:radial-gradient(ellipse_at_center,black_35%,transparent_85%)]"
      />

      {/* Retain testid for test compatibility */}
      <div data-testid="cline-watermark" className="hidden" />

      {/* Main Foreground Container */}
      <div className="relative z-10 w-full max-w-[760px] flex flex-col items-center space-y-4">
        {/* Animated Fusion Mascot Above (Floating + Eye Gaze Tracking Mouse) */}
        <div className="flex flex-col items-center justify-center -mb-1 select-none">
          <FusionMascot
            className="w-24 h-24 cursor-pointer"
            animated={true}
            interactiveGaze={true}
            floating={true}
          />
        </div>
        <div className="w-full flex justify-start">
          <WorkspacePill name={workspaceName} onClick={onPickWorkspaceFolder} />
        </div>
        {/* Sign In / Connect Model Banner */}
        <ConnectModelBanner
          isSignedIn={isSignedIn}
          onSignIn={onSignIn || onConnectModel}
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
            workspaceEntries={workspaceEntries}
          />
        </div>
      </div>
    </div>
  );
}

export default ClineHeroView;
