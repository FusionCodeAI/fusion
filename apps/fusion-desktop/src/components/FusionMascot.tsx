import React, { useEffect, useRef, useState, useId } from "react";

const BODY_PATH =
  "M95.65 -0.13C95.65 3.03 95.65 6.16 95.65 9.35C95.65 12.54 95.65 15.73 95.63 19C95.6 22.28 95.59 25.6 95.5 29.01C95.41 32.43 95.33 35.92 95.1 39.5C94.86 43.07 94.61 46.77 94.08 50.46C93.54 54.15 92.93 57.99 91.87 61.63C90.82 65.27 89.54 69.01 87.73 72.32C85.93 75.62 83.69 78.81 81.06 81.45C78.43 84.09 75.25 86.34 71.96 88.15C68.66 89.96 64.93 91.24 61.3 92.3C57.67 93.36 53.84 93.97 50.17 94.51C46.49 95.05 42.8 95.3 39.24 95.54C35.68 95.77 32.19 95.85 28.79 95.94C25.39 96.03 22.08 96.04 18.82 96.07C15.55 96.09 12.37 96.09 9.19 96.09C6.01 96.1 2.89 96.09 -0.26 96.09C-3.41 96.09 -6.52 96.1 -9.7 96.09C-12.88 96.09 -16.06 96.09 -19.33 96.07C-22.6 96.04 -25.9 96.03 -29.3 95.94C-32.71 95.85 -36.19 95.77 -39.75 95.54C-43.32 95.3 -47 95.05 -50.68 94.51C-54.36 93.97 -58.19 93.36 -61.82 92.3C-65.45 91.24 -69.18 89.96 -72.47 88.15C-75.76 86.34 -78.94 84.09 -81.57 81.45C-84.2 78.81 -86.44 75.62 -88.24 72.32C-90.05 69.01 -91.33 65.27 -92.39 61.63C-93.44 57.99 -94.05 54.15 -94.59 50.46C-95.12 46.77 -95.37 43.07 -95.61 39.5C-95.84 35.92 -95.92 32.43 -96.01 29.01C-96.1 25.6 -96.12 22.28 -96.14 19C-96.16 15.73 -96.16 12.54 -96.16 9.35C-96.16 6.16 -96.16 3.03 -96.16 -0.13C-96.16 -3.28 -96.16 -6.41 -96.14 -9.6C-96.12 -12.79 -96.1 -16.08 -96.01 -19.49C-95.92 -22.91 -95.84 -26.4 -95.61 -29.98C-95.37 -33.55 -95.12 -37.25 -94.59 -40.94C-94.05 -44.63 -93.44 -48.47 -92.39 -52.11C-91.33 -55.75 -90.05 -59.49 -88.24 -62.8C-86.44 -66.1 -84.2 -69.29 -81.57 -71.93C-78.94 -74.57 -75.76 -76.82 -72.47 -78.63C-69.18 -80.44 -65.45 -81.72 -61.82 -82.78C-58.19 -83.84 -54.36 -84.45 -50.68 -84.99C-47 -85.53 -43.32 -85.78 -39.75 -86.02C-36.19 -86.25 -32.71 -86.33 -29.3 -86.42C-25.9 -86.51 -22.6 -86.52 -19.33 -86.55C-16.06 -86.57 -12.88 -86.57 -9.7 -86.57C-6.52 -86.58 -3.41 -86.57 -0.26 -86.57C2.89 -86.57 6.01 -86.58 9.19 -86.57C12.37 -86.57 15.55 -86.57 18.82 -86.55C22.08 -86.52 25.39 -86.51 28.79 -86.42C32.19 -86.33 35.68 -86.25 39.24 -86.02C42.8 -85.78 46.49 -85.53 50.17 -84.99C53.84 -84.45 57.67 -83.84 61.3 -82.78C64.93 -81.72 68.66 -80.44 71.96 -78.63C75.25 -76.82 78.43 -74.57 81.06 -71.93C83.69 -69.29 85.93 -66.1 87.73 -62.8C89.54 -59.49 90.82 -55.75 91.87 -52.11C92.93 -48.47 93.54 -44.63 94.08 -40.94C94.61 -37.25 94.86 -33.55 95.1 -29.98C95.33 -26.4 95.41 -22.91 95.5 -19.49C95.59 -16.08 95.6 -12.79 95.63 -9.6C95.65 -6.41 95.65 -3.28 95.65 -0.13Z";

const EYE_D =
  "M-10.5 -11.5A10.5 10.5 0 0 1 0 -22L0 -22A10.5 10.5 0 0 1 10.5 -11.5L10.5 11.5A10.5 10.5 0 0 1 0 22L0 22A10.5 10.5 0 0 1 -10.5 11.5Z";

export interface FusionMascotProps {
  className?: string;
  size?: number | string;
  color?: string;
  animated?: boolean;
  interactiveGaze?: boolean;
  floating?: boolean;
  title?: string;
}

/**
 * Interactive Fusion Squircle Mascot Avatar.
 * Features:
 * - Real-time gaze tracking following the mouse cursor across the screen.
 * - Periodic natural eye blinking.
 * - Subtle organic floating/breathing animation.
 * - SVG mask rendering in crisp Fusion Brand Violet (#5100cd).
 */
