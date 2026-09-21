import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { PushToast, type PushToastMessage } from "../src/components/PushToast";

describe("Push Toast Floating Notifications", () => {
  test("renders nothing when toasts is empty", () => {
    const html = renderToStaticMarkup(<PushToast toasts={[]} onDismiss={() => {}} />);
    expect(html).toBe("");
  });

  test("renders floating toast banner with icon, title, body, and dismiss button", () => {
    const sampleToasts: PushToastMessage[] = [
      {
        id: "t1",
        type: "taskCompletion",
        title: "Task completed",
        body: "Fusion finished working in 'threejs-shooter'.",
        timestamp: Date.now(),
      },
    ];

    const html = renderToStaticMarkup(
      <PushToast toasts={sampleToasts} onDismiss={() => {}} />
    );

    expect(html).toContain('data-testid="push-toast-container"');
    expect(html).toContain('data-testid="push-toast-item"');
    expect(html).toContain("Task completed");
    expect(html).toContain("Fusion finished working in &#x27;threejs-shooter&#x27;.");
    expect(html).toContain("Dismiss notification");
  });
});
