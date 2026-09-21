# Cline Layout & Fusion Brand Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform the Fusion Desktop UI (`apps/fusion-desktop`) into a 1:1 pixel-accurate adaptation of the Cline interface (as provided in Image #1), featuring Fusion's dual-bracket SVG brand mark, violet theme (`#5100cd`), Cline's navigation sidebar, workspace pill, model connection banner, and composer action bar.

**Architecture:** Maintain the lightweight Tauri v2 host and React 19 + Tailwind CSS v4 component architecture. Refactor `Sidebar.tsx`, create `FusionWatermark.tsx` and `ConnectModelBanner.tsx`, update `Composer.tsx` toolbar, and bind them seamlessly in `App.tsx` with full support for both empty hero state and streaming chat cards.

**Tech Stack:** React 19, TypeScript 5.7, Tailwind CSS v4, Lucide React, Tauri v2, Bun Test.

---

### Task 1: Fusion Brand Marks (`FusionLogo` & `FusionWatermark`)

**Files:**
- Create: `apps/fusion-desktop/src/components/FusionLogo.tsx`
- Create: `apps/fusion-desktop/src/components/FusionWatermark.tsx`
- Test: `apps/fusion-desktop/tests/brand-marks.test.tsx`

- [ ] **Step 1: Write the failing test**

```tsx
// apps/fusion-desktop/tests/brand-marks.test.tsx
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/fusion-desktop && bun test tests/brand-marks.test.tsx`
Expected: FAIL with "Cannot find module '../src/components/FusionLogo'"

- [ ] **Step 3: Write minimal implementation**

```tsx
// apps/fusion-desktop/src/components/FusionLogo.tsx
import React from "react";

export interface FusionLogoProps {
  className?: string;
  fill?: string;
  size?: number;
}

export function FusionLogo({ className = "w-4 h-4", fill = "#5100cd", size }: FusionLogoProps) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 1024 1024"
      width={size}
      height={size}
      className={className}
      fill="none"
      data-testid="fusion-logo"
    >
      {/* Outer bracket ┌ */}
      <path d="M110 110 H864 V210 H210 V864 H110 Z" fill={fill} />
      {/* Inner bracket ┌ */}
      <path d="M382 382 H864 V482 H482 V864 H382 Z" fill={fill} />
    </svg>
  );
}
```

