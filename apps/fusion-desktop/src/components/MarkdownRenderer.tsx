import React, { useState, useTransition } from "react";
import { Check, Copy } from "lucide-react";
import { open as openExternal } from "@tauri-apps/plugin-shell";

export interface MarkdownRendererProps {
  content: string;
  className?: string;
}

// ----------------------------------------------------------------------
// Safe external link opener
// ----------------------------------------------------------------------
async function openExternalUrl(url: string) {
  if (!url) return;
  if (url.startsWith("#")) {
    const el = document.getElementById(url.slice(1));
    el?.scrollIntoView({ behavior: "smooth" });
    return;
  }
  try {
    await openExternal(url);
  } catch {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}

// ----------------------------------------------------------------------
// Code block with copy button and language badge
// ----------------------------------------------------------------------
interface CodeBlockProps {
  language: string;
  code: string;
}

function CodeBlock({ language, code }: CodeBlockProps) {
  const [copied, setCopied] = useState(false);
  const [, startTransition] = useTransition();

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      startTransition(() => {
        setCopied(true);
      });
      setTimeout(() => {
        setCopied(false);
      }, 2000);
    } catch {
      try {
        const textarea = document.createElement("textarea");
        textarea.value = code;
        textarea.style.position = "fixed";
        textarea.style.opacity = "0";
        document.body.appendChild(textarea);
        textarea.select();
        document.execCommand("copy");
        document.body.removeChild(textarea);
        setCopied(true);
        setTimeout(() => {
          setCopied(false);
        }, 2000);
      } catch {
        // Fallback failed silently
      }
    }
  };

  const displayLang = (language || "text").trim().toLowerCase();

  return (
    <div className="my-3 rounded-xl border border-zinc-800 bg-zinc-900 overflow-hidden shadow-xs">
      <div className="flex items-center justify-between px-3.5 py-1.5 bg-zinc-800/90 border-b border-zinc-800 text-[11px] font-mono text-zinc-400 select-none">
        <span className="font-medium text-zinc-300">{displayLang}</span>
        <button
          type="button"
          onClick={handleCopy}
          aria-label={copied ? "Copied" : "Copy code"}
          className="flex items-center gap-1.5 px-2 py-0.5 rounded text-zinc-400 hover:text-zinc-200 hover:bg-zinc-700/60 transition-colors cursor-pointer text-[11px] font-sans"
        >
          {copied ? (
            <>
              <Check className="w-3.5 h-3.5 text-emerald-400" />
              <span className="text-emerald-400 font-medium">Copied!</span>
            </>
          ) : (
            <>
              <Copy className="w-3.5 h-3.5" />
              <span>Copy</span>
            </>
          )}
        </button>
      </div>
      <pre className="bg-zinc-900 text-zinc-100 p-3 font-mono text-[12px] overflow-x-auto leading-relaxed">
        <code>{code}</code>
      </pre>
    </div>
  );
}

// ----------------------------------------------------------------------
// Inline parsing and rendering
// ----------------------------------------------------------------------
type InlineNode =
  | { type: "text"; text: string }
  | { type: "code"; code: string }
  | { type: "link"; text: string; href: string }
  | { type: "image"; alt: string; src: string }
  | { type: "boldItalic"; text: string }
  | { type: "bold"; text: string }
  | { type: "italic"; text: string }
  | { type: "strikethrough"; text: string }
  | { type: "br" };

