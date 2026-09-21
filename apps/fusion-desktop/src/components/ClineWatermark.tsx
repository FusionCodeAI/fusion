import React from "react";

export interface ClineWatermarkProps {
  className?: string;
}

export function ClineWatermark({ className = "w-56 h-56" }: ClineWatermarkProps) {
  return (
    <div
      data-testid="cline-watermark"
      className={`pointer-events-none select-none flex items-center justify-center opacity-[0.08] ${className}`}
    >
      <img
        src="/fusion-mascot.svg"
        alt=""
        className="w-full h-full object-contain pointer-events-none select-none"
      />
    </div>
  );
}

export default ClineWatermark;
