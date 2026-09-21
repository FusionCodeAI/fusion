import React from "react";

export interface FusionLogoProps {
  className?: string;
  fill?: string;
  size?: number;
}

export function FusionLogo({ className = "w-4 h-4", fill = "#5100cd", size }: FusionLogoProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 1024 1024"
      width={size}
      height={size}
      className={className}
      fill="none"
      data-testid="fusion-logo"
    >
      {/* Outer bracket ┌ */}
      <path d="M110 110 H864 V210 H210 V864 H110 Z" fill={fill} />
      {/* Inner bracket ┌ */}
      <path d="M382 382 H864 V482 H482 V864 H382 Z" fill={fill} />
    </svg>
  );
}

export default FusionLogo;
