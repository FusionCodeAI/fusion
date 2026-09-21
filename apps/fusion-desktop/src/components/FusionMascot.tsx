import React from "react";

export interface FusionMascotProps {
  className?: string;
  size?: number | string;
  animated?: boolean;
  title?: string;
}

/**
 * Fusion Squircle Mascot Avatar.
 * Powered by the animated SVG from bloub (customized in Fusion Brand Violet #5100cd with attentive expression).
 */
export function FusionMascot({
  className = "w-7 h-7",
  size,
  animated = true,
  title = "Fusion Assistant",
}: FusionMascotProps) {
  const src = animated ? "/fusion-mascot-animated.svg" : "/fusion-mascot.svg";

  return (
    <div
      data-testid="fusion-mascot"
      className={`relative inline-flex items-center justify-center shrink-0 ${className}`}
      style={size ? { width: size, height: size } : undefined}
      title={title}
    >
      <img
        src={src}
        alt={title}
        className="w-full h-full object-contain pointer-events-none select-none drop-shadow-xs"
        loading="eager"
      />
    </div>
  );
}

export default FusionMascot;
