import { describe, expect, it } from "bun:test";
import React from "react";
import { renderToString } from "react-dom/server";
import { MarkdownRenderer } from "../src/components/MarkdownRenderer";
import { MarkdownRenderer as ExportedRenderer } from "../src/components";

describe("MarkdownRenderer", () => {
  it("is exported from components index", () => {
    expect(ExportedRenderer).toBeDefined();
    expect(ExportedRenderer).toBe(MarkdownRenderer);
  });

  it("returns null for empty content", () => {
    const html = renderToString(<MarkdownRenderer content="" />);
    expect(html).toBe("");
  });

  it("renders headings with tracking-tight and text-zinc-900", () => {
    const content = `# Heading One\n## Heading Two\n### Heading Three`;
    const html = renderToString(<MarkdownRenderer content={content} />);

    expect(html).toContain("<h1");
    expect(html).toContain("text-2xl font-bold tracking-tight text-zinc-900");
    expect(html).toContain("Heading One");

    expect(html).toContain("<h2");
    expect(html).toContain("text-xl font-bold tracking-tight text-zinc-900");
    expect(html).toContain("Heading Two");

    expect(html).toContain("<h3");
    expect(html).toContain("text-lg font-bold tracking-tight text-zinc-900");
    expect(html).toContain("Heading Three");
  });

  it("renders paragraphs with text-[14px] leading-relaxed text-zinc-800", () => {
    const content = `This is a paragraph with regular text.`;
    const html = renderToString(<MarkdownRenderer content={content} />);

    expect(html).toContain("<p");
    expect(html).toContain("text-[14px] leading-relaxed text-zinc-800");
    expect(html).toContain("This is a paragraph with regular text.");
  });

  it("renders inline code as neutral gray pill with absolutely no purple", () => {
    const content = `Here is \`const x = 42;\` inline code.`;
    const html = renderToString(<MarkdownRenderer content={content} />);

    expect(html).toContain("<code");
    expect(html).toContain("bg-zinc-100 text-zinc-900 border border-zinc-200/70 rounded-md px-1.5 py-0.5 font-mono text-[12px]");
    expect(html).toContain("const x = 42;");
    expect(html.toLowerCase()).not.toContain("purple");
  });

  it("renders fenced code blocks with header bar, language label, copy button, and code body", () => {
    const content = "```typescript\nfunction greet() {\n  return 'hello';\n}\n```";
    const html = renderToString(<MarkdownRenderer content={content} />);

    // Header bar
    expect(html).toContain("typescript");
    expect(html).toContain("Copy");

    // Code body
    expect(html).toContain("bg-zinc-900 text-zinc-100 p-3 font-mono text-[12px] overflow-x-auto leading-relaxed");
    expect(html).toContain("function greet()");
  });

  it("renders bullet and numbered lists", () => {
    const content = `- Bullet A\n- Bullet B\n\n1. Step One\n2. Step Two`;
    const html = renderToString(<MarkdownRenderer content={content} />);

    expect(html).toContain("<ul");
    expect(html).toContain("list-disc list-outside ml-5");
    expect(html).toContain("Bullet A");
    expect(html).toContain("Bullet B");

    expect(html).toContain("<ol");
    expect(html).toContain("list-decimal list-outside ml-5");
    expect(html).toContain("Step One");
    expect(html).toContain("Step Two");
  });

  it("renders blockquotes with subtle left border", () => {
    const content = `> This is an important note.\n> Second line of note.`;
    const html = renderToString(<MarkdownRenderer content={content} />);

    expect(html).toContain("<blockquote");
    expect(html).toContain("border-l-2 border-zinc-300 pl-3.5 py-1 my-2.5 text-zinc-600 italic text-[14px]");
    expect(html).toContain("This is an important note.");
    expect(html).toContain("Second line of note.");
  });

  it("renders tables with light borders and alignments", () => {
    const content = `| Feature | Status |\n| :--- | ---: |\n| Agent | Ready |\n| Bridge | Active |`;
    const html = renderToString(<MarkdownRenderer content={content} />);

    expect(html).toContain("<table");
    expect(html).toContain("border-zinc-200/80");
    expect(html).toContain("Feature");
    expect(html).toContain("Status");
    expect(html).toContain("Agent");
    expect(html).toContain("Ready");
  });

  it("renders links in clean blue opening safely", () => {
    const content = `Visit [Fusion Docs](https://fusion.dev) for details.`;
    const html = renderToString(<MarkdownRenderer content={content} />);

    expect(html).toContain("<a");
    expect(html).toContain('href="https://fusion.dev"');
    expect(html).toContain("text-blue-600 hover:underline");
    expect(html).toContain('rel="noopener noreferrer"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain("Fusion Docs");
  });

  it("renders horizontal rules", () => {
    const content = `Section 1\n\n---\n\nSection 2`;
    const html = renderToString(<MarkdownRenderer content={content} />);

    expect(html).toContain("<hr");
    expect(html).toContain("border-t border-zinc-200/80");
  });
});