```tsx
// apps/fusion-desktop/src/components/FusionWatermark.tsx
import React from "react";

export interface FusionWatermarkProps {
  className?: string;
}

export function FusionWatermark({ className = "w-56 h-56" }: FusionWatermarkProps) {
  return (
    <div
      data-testid="fusion-watermark"
      className={`pointer-events-none select-none flex items-center justify-center opacity-[0.06] text-[#5100cd] ${className}`}
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 1024 1024"
        className="w-full h-full"
        fill="currentColor"
      >
        <path d="M110 110 H864 V210 H210 V864 H110 Z" />
        <path d="M382 382 H864 V482 H482 V864 H382 Z" />
      </svg>
    </div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/fusion-desktop && bun test tests/brand-marks.test.tsx`
Expected: PASS (2 tests passed)

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop/src/components/FusionLogo.tsx apps/fusion-desktop/src/components/FusionWatermark.tsx apps/fusion-desktop/tests/brand-marks.test.tsx
git commit -m "feat(desktop): implement FusionLogo and FusionWatermark components"
```

---

### Task 2: Re-architect Sidebar (`Sidebar.tsx`) to Match Cline Layout

**Files:**
- Modify: `apps/fusion-desktop/src/components/Sidebar.tsx`
- Test: `apps/fusion-desktop/tests/sidebar-cline.test.tsx`

- [ ] **Step 1: Write the failing test**

```tsx
// apps/fusion-desktop/tests/sidebar-cline.test.tsx
import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Sidebar } from "../src/components/Sidebar";

describe("Sidebar Cline Layout", () => {
  test("renders traffic light clearance, FusionLogo, navigation arrows, and search", () => {
    const html = renderToStaticMarkup(
      <Sidebar
        sessions={[{ id: "s1", title: "hi", createdAt: Date.now() - 3600 * 1000, updatedAt: Date.now() - 3600 * 1000 }]}
        activeSessionId="s1"
        onNewChat={() => {}}
        onSelectSession={() => {}}
      />
    );
    expect(html).toContain('data-testid="fusion-logo"');
    expect(html).toContain("+ Session");
    expect(html).toContain("Schedule");
    expect(html).toContain("Customize");
    expect(html).toContain("Sessions");
    expect(html).toContain("Settings");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/fusion-desktop && bun test tests/sidebar-cline.test.tsx`
Expected: FAIL with missing "+ Session" or elements

- [ ] **Step 3: Update `Sidebar.tsx`**

Update `Sidebar.tsx` to match the exact structure in Image #1:
- Header: traffic light clearance (`pl-[78px]`), `FusionLogo`, `ArrowLeft`, `ArrowRight`, `Search`.
- Primary actions:
  - `+ Session` button: `bg-zinc-200/60 font-medium text-zinc-900 rounded-lg px-3 py-1.5 flex items-center gap-2`.
  - `Schedule` item with `Clock` icon.
  - `Customize` item with `LayoutGrid` icon.
- Section: `Sessions` with sort button (`ArrowUpDown`) and filter button (`Filter`).
- Session list: item showing `title` on left and `1h` on right.
- Footer: pinned `Settings` (`⚙ Settings`) button at the bottom.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/fusion-desktop && bun test tests/sidebar-cline.test.tsx`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop/src/components/Sidebar.tsx apps/fusion-desktop/tests/sidebar-cline.test.tsx
git commit -m "feat(desktop): adapt Sidebar to Cline navigation hierarchy"
```

---

### Task 3: Workspace Pill & Connect Model Banner (`ConnectModelBanner.tsx`)

**Files:**
- Create: `apps/fusion-desktop/src/components/WorkspacePill.tsx`
- Create: `apps/fusion-desktop/src/components/ConnectModelBanner.tsx`
- Test: `apps/fusion-desktop/tests/banner-components.test.tsx`

- [ ] **Step 1: Write the failing test**

```tsx
// apps/fusion-desktop/tests/banner-components.test.tsx
import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { WorkspacePill } from "../src/components/WorkspacePill";
import { ConnectModelBanner } from "../src/components/ConnectModelBanner";

describe("Workspace Pill & Connect Model Banner", () => {
  test("renders WorkspacePill with desktop and folder icons", () => {
    const html = renderToStaticMarkup(<WorkspacePill name="workspace" />);
    expect(html).toContain("workspace");
    expect(html).toContain("data-testid=\"workspace-pill\"");
  });

  test("renders ConnectModelBanner with Fusion messaging and violet button", () => {
    const html = renderToStaticMarkup(
      <ConnectModelBanner onConnect={() => {}} onSettings={() => {}} />
    );
    expect(html).toContain("Connect a model to start building");
    expect(html).toContain("Connect a model");
    expect(html).toContain("Model settings");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/fusion-desktop && bun test tests/banner-components.test.tsx`
Expected: FAIL

- [ ] **Step 3: Implement `WorkspacePill.tsx` and `ConnectModelBanner.tsx`**

Implement `WorkspacePill.tsx`:
```tsx
import React from "react";
import { Monitor, Folder } from "lucide-react";

export function WorkspacePill({ name = "workspace" }: { name?: string }) {
  return (
    <div
      data-testid="workspace-pill"
      className="inline-flex items-center gap-2 px-2.5 py-1 rounded-lg border border-zinc-200/80 bg-white shadow-2xs text-xs font-medium text-zinc-700"
    >
      <Monitor className="w-3.5 h-3.5 text-zinc-500" />
      <div className="flex items-center gap-1.5 pl-1.5 border-l border-zinc-200/80">
        <Folder className="w-3.5 h-3.5 text-zinc-500" />
        <span>{name}</span>
      </div>
    </div>
  );
}
```

Implement `ConnectModelBanner.tsx`:
```tsx
import React from "react";
import { Zap } from "lucide-react";

export function ConnectModelBanner({
  onConnect,
  onSettings,
}: {
  onConnect?: () => void;
  onSettings?: () => void;
}) {
  return (
    <div
      data-testid="connect-model-banner"
      className="w-full max-w-[760px] mx-auto rounded-xl border border-purple-200/70 bg-[#faf8ff] p-3.5 flex items-center justify-between shadow-2xs"
    >
      <div className="flex items-center gap-3">
        <div className="w-8 h-8 rounded-lg bg-purple-100/80 flex items-center justify-center text-[#5100cd] shrink-0">
          <Zap className="w-4 h-4" />
        </div>
        <div>
          <div className="text-xs font-semibold text-zinc-900">
            Connect a model to start building
          </div>
          <div className="text-[11px] text-zinc-500">
            Sign in with Fusion or add an API key — it takes under a minute.
          </div>
        </div>
      </div>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onConnect}
          className="bg-[#5100cd] hover:bg-[#4300a8] text-white text-xs font-medium rounded-full px-3.5 py-1.5 transition-colors shadow-xs cursor-pointer"
        >
          Connect a model
        </button>
        <button
          type="button"
          onClick={onSettings}
          className="text-xs font-medium text-zinc-600 hover:text-zinc-900 px-2.5 py-1.5 cursor-pointer"
        >
          Model settings
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/fusion-desktop && bun test tests/banner-components.test.tsx`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop/src/components/WorkspacePill.tsx apps/fusion-desktop/src/components/ConnectModelBanner.tsx apps/fusion-desktop/tests/banner-components.test.tsx
git commit -m "feat(desktop): implement WorkspacePill and ConnectModelBanner"
```

---

### Task 4: Re-architect Composer (`Composer.tsx`) to Match Cline Card Toolbar

**Files:**
- Modify: `apps/fusion-desktop/src/components/Composer.tsx`
- Test: `apps/fusion-desktop/tests/composer-cline.test.tsx`

- [ ] **Step 1: Write the failing test**

```tsx
// apps/fusion-desktop/tests/composer-cline.test.tsx
import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Composer } from "../src/components/Composer";

