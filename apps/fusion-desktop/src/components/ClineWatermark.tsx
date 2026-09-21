import React from "react";
import { ClineRobotSvg } from "./ClineAvatar";

export interface ClineWatermarkProps {
  className?: string;
}

export function ClineWatermark({ className = "w-56 h-56" }: ClineWatermarkProps) {
  return (
    <div
      data-testid="cline-watermark"
      className={`pointer-events-none select-none flex items-center justify-center opacity-[0.06] text-purple-900 ${className}`}
    >
      <ClineRobotSvg
        className="w-full h-full"
        color="currentColor"
        strokeWidth={4}
      />
    </div>
  );
}

export default ClineWatermark;
