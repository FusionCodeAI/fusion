import React from "react";
import { Zap, LogIn } from "lucide-react";

export interface ConnectModelBannerProps {
  isSignedIn?: boolean;
  onSignIn?: () => void;
  onConnect?: () => void;
  className?: string;
}

export function ConnectModelBanner({
  isSignedIn = false,
  onSignIn,
  onConnect,
  className = "",
}: ConnectModelBannerProps) {
  // If the user is signed in, do not show the banner
  if (isSignedIn) {
    return null;
  }

  const handleAction = onSignIn || onConnect;

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
            Sign in to start building
          </div>
          <div className="text-[11px] text-zinc-500 truncate">
            Sign in with Fusion or add an API key — it takes under a minute.
          </div>
        </div>
      </div>
      <div className="flex items-center shrink-0">
        <button
          type="button"
          data-testid="banner-signin-btn"
          onClick={handleAction}
          className="bg-[#5100cd] hover:bg-[#4300a8] text-white text-xs font-medium rounded-full px-5 py-1.5 transition-colors shadow-xs cursor-pointer flex items-center gap-1.5"
        >
          <LogIn className="w-3.5 h-3.5" />
          <span>Sign in</span>
        </button>
      </div>
    </div>
  );
}

export default ConnectModelBanner;
