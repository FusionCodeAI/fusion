import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { FusionMascot } from "../src/components/FusionMascot";

describe("FusionMascot Component", () => {
  test("renders interactive SVG mascot avatar with floating animation and testid", () => {
    const html = renderToStaticMarkup(<FusionMascot className="w-12 h-12" />);
    expect(html).toContain('data-testid="fusion-mascot"');
    expect(html).toContain("<svg");
    expect(html).toContain('viewBox="-125 -125 250 250"');
    expect(html).toContain("fusion-float");
    expect(html).toContain('fill="#5100cd"');
  });

  test("renders static mascot avatar when animated is false with size styling", () => {
    const html = renderToStaticMarkup(<FusionMascot animated={false} size={48} />);
    expect(html).toContain('data-testid="fusion-mascot"');
    expect(html).toContain('style="width:48px;height:48px"');
    expect(html).not.toContain("fusion-float");
  });
});
