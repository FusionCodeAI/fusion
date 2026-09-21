import React from "react";

export interface ClineRobotSvgProps {
  className?: string;
  color?: string;
  strokeWidth?: number;
}

export function ClineRobotSvg({
  className = "w-6 h-6",
  color = "currentColor",
  strokeWidth = 6,
}: ClineRobotSvgProps) {
  return (
    <svg
      viewBox="0 0 100 100"
      className={className}
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      data-testid="cline-robot-svg"
    >
      {/* Top Antenna / Ring Handle */}
      <circle
        cx="50"
        cy="18"
        r="8"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
      />

      {/* Main Scalloped Robot Head Geometry */}
      <path
        d="M34 28 C34 26 38 26 42 26 L58 26 C62 26 66 26 66 28 C72 28 76 32 76 38 C82 40 86 46 86 52 C86 58 82 64 76 66 C76 72 72 76 66 76 L34 76 C28 76 24 72 24 66 C18 64 14 58 14 52 C14 46 18 40 24 38 C24 32 28 28 34 28 Z"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinejoin="round"
        strokeLinecap="round"
      />

      {/* Left Vertical Pill Eye */}
      <line
        x1="41"
        y1="44"
        x2="41"
        y2="58"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
      />

      {/* Right Vertical Pill Eye */}
      <line
        x1="59"
        y1="44"
        x2="59"
        y2="58"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
      />
    </svg>
  );
}

export interface ClineAvatarProps {
  size?: number;
  className?: string;
  isThinking?: boolean;
}

export function ClineAvatar({
  className = "w-6 h-6",
  isThinking = false,
}: ClineAvatarProps) {
  return (
    <div
      data-testid="cline-avatar"
      className={`relative rounded-full flex items-center justify-center transition-all ${
        isThinking
          ? "animate-pulse ring-2 ring-purple-300/80 bg-purple-50 shadow-xs"
          : "bg-zinc-100 hover:bg-zinc-200/70"
      } ${className}`}
    >
      <img
        src={isThinking ? "/fusion-mascot-animated.svg" : "/fusion-mascot.svg"}
        alt="Avatar"
        className="w-[85%] h-[85%] object-contain pointer-events-none select-none"
      />
    </div>
  );
}

export default ClineAvatar;
