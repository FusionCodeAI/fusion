import React from "react";
import { FileCode } from "lucide-react";

export interface DiffViewProps {
  patch: string;
}

interface DiffLine {
  type: "header" | "hunk" | "addition" | "deletion" | "context";
  text: string;
  oldLine?: number;
  newLine?: number;
}

interface ParsedDiff {
  fileName: string | null;
  additions: number;
  deletions: number;
  lines: DiffLine[];
}

function parsePatch(patch: string): ParsedDiff {
  if (!patch || patch.trim().length === 0) {
    return { fileName: null, additions: 0, deletions: 0, lines: [] };
  }

  const rawLines = patch.split(/\r?\n/);
  const lines: DiffLine[] = [];
  let fileName: string | null = null;
  let additions = 0;
  let deletions = 0;
  let oldLineNum = 0;
  let newLineNum = 0;

  for (const raw of rawLines) {
    if (raw.startsWith("diff --git")) {
      const match = raw.match(/diff --git a\/(.+?) b\/(.+)/);
      if (match) {
        fileName = match[2];
      }
      lines.push({ type: "header", text: raw });
    } else if (raw.startsWith("--- ")) {
      if (!fileName && !raw.startsWith("--- /dev/null")) {
        fileName = raw.replace(/^---\s+[ab]\//, "");
      }
      lines.push({ type: "header", text: raw });
    } else if (raw.startsWith("+++ ")) {
      if (!fileName && !raw.startsWith("+++ /dev/null")) {
        fileName = raw.replace(/^\+\+\+\s+[ab]\//, "");
      }
      lines.push({ type: "header", text: raw });
    } else if (raw.startsWith("index ")) {
      lines.push({ type: "header", text: raw });
    } else if (raw.startsWith("@@")) {
      const match = raw.match(/@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      if (match) {
        oldLineNum = parseInt(match[1], 10);
        newLineNum = parseInt(match[2], 10);
      }
      lines.push({ type: "hunk", text: raw });
    } else if (raw.startsWith("+")) {
      additions++;
      lines.push({
        type: "addition",
        text: raw.slice(1),
        newLine: newLineNum++,
      });
    } else if (raw.startsWith("-")) {
      deletions++;
      lines.push({
        type: "deletion",
        text: raw.slice(1),
        oldLine: oldLineNum++,
      });
    } else {
      const text = raw.startsWith(" ") ? raw.slice(1) : raw;
      lines.push({
        type: "context",
        text,
        oldLine: oldLineNum > 0 ? oldLineNum++ : undefined,
        newLine: newLineNum > 0 ? newLineNum++ : undefined,
      });
    }
  }

  return { fileName, additions, deletions, lines };
}

export function DiffView({ patch }: DiffViewProps) {
  const { fileName, additions, deletions, lines } = parsePatch(patch);

  if (lines.length === 0) {
    return null;
  }

  return (
    <div className="rounded-lg border border-zinc-200 bg-white overflow-hidden my-2 shadow-2xs font-mono text-[12px]">
      {fileName && (
        <div className="flex items-center justify-between px-3 py-1.5 bg-zinc-50 border-b border-zinc-200 text-xs font-sans">
          <div className="flex items-center gap-1.5 font-medium text-zinc-700 truncate">
            <FileCode className="w-3.5 h-3.5 text-zinc-400 shrink-0" />
            <span className="font-mono text-[11px] truncate">{fileName}</span>
          </div>
          <div className="flex items-center gap-2 text-[11px] font-mono shrink-0 ml-2">
            {additions > 0 && (
              <span className="text-emerald-600 font-medium">+{additions}</span>
            )}
            {deletions > 0 && (
              <span className="text-rose-600 font-medium">-{deletions}</span>
            )}
          </div>
        </div>
      )}

      <div className="overflow-x-auto">
        <table className="w-full border-collapse">
          <tbody>
            {lines.map((line, index) => {
              if (line.type === "header") {
                return (
                  <tr
                    key={index}
                    className="bg-zinc-100/50 text-zinc-500 font-mono text-[11px]"
                  >
                    <td
                      colSpan={4}
                      className="px-3 py-0.5 text-zinc-500 select-none whitespace-pre"
                    >
                      {line.text}
                    </td>
                  </tr>
                );
              }

              if (line.type === "hunk") {
                return (
                  <tr
                    key={index}
                    className="bg-zinc-100/80 text-zinc-500 font-mono text-[11px] border-y border-zinc-200/60"
                  >
                    <td
                      colSpan={4}
                      className="px-3 py-1 font-mono text-[11px] text-zinc-500 select-none font-medium whitespace-pre"
                    >
                      {line.text}
                    </td>
                  </tr>
                );
              }

              if (line.type === "addition") {
                return (
                  <tr
                    key={index}
                    className="bg-emerald-50/70 hover:bg-emerald-100/50 text-emerald-950 font-mono text-[12px] leading-relaxed transition-colors"
                  >
                    <td className="w-10 px-2 py-0.5 text-right select-none text-[11px] text-emerald-600/70 font-mono border-r border-emerald-100/60 bg-emerald-50/40"></td>
                    <td className="w-10 px-2 py-0.5 text-right select-none text-[11px] text-emerald-600/80 font-mono border-r border-emerald-100/60 bg-emerald-50/40">
                      {line.newLine}
                    </td>
                    <td className="w-6 text-center select-none text-emerald-600 font-semibold font-mono text-[12px]">
                      +
                    </td>
                    <td className="px-2 py-0.5 whitespace-pre font-mono text-[12px] text-emerald-950">
                      {line.text}
                    </td>
                  </tr>
                );
              }

              if (line.type === "deletion") {
                return (
                  <tr
                    key={index}
                    className="bg-rose-50/70 hover:bg-rose-100/50 text-rose-950 font-mono text-[12px] leading-relaxed transition-colors"
                  >
                    <td className="w-10 px-2 py-0.5 text-right select-none text-[11px] text-rose-600/80 font-mono border-r border-rose-100/60 bg-rose-50/40">
                      {line.oldLine}
                    </td>
                    <td className="w-10 px-2 py-0.5 text-right select-none text-[11px] text-rose-600/70 font-mono border-r border-rose-100/60 bg-rose-50/40"></td>
                    <td className="w-6 text-center select-none text-rose-600 font-semibold font-mono text-[12px]">
                      -
                    </td>
                    <td className="px-2 py-0.5 whitespace-pre font-mono text-[12px] text-rose-950">
                      {line.text}
                    </td>
                  </tr>
                );
              }

              return (
                <tr
                  key={index}
                  className="hover:bg-zinc-50/80 text-zinc-800 font-mono text-[12px] leading-relaxed transition-colors"
                >
                  <td className="w-10 px-2 py-0.5 text-right select-none text-[11px] text-zinc-400 font-mono border-r border-zinc-100">
                    {line.oldLine ?? ""}
                  </td>
                  <td className="w-10 px-2 py-0.5 text-right select-none text-[11px] text-zinc-400 font-mono border-r border-zinc-100">
                    {line.newLine ?? ""}
                  </td>
                  <td className="w-6 text-center select-none text-transparent font-mono text-[12px]">
                    &nbsp;
                  </td>
                  <td className="px-2 py-0.5 whitespace-pre font-mono text-[12px] text-zinc-800">
                    {line.text}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export default DiffView;