describe("Composer Cline Layout", () => {
  test("renders Cline-style placeholder, paperclip, billing profile, model picker, and effort selector", () => {
    const html = renderToStaticMarkup(
      <Composer onSend={() => {}} selectedModel="deepseek-4-flash" />
    );
    expect(html).toContain("Ask to make changes, @mention files, reference #PRs, or run /commands.");
    expect(html).toContain("Fusion Usage-Billing");
    expect(html).toContain("Low");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/fusion-desktop && bun test tests/composer-cline.test.tsx`
Expected: FAIL

- [ ] **Step 3: Update `Composer.tsx`**

Refactor `Composer.tsx`:
- Textarea placeholder: `"Ask to make changes, @mention files, reference #PRs, or run /commands."`
- Send button: circular button with purple arrow (`ArrowUp`).
- Bottom toolbar:
  - Left group:
    - Attachment paperclip icon (`Paperclip`).
    - Provider / Billing dropdown: `Fusion Usage-Billing ▾`.
    - Divider `|`.
    - Model picker: `DeepSeek 4 Flash ▾` (or selected model).
    - Divider `|`.
    - Reasoning effort selector: `🌐 Low ▾`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/fusion-desktop && bun test tests/composer-cline.test.tsx`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop/src/components/Composer.tsx apps/fusion-desktop/tests/composer-cline.test.tsx
git commit -m "feat(desktop): adapt Composer to Cline card layout and toolbar"
```

---

### Task 5: Assemble Hero Canvas & Integrate into `App.tsx`

**Files:**
- Create/Modify: `apps/fusion-desktop/src/components/ClineHeroView.tsx`
- Modify: `apps/fusion-desktop/src/components/index.ts`
- Modify: `apps/fusion-desktop/src/App.tsx`
- Test: `apps/fusion-desktop/tests/app-cline-integration.test.tsx`

- [ ] **Step 1: Write the failing test**

```tsx
// apps/fusion-desktop/tests/app-cline-integration.test.tsx
import { describe, test, expect } from "bun:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { App } from "../src/App";

describe("App Integration with Cline Layout", () => {
  test("renders FusionWatermark, WorkspacePill, ConnectModelBanner, and Composer in initial state", () => {
    const html = renderToStaticMarkup(<App initialMessages={[]} />);
    expect(html).toContain('data-testid="fusion-watermark"');
    expect(html).toContain('data-testid="workspace-pill"');
    expect(html).toContain('data-testid="connect-model-banner"');
    expect(html).toContain("Fusion Usage-Billing");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd apps/fusion-desktop && bun test tests/app-cline-integration.test.tsx`
Expected: FAIL

- [ ] **Step 3: Implement `ClineHeroView.tsx` and wire in `App.tsx`**

- Connect `FusionWatermark`, `WorkspacePill`, and `ConnectModelBanner` inside `ClineHeroView.tsx`.
- Wire `ClineHeroView` in `App.tsx` when `messages.length === 0`.
- Maintain transition to chat stream when messages are sent.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd apps/fusion-desktop && bun test tests/app-cline-integration.test.tsx`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add apps/fusion-desktop/src/components/ apps/fusion-desktop/src/App.tsx apps/fusion-desktop/tests/app-cline-integration.test.tsx
git commit -m "feat(desktop): integrate ClineHeroView and full Cline layout into App"
```

---

### Task 6: Full Verification & Visual Confirmation

**Files:**
- Test all: `apps/fusion-desktop/tests/*.test.ts*`

- [ ] **Step 1: Run typecheck**

Run: `cd apps/fusion-desktop && bun run typecheck`
Expected: `tsc --noEmit` exits with 0 errors.

- [ ] **Step 2: Run all test suites**

Run: `cd apps/fusion-desktop && bun test`
Expected: 100% passing tests (0 failures).

- [ ] **Step 3: Production build**

Run: `cd apps/fusion-desktop && bun run build`
Expected: Vite build succeeds with 0 errors.

- [ ] **Step 4: Visual Confirmation via Headless Browser**

Drive `http://localhost:5173/` using `browser` tool, capture high-res screenshot, and confirm 1:1 match with Image #1.

- [ ] **Step 5: Final Commit**

```bash
git add apps/fusion-desktop/
git commit -m "feat(desktop): complete pixel-accurate Cline layout adaptation with Fusion branding"
```
