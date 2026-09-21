import React, { useState } from "react";
import { Sparkles, Terminal, Cpu, ShieldCheck } from "lucide-react";

export function App() {
  const [active, setActive] = useState(false);

  return (
    <div className="flex flex-col h-screen w-screen bg-[#18181b] text-zinc-100 font-sans select-none">
      {/* Title bar clearance / drag region */}
      <header
        className="h-10 border-b border-zinc-800/80 flex items-center px-4 justify-between bg-[#121214]/60 backdrop-blur-md"
        data-tauri-drag-region
      >
        <div className="flex items-center gap-2 pl-16">
          <Sparkles className="w-4 h-4 text-emerald-400" />
          <span className="text-xs font-medium text-zinc-300">Fusion Agent</span>
        </div>
        <div className="flex items-center gap-2 text-xs text-zinc-500">
          <span className="inline-block w-2 h-2 rounded-full bg-emerald-500" />
          <span>Ready</span>
        </div>
      </header>

      {/* Main Content */}
      <main className="flex-1 flex flex-col items-center justify-center p-8">
        <div className="max-w-md w-full p-6 rounded-2xl bg-zinc-900/70 border border-zinc-800 shadow-2xl backdrop-blur-xl flex flex-col items-center text-center space-y-4">
          <div className="w-12 h-12 rounded-xl bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center text-emerald-400">
            <Terminal className="w-6 h-6" />
          </div>

          <div>
            <h1 className="text-lg font-semibold text-zinc-100">Fusion Desktop Workspace</h1>
            <p className="text-xs text-zinc-400 mt-1">
              Tauri v2 + React 19 + Tailwind v4 + Vite Control Plane
            </p>
          </div>

          <div className="grid grid-cols-2 gap-2 w-full pt-2">
            <div className="p-3 rounded-lg bg-zinc-800/40 border border-zinc-700/40 flex items-center gap-2.5 text-left">
              <Cpu className="w-4 h-4 text-zinc-400 shrink-0" />
              <div className="truncate">
                <div className="text-[11px] font-medium text-zinc-300">Engine</div>
                <div className="text-[10px] text-zinc-500">Fusion Sidecar</div>
              </div>
            </div>
            <div className="p-3 rounded-lg bg-zinc-800/40 border border-zinc-700/40 flex items-center gap-2.5 text-left">
              <ShieldCheck className="w-4 h-4 text-emerald-400 shrink-0" />
              <div className="truncate">
                <div className="text-[11px] font-medium text-zinc-300">Protocol</div>
                <div className="text-[10px] text-zinc-500">ACP v2.0</div>
              </div>
            </div>
          </div>

          <button
            type="button"
            onClick={() => setActive((prev) => !prev)}
            className={`w-full py-2 px-4 rounded-lg text-xs font-medium transition-all ${
              active
                ? "bg-emerald-600 text-white shadow-lg shadow-emerald-600/20"
                : "bg-zinc-800 hover:bg-zinc-700 text-zinc-200 border border-zinc-700"
            }`}
          >
            {active ? "Workspace Initialized" : "Test Interaction"}
          </button>
        </div>
      </main>
    </div>
  );
}
