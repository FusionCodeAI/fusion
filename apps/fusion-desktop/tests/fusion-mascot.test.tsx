import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { FusionMascot } from "../src/components/FusionMascot";

describe("FusionMascot Component", () => {
  test("renders animated mascot avatar by default with correct image source and testid", () => {
    const html = renderToStaticMarkup(<FusionMascot className="w-12 h-12" />);
    expect(html).toContain('data-testid="fusion-mascot"');
    expect(html).toContain('src="/fusion-mascot-animated.svg"');
    expect(html).toContain('alt="Fusion Assistant"');
  });

  test("renders static mascot avatar when animated is false", () => {
    const html = renderToStaticMarkup(<FusionMascot animated={false} size={48} />);
    expect(html).toContain('src="/fusion-mascot.svg"');
    expect(html).toContain('style="width:48px;height:48px"');
  });
});
