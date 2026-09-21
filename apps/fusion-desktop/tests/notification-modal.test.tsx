import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { NotificationModal } from "../src/components/NotificationModal";

describe("Cline Notification Settings Modal Feature Parity", () => {
  test("renders closed when isOpen is false", () => {
    const html = renderToStaticMarkup(
      <NotificationModal isOpen={false} onClose={() => {}} />
    );
    expect(html).toBe("");
  });

  test("renders 4 Cline notification events: taskCompletion, approvalNeeded, questionAsked, sessionError", () => {
    const html = renderToStaticMarkup(
      <NotificationModal isOpen={true} onClose={() => {}} />
    );
    expect(html).toContain("Desktop Notifications");
    expect(html).toContain("Task completed");
    expect(html).toContain("Approval needed");
    expect(html).toContain("Question asked");
    expect(html).toContain("Session error");
    expect(html).toContain('data-testid="notif-toggle-taskCompletion"');
    expect(html).toContain('data-testid="sound-toggle-taskCompletion"');
  });
});