export function FusionMascot({
  className = "w-20 h-20",
  size,
  color = "#5100cd",
  animated = true,
  interactiveGaze = true,
  floating = true,
  title = "Fusion Assistant",
}: FusionMascotProps) {
  const maskId = useId();
  const containerRef = useRef<HTMLDivElement>(null);

  // Eye gaze offset: current smooth value and target value
  const [gaze, setGaze] = useState({ x: 0, y: 0 });
  const [isBlinking, setIsBlinking] = useState(false);

  const targetGaze = useRef({ x: 0, y: 0 });
  const currentGaze = useRef({ x: 0, y: 0 });
  const animFrame = useRef<number | null>(null);

  // Mouse / Pointer Gaze Tracking
  useEffect(() => {
    if (!animated || !interactiveGaze) return;

    const onPointerMove = (e: PointerEvent) => {
      if (!containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      const centerX = rect.left + rect.width / 2;
      const centerY = rect.top + rect.height / 2;

      // Normalized coordinates (-1 to 1) relative to center of screen
      const rawNx = (e.clientX - centerX) / (window.innerWidth / 2 || 1);
      const rawNy = (e.clientY - centerY) / (window.innerHeight / 2 || 1);

      const nx = Math.max(-1, Math.min(1, rawNx));
      const ny = Math.max(-1, Math.min(1, rawNy));

      // Maximum travel in SVG coordinate space: ±10px horizontally, ±7px vertically
      targetGaze.current = {
        x: nx * 10,
        y: ny * 7,
      };
    };

    window.addEventListener("pointermove", onPointerMove, { passive: true });

    // Smooth lerp loop (exponential ease-out for natural eye movement)
    const updateGaze = () => {
      const speed = 0.14; // Snappy yet smooth
      const dx = targetGaze.current.x - currentGaze.current.x;
      const dy = targetGaze.current.y - currentGaze.current.y;

      if (Math.abs(dx) > 0.01 || Math.abs(dy) > 0.01) {
        currentGaze.current.x += dx * speed;
        currentGaze.current.y += dy * speed;
        setGaze({
          x: Math.round(currentGaze.current.x * 100) / 100,
          y: Math.round(currentGaze.current.y * 100) / 100,
        });
      }

      animFrame.current = requestAnimationFrame(updateGaze);
    };

    animFrame.current = requestAnimationFrame(updateGaze);

    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      if (animFrame.current) {
        cancelAnimationFrame(animFrame.current);
      }
    };
  }, [animated, interactiveGaze]);

  // Periodic Natural Blinking
  useEffect(() => {
    if (!animated) return;

    let blinkTimeout: number | NodeJS.Timeout | undefined;

    const scheduleBlink = () => {
      // Random interval between 3.2s and 5.8s
      const delay = 3200 + Math.random() * 2600;
      blinkTimeout = setTimeout(() => {
        setIsBlinking(true);
        // Fast blink duration: 150ms
        setTimeout(() => {
          setIsBlinking(false);
          scheduleBlink();
        }, 150);
      }, delay);
    };

    scheduleBlink();

    return () => {
      clearTimeout(blinkTimeout);
    };
  }, [animated]);

  return (
    <div
      ref={containerRef}
      data-testid="fusion-mascot"
      className={`relative inline-flex items-center justify-center shrink-0 ${className} ${
        floating && animated ? "animate-[fusion-float_3.6s_ease-in-out_infinite]" : ""
      }`}
      style={size ? { width: size, height: size } : undefined}
      title={title}
    >
      <svg
        viewBox="-125 -125 250 250"
        role="img"
        aria-label={title}
        className="w-full h-full select-none pointer-events-none drop-shadow-md transition-transform duration-75"
        style={{
          transform: `rotate(${gaze.x * 0.45}deg)`,
        }}
        xmlns="http://www.w3.org/2000/svg"
      >
        <defs>
          <mask
            id={maskId}
            maskUnits="userSpaceOnUse"
            x="-158"
            y="-158"
            width="316"
            height="316"
          >
            {/* White Body Cutout */}
            <path d={BODY_PATH} fill="#ffffff" />

            {/* Black Eyes: Dynamic Gaze Translation + Blink Scaling */}
            <g transform={`translate(${gaze.x}, ${gaze.y})`}>
              <g
                style={{
                  transformOrigin: "0px -10px",
                  transform: isBlinking ? "scale(1, 0.08)" : "scale(1, 1)",
                  transition: "transform 80ms ease-out",
                }}
              >
                {/* Left Eye */}
                <path
                  transform="matrix(0.97,-0.08,0.06,0.99,-25.6,-7.51)"
                  d={EYE_D}
                  fill="#000000"
                />
                {/* Right Eye */}
                <path
                  transform="matrix(0.95,-0.03,0.06,0.99,29.88,-11.08)"
                  d={EYE_D}
                  fill="#000000"
                />
              </g>
            </g>
          </mask>
        </defs>

        {/* Main Colored Body Shape */}
        <g opacity="1">
          <path d={BODY_PATH} fill="none" />
          <g mask={`url(#${maskId})`}>
            <rect x="-158" y="-158" width="316" height="316" fill={color} />
          </g>
        </g>
      </svg>
    </div>
  );
}

export default FusionMascot;
