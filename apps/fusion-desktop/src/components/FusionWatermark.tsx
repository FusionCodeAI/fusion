import React from "react";

export interface FusionWatermarkProps {
  className?: string;
}

export function FusionWatermark({ className = "w-56 h-56" }: FusionWatermarkProps) {
  return (
    <div
      data-testid="fusion-watermark"
      className={`pointer-events-none select-none flex items-center justify-center opacity-[0.06] text-[#5100cd] ${className}`}
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 1024 1024"
        className="w-full h-full"
        fill="currentColor"
      >
        {/* Outer bracket ┌ */}
        <path d="M110 110 H864 V210 H210 V864 H110 Z" />
        {/* Inner bracket ┌ */}
        <path d="M382 382 H864 V482 H482 V864 H382 Z" />
      </svg>
    </div>
  );
}

export default FusionWatermark;
