import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Composer } from "../src/components/Composer";
import { SlashCommandMenu } from "../src/components/SlashCommandMenu";
import { AtMentionMenu } from "../src/components/AtMentionMenu";
import { UserMessage } from "../src/components/UserMessage";
import { DEFAULT_SKILLS, filterSkills } from "../src/lib/skills-catalog";
import { DEFAULT_CONTEXT_MENTIONS, filterMentions } from "../src/lib/mentions-catalog";

describe("Skills & Slash Commands Catalog", () => {
  test("contains standard slash commands and domain skills", () => {
    const skillTriggers = DEFAULT_SKILLS.map((s) => s.trigger);
    expect(skillTriggers).toContain("/skills");
    expect(skillTriggers).toContain("/plan");
    expect(skillTriggers).toContain("/act");
    expect(skillTriggers).toContain("/clear");
    expect(skillTriggers).toContain("/mcp");
    expect(skillTriggers).toContain("/help");
    expect(skillTriggers).toContain("/skill:systematic-debugging");
    expect(skillTriggers).toContain("/skill:brainstorming");
    expect(skillTriggers).toContain("/skill:test-driven-development");
    expect(skillTriggers).toContain("/skill:design-taste-frontend");
  });

  test("filters skills cleanly by prefix and query", () => {
    const matchingSkills = filterSkills("debug");
    expect(matchingSkills.length).toBeGreaterThan(0);
    expect(matchingSkills.some((s) => s.name.includes("debugging"))).toBe(true);

    const matchingPlan = filterSkills("/pl");
    expect(matchingPlan.some((s) => s.trigger === "/plan")).toBe(true);

    const matchingAll = filterSkills("/");
    expect(matchingAll.length).toBe(DEFAULT_SKILLS.length);
  });

  test("SlashCommandMenu renders all items and categories", () => {
    const html = renderToStaticMarkup(
      <SlashCommandMenu
        skills={DEFAULT_SKILLS.slice(0, 5)}
        selectedIndex={0}
        onSelect={() => {}}
        onClose={() => {}}
      />
    );
    expect(html).toContain("data-testid=\"slash-command-menu\"");
    expect(html).toContain("/skills");
    expect(html).toContain("/plan");
    expect(html).toContain("/act");
  });
});

describe("@ Context & Mentions Catalog", () => {
  test("contains core context triggers", () => {
    const mentionLabels = DEFAULT_CONTEXT_MENTIONS.map((m) => m.label);
    expect(mentionLabels).toContain("@file");
    expect(mentionLabels).toContain("@folder");
    expect(mentionLabels).toContain("@git-diff");
    expect(mentionLabels).toContain("@commits");
    expect(mentionLabels).toContain("@problems");
    expect(mentionLabels).toContain("@terminal");
  });

  test("filters mentions and includes workspace files", () => {
    const mentions = filterMentions("@git");
    expect(mentions.some((m) => m.label === "@git-diff")).toBe(true);

    const fileMentions = filterMentions("@App", ["src/App.tsx", "src/main.rs"]);
    expect(fileMentions.some((m) => m.value === "@src/App.tsx")).toBe(true);
  });

  test("AtMentionMenu renders items with badges and paths", () => {
    const html = renderToStaticMarkup(
      <AtMentionMenu
        mentions={DEFAULT_CONTEXT_MENTIONS.slice(0, 4)}
        selectedIndex={1}
        onSelect={() => {}}
        onClose={() => {}}
      />
    );
    expect(html).toContain("data-testid=\"at-mention-menu\"");
    expect(html).toContain("@file");
    expect(html).toContain("@folder");
    expect(html).toContain("@git-diff");
  });
});

describe("UserMessage with Image Attachments", () => {
  test("renders text content cleanly without images", () => {
    const html = renderToStaticMarkup(<UserMessage content="Hello world" />);
    expect(html).toContain("Hello world");
    expect(html).not.toContain("data-testid=\"user-message-images\"");
  });

  test("renders image thumbnails when images are present", () => {
    const images = [
      {
        id: "img-1",
        url: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
        name: "test-preview.png",
      },
    ];

    const html = renderToStaticMarkup(
      <UserMessage content="Explain this image" images={images} />
    );

    expect(html).toContain("Explain this image");
    expect(html).toContain("data-testid=\"user-message-images\"");
    expect(html).toContain("data-testid=\"user-message-image-img-1\"");
    expect(html).toContain("test-preview.png");
    expect(html).toContain("data:image/png;base64");
  });
});

describe("Composer with Slash, Mention, and Image Support", () => {
  test("renders elevated card with paperclip file attach button", () => {
    const html = renderToStaticMarkup(
      <Composer onSend={() => {}} />
    );

    expect(html).toContain("data-testid=\"composer-container\"");
    expect(html).toContain("data-testid=\"composer-textarea\"");
    expect(html).toContain("data-testid=\"composer-paperclip\"");
    expect(html).toContain("accept=\"image/*,.png,.jpg,.jpeg,.webp,.gif\"");
  });
});