function parseInline(text: string): InlineNode[] {
  const nodes: InlineNode[] = [];
  let remaining = text;

  while (remaining.length > 0) {
    if (remaining.startsWith("\n")) {
      nodes.push({ type: "br" });
      remaining = remaining.slice(1);
      continue;
    }

    // Inline code: `...` or ``...``
    const codeMatch = remaining.match(/^(`+)([\s\S]+?)\1/);
    if (codeMatch) {
      nodes.push({ type: "code", code: codeMatch[2] });
      remaining = remaining.slice(codeMatch[0].length);
      continue;
    }

    // Image: ![alt](url)
    const imgMatch = remaining.match(/^!\[([^\]]*)\]\(([^)]+)\)/);
    if (imgMatch) {
      nodes.push({ type: "image", alt: imgMatch[1], src: imgMatch[2] });
      remaining = remaining.slice(imgMatch[0].length);
      continue;
    }

    // Link: [text](url)
    const linkMatch = remaining.match(/^\[([^\]]+)\]\(([^)]+)\)/);
    if (linkMatch) {
      nodes.push({
        type: "link",
        text: linkMatch[1],
        href: linkMatch[2],
      });
      remaining = remaining.slice(linkMatch[0].length);
      continue;
    }

    // Bold italic: ***...*** or ___...___
    const boldItalicMatch = remaining.match(/^(\*{3}|_{3})(?!\s)(.+?)(?<!\s|\*|_)\1(?!\*|_)/);
    if (boldItalicMatch) {
      nodes.push({
        type: "boldItalic",
        text: boldItalicMatch[2],
      });
      remaining = remaining.slice(boldItalicMatch[0].length);
      continue;
    }

    // Bold: **...** or __...__
    const boldMatch = remaining.match(/^(\*{2}|_{2})(?!\s)(.+?)(?<!\s|\*|_)\1(?!\*|_)/);
    if (boldMatch) {
      nodes.push({
        type: "bold",
        text: boldMatch[2],
      });
      remaining = remaining.slice(boldMatch[0].length);
      continue;
    }

    // Italic: *...* or _..._
    const italicMatch = remaining.match(/^(\*|_)(?!\s)(.+?)(?<!\s|\*|_)\1(?!\*|_)/);
    if (italicMatch) {
      nodes.push({
        type: "italic",
        text: italicMatch[2],
      });
      remaining = remaining.slice(italicMatch[0].length);
      continue;
    }

    // Strikethrough: ~~...~~
    const strikeMatch = remaining.match(/^~~(?!\s)(.+?)(?<!\s)~~/);
    if (strikeMatch) {
      nodes.push({
        type: "strikethrough",
        text: strikeMatch[1],
      });
      remaining = remaining.slice(strikeMatch[0].length);
      continue;
    }

    // Auto-link: https://... or http://...
    const urlMatch = remaining.match(/^(https?:\/\/[^\s<]+[^<.,:;"\x27)\]\s!?])/);
    if (urlMatch) {
      nodes.push({
        type: "link",
        text: urlMatch[1],
        href: urlMatch[1],
      });
      remaining = remaining.slice(urlMatch[0].length);
      continue;
    }

    // Plain text until next special character
    const nextSpecial = remaining.search(/[`!\[*_~\n]|https?:\/\//);
    if (nextSpecial === -1) {
      nodes.push({ type: "text", text: remaining });
      break;
    } else if (nextSpecial === 0) {
      nodes.push({ type: "text", text: remaining[0] });
      remaining = remaining.slice(1);
    } else {
      nodes.push({ type: "text", text: remaining.slice(0, nextSpecial) });
      remaining = remaining.slice(nextSpecial);
    }
  }

  return nodes;
}

function renderInline(text: string): React.ReactNode {
  const nodes = parseInline(text);
  return nodes.map((node, index) => {
    switch (node.type) {
      case "text":
        return <React.Fragment key={index}>{node.text}</React.Fragment>;
      case "br":
        return <br key={index} />;
      case "code":
        return (
          <code
            key={index}
            className="bg-zinc-100 text-zinc-900 border border-zinc-200/70 rounded-md px-1.5 py-0.5 font-mono text-[12px]"
          >
            {node.code}
          </code>
        );
      case "link":
        return (
          <a
            key={index}
            href={node.href}
            onClick={(e) => {
              e.preventDefault();
              void openExternalUrl(node.href);
            }}
            target="_blank"
            rel="noopener noreferrer"
            className="text-blue-600 hover:underline font-medium cursor-pointer"
          >
            {renderInline(node.text)}
          </a>
        );
      case "image":
        return (
          <img
            key={index}
            src={node.src}
            alt={node.alt}
            className="max-w-full h-auto rounded-lg my-2 border border-zinc-200/70 inline-block"
          />
        );
      case "boldItalic":
        return (
          <strong key={index} className="font-bold italic text-zinc-900">
            {renderInline(node.text)}
          </strong>
        );
      case "bold":
        return (
          <strong key={index} className="font-bold text-zinc-900">
            {renderInline(node.text)}
          </strong>
        );
      case "italic":
        return (
          <em key={index} className="italic text-zinc-800">
            {renderInline(node.text)}
          </em>
        );
      case "strikethrough":
        return (
          <del key={index} className="line-through text-zinc-500">
            {renderInline(node.text)}
          </del>
        );
      default:
        return null;
    }
  });
}

// ----------------------------------------------------------------------
// Block parsing and rendering
// ----------------------------------------------------------------------
interface ListItemNode {
  text: string;
  ordered: boolean;
  checked?: boolean;
  children: ListItemNode[];
}

type Block =
  | { type: "heading"; level: 1 | 2 | 3 | 4 | 5 | 6; text: string }
  | { type: "code"; language: string; code: string }
  | { type: "paragraph"; text: string }
  | { type: "list"; ordered: boolean; items: ListItemNode[] }
  | { type: "blockquote"; text: string }
  | {
      type: "table";
      headers: string[];
      alignments: ("left" | "center" | "right")[];
      rows: string[][];
    }
  | { type: "hr" };

function parseBlocks(markdown: string): Block[] {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // 1. Fenced code block
    const codeFenceMatch = line.match(/^```([a-zA-Z0-9_\-\+]*)\s*$/);
    if (codeFenceMatch) {
      const language = codeFenceMatch[1] || "";
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].match(/^```\s*$/)) {
        codeLines.push(lines[i]);
        i++;
      }
      if (i < lines.length) {
        i++; // skip closing ```
      }
      blocks.push({ type: "code", language, code: codeLines.join("\n") });
      continue;
    }

    // 2. Empty line
    if (line.trim() === "") {
      i++;
      continue;
    }

    // 3. Horizontal rule (---, ***, ___)
    if (/^(\*{3,}|-{3,}|_{3,})\s*$/.test(line)) {
      blocks.push({ type: "hr" });
      i++;
      continue;
    }

    // 4. Heading (# to ######)
    const headingMatch = line.match(/^(#{1,6})\s+(.+)$/);
    if (headingMatch) {
      blocks.push({
        type: "heading",
        level: headingMatch[1].length as 1 | 2 | 3 | 4 | 5 | 6,
        text: headingMatch[2].trim(),
      });
      i++;
      continue;
    }

    // 5. Blockquote (> ...)
    if (line.match(/^>\s?/)) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].match(/^>\s?/)) {
        quoteLines.push(lines[i].replace(/^>\s?/, ""));
        i++;
      }
      blocks.push({
        type: "blockquote",
        text: quoteLines.join("\n"),
      });
      continue;
    }

    // 6. Table detection
    const nextLine = i + 1 < lines.length ? lines[i + 1].trim() : "";
    const nextCells = nextLine.includes("-")
      ? nextLine.replace(/^\|/, "").replace(/\|$/, "").split("|")
      : [];
    const isTableDelim =
      nextCells.length > 0 && nextCells.every((c) => /^\s*:?-+:?\s*$/.test(c));

    if (line.includes("|") && isTableDelim) {
      const parseCells = (row: string) => {
        let trimmed = row.trim();
        if (trimmed.startsWith("|")) trimmed = trimmed.slice(1);
        if (trimmed.endsWith("|")) trimmed = trimmed.slice(0, -1);
        return trimmed.split("|").map((c) => c.trim());
      };

      const headers = parseCells(line);
      const delims = parseCells(lines[i + 1]);
      const alignments = delims.map<"left" | "center" | "right">((d) => {
        const left = d.startsWith(":");
        const right = d.endsWith(":");
        if (left && right) return "center";
        if (right) return "right";
        return "left";
      });

      i += 2;
      const rows: string[][] = [];
      while (
        i < lines.length &&
        lines[i].includes("|") &&
        lines[i].trim() !== ""
      ) {
        rows.push(parseCells(lines[i]));
        i++;
      }
      blocks.push({ type: "table", headers, alignments, rows });
      continue;
    }

    // 7. List items (- , * , + or 1. )
    const listMatch = line.match(/^(\s*)([-*+]|\d+\.)\s+(.*)$/);
    if (listMatch) {
      const baseIndent = listMatch[1].replace(/\t/g, "    ").length;
      const baseOrdered = /^\d+\./.test(listMatch[2]);
      const rawList: Array<{
        indent: number;
        ordered: boolean;
        text: string;
      }> = [];

      while (i < lines.length) {
        const match = lines[i].match(/^(\s*)([-*+]|\d+\.)\s+(.*)$/);
        if (!match) break;
        const indent = match[1].replace(/\t/g, "    ").length;
        const isOrdered = /^\d+\./.test(match[2]);
        if (indent === baseIndent && isOrdered !== baseOrdered) {
          break;
        }
        rawList.push({
          indent,
          ordered: isOrdered,
          text: match[3],
        });
        i++;
      }

      const buildTree = (items: typeof rawList): ListItemNode[] => {
        const result: ListItemNode[] = [];
        let idx = 0;
        while (idx < items.length) {
          const item = items[idx];
          const childrenRaw: typeof rawList = [];
          let nextIdx = idx + 1;
          while (
            nextIdx < items.length &&
            items[nextIdx].indent > item.indent
          ) {
            childrenRaw.push(items[nextIdx]);
            nextIdx++;
          }

          // Check for task checkbox [ ] or [x]
          let text = item.text;
          let checked: boolean | undefined;
          const taskMatch = text.match(/^\[([ xX])\]\s+(.*)$/);
          if (taskMatch) {
            checked = taskMatch[1].toLowerCase() === "x";
            text = taskMatch[2];
          }

          result.push({
            text,
            ordered: item.ordered,
            checked,
            children: childrenRaw.length > 0 ? buildTree(childrenRaw) : [],
          });
          idx = nextIdx;
        }
        return result;
      };

      blocks.push({
        type: "list",
        ordered: baseOrdered,
        items: buildTree(rawList),
      });
      continue;
    }

    // 8. Paragraph
    const paragraphLines: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].match(/^```/) &&
      !lines[i].match(/^(#{1,6})\s+/) &&
      !lines[i].match(/^>\s?/) &&
      !/^(\*{3,}|-{3,}|_{3,})\s*$/.test(lines[i]) &&
      !/^\s*[-*+]\s+/.test(lines[i]) &&
      !/^\s*\d+\.\s+/.test(lines[i]) &&
      !(
        lines[i].includes("|") &&
        i + 1 < lines.length &&
        lines[i + 1].includes("-") &&
        /^\s*:?-+:?\s*$/.test(
          lines[i + 1].replace(/^\|/, "").replace(/\|$/, "").split("|")[0] || ""
        )
      )
    ) {
      paragraphLines.push(lines[i]);
      i++;
    }

    if (paragraphLines.length > 0) {
      blocks.push({
        type: "paragraph",
        text: paragraphLines.join("\n"),
      });
    }
  }

  return blocks;
}

// ----------------------------------------------------------------------
// Rendering lists recursively
// ----------------------------------------------------------------------
function renderListItems(items: ListItemNode[], ordered: boolean, keyPrefix: string): React.ReactNode {
  const ListTag = ordered ? "ol" : "ul";
  const listClass = ordered
    ? "list-decimal list-outside ml-5 my-2 space-y-1 text-[14px] leading-relaxed text-zinc-800"
    : "list-disc list-outside ml-5 my-2 space-y-1 text-[14px] leading-relaxed text-zinc-800";

  return (
    <ListTag className={listClass}>
      {items.map((item, idx) => {
        const itemKey = `${keyPrefix}-${idx}`;
        if (item.checked !== undefined) {
          return (
            <li key={itemKey} className="list-none flex items-start gap-2 -ml-5">
              <input
                type="checkbox"
                checked={item.checked}
                readOnly
                className="mt-1 rounded border-zinc-300 accent-zinc-800 cursor-default"
              />
              <div className="flex-1">
                {renderInline(item.text)}
                {item.children.length > 0 &&
                  renderListItems(
                    item.children,
                    item.children[0].ordered,
                    `${itemKey}-sub`
                  )}
              </div>
            </li>
          );
        }

        return (
          <li key={itemKey} className="pl-1">
            {renderInline(item.text)}
            {item.children.length > 0 &&
              renderListItems(
                item.children,
                item.children[0].ordered,
                `${itemKey}-sub`
              )}
          </li>
        );
      })}
    </ListTag>
  );
}

// ----------------------------------------------------------------------
// Main MarkdownRenderer Component
// ----------------------------------------------------------------------
export function MarkdownRenderer({ content, className = "" }: MarkdownRendererProps) {
  if (!content) {
    return null;
  }

  const blocks = parseBlocks(content);

  return (
    <div className={`markdown-body text-zinc-900 ${className}`}>
      {blocks.map((block, idx) => {
        switch (block.type) {
          case "heading": {
            switch (block.level) {
              case 1:
                return (
                  <h1
                    key={idx}
                    className="text-2xl font-bold tracking-tight text-zinc-900 mt-6 mb-3 first:mt-0"
                  >
                    {renderInline(block.text)}
                  </h1>
                );
              case 2:
                return (
                  <h2
                    key={idx}
                    className="text-xl font-bold tracking-tight text-zinc-900 mt-5 mb-2.5 first:mt-0"
                  >
                    {renderInline(block.text)}
                  </h2>
                );
              case 3:
                return (
                  <h3
                    key={idx}
                    className="text-lg font-bold tracking-tight text-zinc-900 mt-4 mb-2 first:mt-0"
                  >
                    {renderInline(block.text)}
                  </h3>
                );
              case 4:
                return (
                  <h4
                    key={idx}
                    className="text-base font-bold tracking-tight text-zinc-900 mt-3 mb-1.5 first:mt-0"
                  >
                    {renderInline(block.text)}
                  </h4>
                );
              case 5:
                return (
                  <h5
                    key={idx}
                    className="text-sm font-bold tracking-tight text-zinc-900 mt-2.5 mb-1 first:mt-0"
                  >
                    {renderInline(block.text)}
                  </h5>
                );
              case 6:
                return (
                  <h6
                    key={idx}
                    className="text-xs font-bold tracking-tight text-zinc-900 mt-2 mb-1 first:mt-0"
                  >
                    {renderInline(block.text)}
                  </h6>
                );
            }
            break;
          }

          case "paragraph":
            return (
              <p
                key={idx}
                className="my-2.5 first:mt-0 last:mb-0 text-[14px] leading-relaxed text-zinc-800"
              >
                {renderInline(block.text)}
              </p>
            );

          case "code":
            return (
              <CodeBlock
                key={idx}
                language={block.language}
                code={block.code}
              />
            );

          case "list":
            return (
              <div key={idx} className="my-2">
                {renderListItems(block.items, block.ordered, `list-${idx}`)}
              </div>
            );

          case "blockquote":
            return (
              <blockquote
                key={idx}
                className="border-l-2 border-zinc-300 pl-3.5 py-1 my-2.5 text-zinc-600 italic text-[14px] bg-zinc-50/40 rounded-r-md"
              >
                {block.text.split("\n").map((line, lIdx) => (
                  <p key={lIdx} className="my-1 first:mt-0 last:mb-0">
                    {renderInline(line)}
                  </p>
                ))}
              </blockquote>
            );

          case "table":
            return (
              <div
                key={idx}
                className="my-3 overflow-x-auto rounded-lg border border-zinc-200/80 shadow-2xs"
              >
                <table className="w-full text-left text-[13px] border-collapse">
                  <thead className="bg-zinc-50 border-b border-zinc-200/80 text-zinc-700 font-semibold">
                    <tr>
                      {block.headers.map((header, hIdx) => (
                        <th
                          key={hIdx}
                          className="px-3 py-2 border-r last:border-r-0 border-zinc-200/60"
                          style={{
                            textAlign: block.alignments[hIdx] || "left",
                          }}
                        >
                          {renderInline(header)}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-zinc-200/60 bg-white">
                    {block.rows.map((row, rIdx) => (
                      <tr
                        key={rIdx}
                        className="hover:bg-zinc-50/60 transition-colors"
                      >
                        {row.map((cell, cIdx) => (
                          <td
                            key={cIdx}
                            className="px-3 py-2 border-r last:border-r-0 border-zinc-200/60 text-zinc-800"
                            style={{
                              textAlign: block.alignments[cIdx] || "left",
                            }}
                          >
                            {renderInline(cell)}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            );

          case "hr":
            return (
              <hr key={idx} className="my-4 border-t border-zinc-200/80" />
            );

          default:
            return null;
        }
      })}
    </div>
  );
}

export default MarkdownRenderer;
