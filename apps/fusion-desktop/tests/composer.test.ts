import { describe, expect, it } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Composer, type ComposerProps } from "../src/components/Composer";
import { HeroView, type HeroViewProps } from "../src/components/HeroView";
import { FUSION_MODELS, DEFAULT_FUSION_MODEL } from "../src/models";

describe("Composer & HeroView components", () => {
  it("renders Composer with default model and elevated card classes", () => {
    let sent = "";
    const props: ComposerProps = {
      onSend: (text) => {
        sent = text;
      },
    };

    const html = renderToStaticMarkup(React.createElement(Composer, props));

    // Must be elevated card, not pill
    expect(html).toContain("max-w-[760px]");
    expect(html).toContain("rounded-2xl");
    expect(html).toContain("bg-white");
    expect(html).toContain("border-zinc-200");
    expect(html).toContain("focus-within:border-zinc-300");

    // Must include default model shortName
    expect(html).toContain(DEFAULT_FUSION_MODEL.shortName);
    // Must include toolbar items: paperclip, model, effort (no billing profile)
    expect(html).not.toContain("Fusion Usage-Billing");
    expect(html).toContain("Low");
  });

  it("renders Composer with custom selectedModel", () => {
    const targetModel = FUSION_MODELS[2]; // GLM 5.3 Flash
    const html = renderToStaticMarkup(
      React.createElement(Composer, {
        onSend: () => {},
        selectedModel: targetModel.id,
      })
    );

    expect(html).toContain(targetModel.shortName);
  });

  it("renders HeroView with title, subtitle, and prompt composer", () => {
    let sent = "";
    const props: HeroViewProps = {
      onSend: (text) => {
        sent = text;
      },
    };

    const html = renderToStaticMarkup(React.createElement(HeroView, props));

    expect(html).toContain("What should we build today?");
    expect(html).toContain("Ask questions, plan features, or generate code");
    expect(html).toContain("max-w-[680px]");
    expect(html).toContain("rounded-2xl");
    expect(html).toContain("DeepSeek 4 Flash");
  });
});
