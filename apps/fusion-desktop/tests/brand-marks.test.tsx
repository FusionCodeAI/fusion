import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { FusionLogo } from "../src/components/FusionLogo";
import { FusionWatermark } from "../src/components/FusionWatermark";

describe("Fusion Brand Marks", () => {
  test("renders FusionLogo with proper SVG brackets and primary violet color", () => {
    const html = renderToStaticMarkup(<FusionLogo className="w-5 h-5" fill="#5100cd" />);
    expect(html).toContain("<svg");
    expect(html).toContain('fill="#5100cd"');
    expect(html).toContain('viewBox="0 0 1024 1024"');
  });

  test("renders FusionWatermark with subtle opacity and background geometry", () => {
    const html = renderToStaticMarkup(<FusionWatermark className="w-48 h-48" />);
    expect(html).toContain("<svg");
    expect(html).toContain("opacity-");
  });
});
