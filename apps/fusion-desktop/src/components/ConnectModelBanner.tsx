import React from "react";
import { Zap } from "lucide-react";

export interface ConnectModelBannerProps {
  onConnect?: () => void;
  onSettings?: () => void;
  className?: string;
}

export function ConnectModelBanner({
  onConnect,
  onSettings,
  className = "",
}: ConnectModelBannerProps) {
  return (
    <div
      data-testid="connect-model-banner"
      className={`w-full max-w-[760px] mx-auto rounded-xl border border-purple-200/70 bg-[#faf8ff] p-3.5 flex items-center justify-between shadow-2xs ${className}`}
    >
      <div className="flex items-center gap-3 min-w-0 pr-4">
        <div className="w-8 h-8 rounded-lg bg-purple-100/80 flex items-center justify-center text-[#5100cd] shrink-0">
          <Zap className="w-4 h-4 fill-current" />
        </div>
        <div className="min-w-0">
          <div className="text-xs font-semibold text-zinc-900 truncate">
            Connect a model to start building
          </div>
          <div className="text-[11px] text-zinc-500 truncate">
            Sign in with Fusion or add an API key — it takes under a minute.
          </div>
        </div>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        <button
          type="button"
          data-testid="banner-connect-btn"
          onClick={onConnect}
          className="bg-[#5100cd] hover:bg-[#4300a8] text-white text-xs font-medium rounded-full px-4 py-1.5 transition-colors shadow-xs cursor-pointer"
        >
          Connect a model
        </button>
        <button
          type="button"
          data-testid="banner-settings-btn"
          onClick={onSettings}
          className="text-xs font-medium text-zinc-600 hover:text-zinc-900 px-2.5 py-1.5 cursor-pointer"
        >
          Model settings
        </button>
      </div>
    </div>
  );
}

export default ConnectModelBanner;
