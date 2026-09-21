# Design Spec: Cline Layout & Fusion Brand Integration for Desktop App

- **Date:** 2026-09-21
- **Status:** Approved / Ready for Implementation
- **Target:** `apps/fusion-desktop` (Tauri v2 + React 19 + Tailwind CSS v4)
- **Reference:** Visual spec provided in screenshot (`Image #1`)

---

## 1. Goal & Vision

Adapt the exact UI layout, visual structure, and component hierarchy of the **Cline Desktop Interface** (as captured in Image #1) into `fusion-desktop`, seamlessly replacing Cline's branding with **Fusion's dual-bracket logo** and **Fusion violet theme (`#5100cd`)**, while maintaining clean neutral surfaces and high-performance agent capabilities.

---

## 2. Layout Structure & Geometry

The application shell consists of a two-pane layout with native macOS framing:

```
+-----------------------------------------------------------------------------------------------+
| [Traffic Lights]  [Logo]   [<-][->][Search] |                                                  |
+---------------------------------------------+                                                  |
|  [+ Session]                                |                                                  |
|  [Schedule]                                 |                                                  |
|  [Customize]                                |                                                  |
|                                             |                [ Fusion Watermark ]              |
|  Sessions                             [=][Y]|                                                  |
|    hi                                    1h |                [ [💻] [📁 workspace] ]           |
|                                             |                                                  |
|                                             |  [ ⚡ Connect a model to start building...     ] |
|                                             |                                                  |
|                                             |  +--------------------------------------------+  |
|                                             |  | Ask to make changes, @mention files... [^] |  |
|                                             |  +--------------------------------------------+  |
|                                             |  | 📎 Fusion Usage-Billing ▾ | Model ▾ | Effort| |
| [⚙ Settings]                                |  +--------------------------------------------+  |
+---------------------------------------------+--------------------------------------------------+
```

### 2.1 Left Sidebar (`Sidebar.tsx`)
- **Width:** Fixed `w-64` (`256px`), `border-r border-zinc-200/80 bg-[#fafafa]`.
- **Top Bar:**
  - Native macOS traffic lights clearance (`pl-[78px]` or dedicated alignment).
  - Fusion dual-bracket SVG logo (`w-4 h-4 fill-[#5100cd]`).
  - Right action controls:
    - Back button (`←`) with hover state.
    - Forward button (`→`) with hover state.
    - Search button (`🔍`) with hover state.
- **Top Actions:**
  - `+ Session`: Primary action pill with subtle neutral background (`bg-zinc-200/60 font-medium text-zinc-900 rounded-lg px-3 py-1.5`).
  - `Schedule`: Clock icon (`Clock`) + label `Schedule`.
  - `Customize`: Grid/window icon (`LayoutGrid`) + label `Customize`.
- **Sessions Section:**
  - Header: `Sessions` text with two right-aligned icons:
    - Sort order icon (`ArrowDownUp` or `SlidersHorizontal`).
    - Filter icon (`Filter` funnel).
  - Session List:
    - Session item: Title on left (e.g. `hi`), relative timestamp on right (e.g. `1h`).
    - Active session highlight (`bg-zinc-100` or subtle border).
    - Hover reveals delete/rename action triggers.
- **Sidebar Footer:**
  - Fixed at bottom: `⚙ Settings` icon and label (`Settings` from lucide-react).

---

## 3. Main Workspace Canvas

### 3.1 Background & Visual Watermark
- **Background:** Clean solid `#ffffff`.
- **Center Watermark:** A subtle, elegant SVG outline of the **Fusion dual-bracket mark** centered in the upper canvas (`opacity-[0.06] w-48 h-48 stroke-[#5100cd]`), matching the watermark placement in Cline.

### 3.2 Directory & Workspace Pill
- Horizontally centered above the banner.
- Pill styling: `inline-flex items-center gap-2 px-2.5 py-1 rounded-lg border border-zinc-200/80 bg-white shadow-2xs text-xs font-medium text-zinc-700`.
- Left icon: Monitor / Desktop (`Laptop` or `Monitor`).
- Right segment: Folder icon (`Folder`) + Workspace Name (e.g. `workspace` or current folder).

### 3.3 Connect Model Banner (`ConnectModelBanner.tsx`)
- Appears prominently when in initial state or setup mode.
- Container: `rounded-xl border border-purple-200/70 bg-[#faf8ff] p-4 flex items-center justify-between shadow-2xs`.
- Left icon: Connector plug (`Zap` or `Cable`) in soft purple container (`bg-purple-100 text-[#5100cd] p-2 rounded-lg`).
- Text content:
  - Title: **Connect a model to start building** (`text-sm font-semibold text-zinc-900`).
  - Description: `Sign in with Fusion or add an API key — it takes under a minute.` (`text-xs text-zinc-500`).
- Right action buttons:
  - Primary button: `Connect a model` (`bg-[#5100cd] hover:bg-[#4300a8] text-white text-xs font-medium rounded-full px-4 py-1.5 transition-colors shadow-xs cursor-pointer`).
  - Secondary button: `Model settings` (`text-xs font-medium text-zinc-700 hover:text-zinc-900 px-3 py-1.5 cursor-pointer`).

### 3.4 Composer Box (`Composer.tsx`)
- Container: `w-full max-w-[760px] mx-auto rounded-2xl border border-zinc-200 bg-white shadow-sm focus-within:border-zinc-300 focus-within:shadow-md transition-all`.
- Upper textarea:
  - Placeholder: `Ask to make changes, @mention files, reference #PRs, or run /commands.`
  - Send button: Circular submit button in the bottom-right corner of the input area with purple arrow (`ArrowUp`), active when text is typed or disabled when empty.
- Lower toolbar (split by subtle border `border-t border-zinc-100 px-3 py-2 flex items-center justify-between text-xs`):
  - Left Group:
    - Attachment button: Paperclip (`Paperclip`) with tooltip `Attach files or images`.
    - Profile / Billing selector: `Fusion Usage-Billing ▾` (`ChevronDown`).
    - Divider: `|` in `text-zinc-200`.
    - Model selector: `DeepSeek 4 Flash ▾` (or `Claude Sonnet 5 ▾`, etc.).
    - Divider: `|` in `text-zinc-200`.
    - Reasoning effort: `🌐 Low ▾` (or `⚡ Fast` / `🧠 High`).

---

## 4. Conversation Stream & Interactive Execution Cards

When a conversation is active (messages exist), the hero watermark and banner transition smoothly into the vertical chat stream:

1. **User Message:**
   - Soft rounded bubble on the right side (`bg-zinc-100 text-zinc-900 px-4 py-2.5 rounded-2xl max-w-[80%] text-[13px]`).
2. **Thinking Row (`ThinkingRow.tsx`):**
   - Expandable disclosure with lightbulb/brain icon, elapsed time (`Thinking (4.2s)`), and markdown formatted thought reasoning.
3. **Execution Cards:**
   - **File Edit Card:** Header `Edited <file>`, diff stats `+12 -3`, expandable diff patch with syntax colors.
   - **Terminal Command Card:** Header `Ran <command>`, expandable terminal box with live stdout/stderr.
   - **Approval Checkpoint Gate:** `Approve` (green button) / `Reject` (red button) / `Always allow` checkbox for command safety.
4. **Markdown Assistant Responses:**
   - Rendered using `MarkdownRenderer` with formatted lists, code blocks with copy button, blockquotes, and external links.

---

## 5. Color Palette & Brand Token Alignment

| Token | Value | Purpose |
|---|---|---|
| `--color-brand` | `#5100cd` | Fusion brand primary violet |
| `--color-brand-hover` | `#4300a8` | Button hover state |
| `--color-brand-light` | `#faf8ff` | Banner card background |
| `--color-brand-border` | `rgba(81, 0, 205, 0.15)` | Banner card border |
| `--color-sidebar` | `#fafafa` | Left sidebar background |
| `--color-canvas` | `#ffffff` | Main workspace background |
| `--color-border` | `rgba(228, 228, 231, 0.8)` | General borders (`zinc-200`) |

---

## 6. Verification & Acceptance Criteria

1. **Visual Match:** 1:1 match with Image #1 (Sidebar navigation, `+ Session`, `Schedule`, `Customize`, centered Fusion watermark, directory pill, purple model banner, composer toolbar).
2. **Type Safety:** 0 TypeScript errors on `bun run typecheck`.
3. **Automated Tests:** All unit and component tests pass (`bun test`).
4. **Desktop Native Verification:** Native macOS window runs cleanly with no titlebar glitches.
