/**
 * Fusion v2 — xterm.js Browser Terminal Adapter
 *
 * High-performance, fully typed TypeScript adapter connecting xterm.js
 * to Fusion WebAssembly agents and WebSocket ACP (Agent Client Protocol) servers.
 *
 * Features:
 * - Dual operational modes: Interactive Line Editor (REPL) and Raw PTY streaming
 * - ANSI parser & HTML renderer with custom theme color mapping
 * - Built-in Fusion theme palettes: Deep Ocean, Cyberpunk, Monokai, Nord, Gruvbox, Tokyo Night
 * - Terminal auto-resize handling with FitAddon & ResizeObserver integration
 * - Paste event sanitization with bracketed paste and control-code stripping
 * - Keyboard shortcuts: Ctrl+C, Ctrl+D, Shift+Enter (multiline), Ctrl+J, Ctrl+A/E/K/U/W/L
 * - Streaming token insertion with throughput metrics & thinking delta support
 * - Inline animated spinners (dots, line, pulse, arrows, bounce, moon)
 * - Extensible Slash Command engine with tab autocompletion & history search
 *
 * @packageDocumentation
 */

// ============================================================================
// 1. Core Interfaces & Types
// ============================================================================

/**
 * Disposable resource interface.
 */
export interface IDisposable {
  dispose(): void;
}

/**
 * Minimal structural interface for an xterm.js Terminal instance.
 * Compatible with `@xterm/xterm` (v5+ & v6+) and legacy `xterm.Terminal`.
 */
export interface XtermInstance {
  cols: number;
  rows: number;
  element?: HTMLElement;
  textarea?: HTMLTextAreaElement;
  options?: Record<string, unknown>;
  buffer?: {
    active: {
      cursorX: number;
      cursorY: number;
      baseY: number;
      viewportY: number;
      length: number;
      getLine(y: number): { translateToString(trimRight?: boolean): string } | undefined;
    };
  };

  write(data: string | Uint8Array, callback?: () => void): void;
  writeln(data: string | Uint8Array, callback?: () => void): void;
  clear(): void;
  reset(): void;
  focus?(): void;
  blur?(): void;
  resize(cols: number, rows: number): void;
  open?(parent: HTMLElement): void;
  dispose?(): void;
  loadAddon?(addon: unknown): void;

  onData(listener: (data: string) => void): IDisposable;
  onKey?(listener: (e: { key: string; domEvent: KeyboardEvent }) => void): IDisposable;
  onResize?(listener: (e: { cols: number; rows: number }) => void): IDisposable;
  onBinary?(listener: (data: string) => void): IDisposable;
  onSelectionChange?(listener: () => void): IDisposable;
  onRender?(listener: (e: { start: number; end: number }) => void): IDisposable;
  onScroll?(listener: (ydisp: number) => void): IDisposable;
}

/**
 * Interface for xterm FitAddon (`@xterm/addon-fit`).
 */
export interface FitAddonInstance {
  fit(): void;
  proposeDimensions?(): { cols: number; rows: number } | undefined;
  dispose?(): void;
}

/**
 * Terminal color theme definition.
 */
export interface TerminalTheme {
  name?: string;
  background: string;
  foreground: string;
  cursor: string;
  cursorAccent?: string;
  selectionBackground: string;
  selectionForeground?: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
  [key: string]: string | undefined;
}

/**
 * Real-time event emitted during agent turn execution.
 */
export type AgentEvent =
  | { type: 'status'; message: string; level?: 'info' | 'warn' | 'error' | 'success' }
  | { type: 'thinking_delta'; delta: string }
  | { type: 'text_delta'; delta: string }
  | { type: 'tool_started'; id: string; name: string; args?: Record<string, unknown> }
  | { type: 'tool_finished'; id: string; name: string; success: boolean; output: string; duration_ms?: number }
  | { type: 'advisor_critique'; advisor: string; approved: boolean; critique: string }
  | { type: 'finished'; usage?: { prompt_tokens?: number; completion_tokens?: number; total_tokens?: number } }
  | { type: 'error'; message: string; code?: number | string }
  | { type: string; [key: string]: unknown };

/**
 * Callback function for receiving real-time agent streaming events.
 */
export type AgentEventCallback = (event: AgentEvent) => void;

/**
 * Structural interface for the WebAssembly Fusion Agent (`WasmFusionAgent`).
 */
export interface WasmFusionAgentInstance {
  prompt_turn(
    input: string,
    callback?: (event: unknown) => void
  ): Promise<string> | string;
  checkpoint?(): string;
  restore?(checkpointJson: string): void;
  get_session_id?(): string;
  get_active_model?(): string;
  set_active_model?(model: string): void;
  clear_messages?(): void;
  fs_write?(path: string, content: string): void;
  fs_read?(path: string): string;
  fs_list?(): string;
  fs_delete?(path: string): boolean;
}

/**
 * Universal Timer Identifier type for browser (number) and Node (Timeout object) environments.
 */
export type TimerId = number | { ref?: () => void; unref?: () => void } | unknown;

/**
 * Agent Backend interface connecting xterm adapter to WASM or WebSocket agent.
 */
export interface AgentBackend {
  readonly id: string;
  readonly name: string;
  readonly isConnected: boolean;

  /**
   * Initializes the backend connection.
   */
  connect?(): Promise<void>;

  /**
   * Closes the backend connection.
   */
  disconnect?(): Promise<void>;

  /**
   * Executes a prompt turn with real-time event streaming.
   */
  promptTurn(
    prompt: string,
    onEvent: AgentEventCallback,
    signal?: AbortSignal
  ): Promise<string>;

  /**
   * Cancels the currently running turn.
   */
  cancelTurn?(): Promise<void>;

  /**
   * Notifies backend of terminal window resizing.
   */
  resizeTerminal?(cols: number, rows: number): void;

  /**
   * Exports checkpoint session JSON if supported.
   */
  checkpoint?(): Promise<string> | string;

  /**
   * Restores session from checkpoint JSON if supported.
   */
  restore?(checkpointJson: string): Promise<void> | void;
}

/**
 * Session token and cost statistics.
 */
export interface SessionStats {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  totalTurns: number;
  estimatedCost: number;
}

/**
 * Slash command handler definition.
 */
export interface SlashCommand {
  name: string;
  description: string;
  usage?: string;
  aliases?: string[];
  handler: (args: string, adapter: XtermAdapter) => boolean | void | Promise<boolean | void>;
}

/**
 * Configuration options for XtermAdapter.
 */
export interface XtermAdapterOptions {
  /** Existing terminal instance (optional; can be attached later via attach()) */
  terminal?: XtermInstance;
  /** HTML container element or selector to mount terminal */
  container?: HTMLElement | string;
  /** FitAddon instance for auto-fitting */
  fitAddon?: FitAddonInstance;
  /** Enable auto-fitting on window resize and container resize */
  autoFit?: boolean;
  /** Operational mode: 'interactive' (REPL line-editor) or 'raw' (direct PTY byte forwarding) */
  mode?: 'interactive' | 'raw' | 'hybrid';
  /** Command prompt prefix string or factory */
  prompt?: string | (() => string);
  /** Multiline continuation prompt prefix */
  multilinePrompt?: string | (() => string);
  /** Print welcome banner on terminal initialization */
  welcomeBanner?: boolean | string;
  /** Active agent backend */
  backend?: AgentBackend;
  /** Active model identifier (e.g. 'anthropic/claude-3-5-sonnet') */
  model?: string;
  /** Max commands in history buffer */
  historyLimit?: number;
  /** LocalStorage key for persisting history across page reloads */
  historyStorageKey?: string;
  /** Enable built-in slash commands (/help, /model, /clear, etc.) */
  enableSlashCommands?: boolean;
  /** Custom slash commands to register */
  customCommands?: SlashCommand[];
  /** Custom onData handler interceptor */
  onData?: (data: string) => boolean | void;
  /** Prompt submit interceptor */
  onPromptSubmit?: (prompt: string) => boolean | void | Promise<boolean | void>;
  /** Terminal resize listener */
  onResize?: (cols: number, rows: number) => void;
  /** Theme colors or theme name preset override */
  theme?: string | Partial<TerminalTheme>;
  /** Sanitize pasted content */
  sanitizePaste?: boolean;
  /** Allow multiline inputs via Shift+Enter / Ctrl+J */
  allowMultiline?: boolean;
}

/**
 * Options for stream token insertion.
 */
export interface StreamTokenOptions {
  role?: 'assistant' | 'thinking' | 'system' | 'user';
  style?: string;
  trackUsage?: boolean;
  onDelta?: (delta: string) => void;
}

/**
 * Spinner animation style identifiers.
 */
export type SpinnerStyle = 'dots' | 'line' | 'pulse' | 'arrows' | 'bounce' | 'moon';

/**
 * Options for terminal spinner helper.
 */
export interface SpinnerOptions {
  style?: SpinnerStyle;
  interval?: number;
  color?: string;
  prefix?: string;
}

// ============================================================================
// 2. Helper: Deferred Promise Resolver
// ============================================================================

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T | PromiseLike<T>) => void;
  reject: (reason?: unknown) => void;
}

function createDeferred<T>(): Deferred<T> {
  if (typeof Promise.withResolvers === 'function') {
    return Promise.withResolvers<T>();
  }
  let resolveFunc!: (value: T | PromiseLike<T>) => void;
  let rejectFunc!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolveFunc = res;
    rejectFunc = rej;
  });
  return { promise, resolve: resolveFunc, reject: rejectFunc };
}

// ============================================================================
// 3. ANSI & Styling Constants
// ============================================================================

export const ANSI = {
  reset: '\x1b[0m',
  bold: '\x1b[1m',
  dim: '\x1b[2m',
  italic: '\x1b[3m',
  underline: '\x1b[4m',
  inverse: '\x1b[7m',
  hidden: '\x1b[8m',
  strikethrough: '\x1b[9m',

  // 16 Standard Colors
  black: '\x1b[30m',
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  magenta: '\x1b[35m',
  cyan: '\x1b[36m',
  white: '\x1b[37m',

  // Bright Colors
  brightBlack: '\x1b[90m',
  brightRed: '\x1b[91m',
  brightGreen: '\x1b[92m',
  brightYellow: '\x1b[93m',
  brightBlue: '\x1b[94m',
  brightMagenta: '\x1b[95m',
  brightCyan: '\x1b[96m',
  brightWhite: '\x1b[97m',

  // Background Colors
  bgBlack: '\x1b[40m',
  bgRed: '\x1b[41m',
  bgGreen: '\x1b[42m',
  bgYellow: '\x1b[43m',
  bgBlue: '\x1b[44m',
  bgMagenta: '\x1b[45m',
  bgCyan: '\x1b[46m',
  bgWhite: '\x1b[47m',

  // 256 Palette Favorites
  purple: '\x1b[38;5;141m',
  neonCyan: '\x1b[38;5;51m',
  emerald: '\x1b[38;5;48m',
  amber: '\x1b[38;5;214m',
  slate: '\x1b[38;5;244m',
  darkGray: '\x1b[38;5;238m',
  rose: '\x1b[38;5;204m',
  lavender: '\x1b[38;5;183m',
  sky: '\x1b[38;5;117m',

  /**
   * Generates 256-color foreground ANSI code.
   */
  fg256: (code: number) => `\x1b[38;5;${code}m`,

  /**
   * Generates 256-color background ANSI code.
   */
  bg256: (code: number) => `\x1b[48;5;${code}m`,

  /**
   * Generates 24-bit TrueColor RGB foreground ANSI code.
   */
  rgb: (r: number, g: number, b: number) => `\x1b[38;2;${r};${g};${b}m`,

  /**
   * Generates 24-bit TrueColor RGB background ANSI code.
   */
  bgRgb: (r: number, g: number, b: number) => `\x1b[48;2;${r};${g};${b}m`,

  /**
   * Cursor and screen controls
   */
  cursor: {
    up: (n = 1) => `\x1b[${n}A`,
    down: (n = 1) => `\x1b[${n}B`,
    forward: (n = 1) => `\x1b[${n}C`,
    back: (n = 1) => `\x1b[${n}D`,
    toColumn: (n = 1) => `\x1b[${n}G`,
    toPosition: (row = 1, col = 1) => `\x1b[${row};${col}H`,
    save: '\x1b[s',
    restore: '\x1b[u',
    hide: '\x1b[?25l',
    show: '\x1b[?25h',
    clearToEndOfLine: '\x1b[K',
    clearToStartOfLine: '\x1b[1K',
    clearLine: '\x1b[2K',
    clearScreen: '\x1b[2J\x1b[H'
  }
};

// ============================================================================
// 4. Built-in Theme Presets for Fusion
// ============================================================================

export const THEMES: Record<string, TerminalTheme> = {
  // 1. Deep Ocean (Oceanic navy & aqua neon)
  deepOcean: {
    name: 'Deep Ocean',
    background: '#0b132b',
    foreground: '#e0fbfc',
    cursor: '#00f0ff',
    cursorAccent: '#0b132b',
    selectionBackground: '#1c2541',
    selectionForeground: '#e0fbfc',
    black: '#0a1128',
    red: '#ff5470',
    green: '#00ebc7',
    yellow: '#fde24f',
    blue: '#0077b6',
    magenta: '#7209b7',
    cyan: '#48cae4',
    white: '#e0fbfc',
    brightBlack: '#1c2541',
    brightRed: '#ff708d',
    brightGreen: '#38ef7d',
    brightYellow: '#ffeb3b',
    brightBlue: '#00b4d8',
    brightMagenta: '#9d4edd',
    brightCyan: '#90e0ef',
    brightWhite: '#ffffff'
  },

  // 2. Cyberpunk (High-voltage fluorescent neon & dark violet)
  cyberpunk: {
    name: 'Cyberpunk',
    background: '#0d0d1a',
    foreground: '#00ffcc',
    cursor: '#ff007f',
    cursorAccent: '#0d0d1a',
    selectionBackground: '#330066',
    selectionForeground: '#00ffcc',
    black: '#101020',
    red: '#ff0055',
    green: '#00ff66',
    yellow: '#ffe600',
    blue: '#00aaff',
    magenta: '#ff00ff',
    cyan: '#00ffff',
    white: '#e0e0ff',
    brightBlack: '#303050',
    brightRed: '#ff3377',
    brightGreen: '#33ff88',
    brightYellow: '#ffee33',
    brightBlue: '#33bbff',
    brightMagenta: '#ff33ff',
    brightCyan: '#33ffff',
    brightWhite: '#ffffff'
  },

  // 3. Monokai (Classic charcoal with vibrant lime/yellow/magenta)
  monokai: {
    name: 'Monokai',
    background: '#272822',
    foreground: '#f8f8f2',
    cursor: '#f8f8f0',
    cursorAccent: '#272822',
    selectionBackground: '#49483e',
    selectionForeground: '#f8f8f2',
    black: '#272822',
    red: '#f92672',
    green: '#a6e22e',
    yellow: '#e6db74',
    blue: '#66d9ef',
    magenta: '#ae81ff',
    cyan: '#a1efe4',
    white: '#f8f8f2',
    brightBlack: '#75715e',
    brightRed: '#f92672',
    brightGreen: '#a6e22e',
    brightYellow: '#e6db74',
    brightBlue: '#66d9ef',
    brightMagenta: '#ae81ff',
    brightCyan: '#a1efe4',
    brightWhite: '#f9f8f5'
  },

  // 4. Nord (Arctic, north-bluish clean aesthetic)
  nord: {
    name: 'Nord',
    background: '#2e3440',
    foreground: '#d8dee9',
    cursor: '#d8dee9',
    cursorAccent: '#2e3440',
    selectionBackground: '#434c5e',
    selectionForeground: '#eceff4',
    black: '#3b4252',
    red: '#bf616a',
    green: '#a3be8c',
    yellow: '#ebcb8b',
    blue: '#81a1c1',
    magenta: '#b48ead',
    cyan: '#88c0d0',
    white: '#e5e9f0',
    brightBlack: '#4c566a',
    brightRed: '#bf616a',
    brightGreen: '#a3be8c',
    brightYellow: '#ebcb8b',
    brightBlue: '#81a1c1',
    brightMagenta: '#b48ead',
    brightCyan: '#8fbcbb',
    brightWhite: '#eceff4'
  },

  // 5. Gruvbox (Retro groove dark & warm tones)
  gruvbox: {
    name: 'Gruvbox',
    background: '#282828',
    foreground: '#ebdbb2',
    cursor: '#ebdbb2',
    cursorAccent: '#282828',
    selectionBackground: '#504945',
    selectionForeground: '#ebdbb2',
    black: '#282828',
    red: '#cc241d',
    green: '#98971a',
    yellow: '#d79921',
    blue: '#458588',
    magenta: '#b16286',
    cyan: '#689d6a',
    white: '#a89984',
    brightBlack: '#928374',
    brightRed: '#fb4934',
    brightGreen: '#b8bb26',
    brightYellow: '#fabd2f',
    brightBlue: '#83a598',
    brightMagenta: '#d3869b',
    brightCyan: '#8ec07c',
    brightWhite: '#ebdbb2'
  },

  // 6. Tokyo Night (Balanced deep indigo & neon accents)
  tokyoNight: {
    name: 'Tokyo Night',
    background: '#1a1b26',
    foreground: '#c0caf5',
    cursor: '#7aa2f7',
    cursorAccent: '#1a1b26',
    selectionBackground: '#283457',
    selectionForeground: '#c0caf5',
    black: '#15161e',
    red: '#f7768e',
    green: '#9ece6a',
    yellow: '#e0af68',
    blue: '#7aa2f7',
    magenta: '#bb9af7',
    cyan: '#7dcfff',
    white: '#a9b1d6',
    brightBlack: '#414868',
    brightRed: '#f7768e',
    brightGreen: '#9ece6a',
    brightYellow: '#e0af68',
    brightBlue: '#7aa2f7',
    brightMagenta: '#bb9af7',
    brightCyan: '#7dcfff',
    brightWhite: '#c0caf5'
  },

  // Additional Popular Themes
  catppuccinMocha: {
    name: 'Catppuccin Mocha',
    background: '#1e1e2e',
    foreground: '#cdd6f4',
    cursor: '#f5e0dc',
    cursorAccent: '#11111b',
    selectionBackground: '#45475a',
    selectionForeground: '#cdd6f4',
    black: '#45475a',
    red: '#f38ba8',
    green: '#a6e3a1',
    yellow: '#f9e2af',
    blue: '#89b4fa',
    magenta: '#f5c2e7',
    cyan: '#94e2d5',
    white: '#bac2de',
    brightBlack: '#585b70',
    brightRed: '#f38ba8',
    brightGreen: '#a6e3a1',
    brightYellow: '#f9e2af',
    brightBlue: '#89b4fa',
    brightMagenta: '#f5c2e7',
    brightCyan: '#94e2d5',
    brightWhite: '#a6adc8'
  },

  dracula: {
    name: 'Dracula',
    background: '#282a36',
    foreground: '#f8f8f2',
    cursor: '#f8f8f2',
    cursorAccent: '#282a36',
    selectionBackground: '#44475a',
    selectionForeground: '#f8f8f2',
    black: '#21222c',
    red: '#ff5555',
    green: '#50fa7b',
    yellow: '#f1fa8c',
    blue: '#bd93f9',
    magenta: '#ff79c6',
    cyan: '#8be9fd',
    white: '#f8f8f2',
    brightBlack: '#6272a4',
    brightRed: '#ff6e6e',
    brightGreen: '#69ff94',
    brightYellow: '#ffffa5',
    brightBlue: '#d6acff',
    brightMagenta: '#ff92df',
    brightCyan: '#a4ffff',
    brightWhite: '#ffffff'
  },

  highContrastDark: {
    name: 'High Contrast (Dark)',
    background: '#000000',
    foreground: '#ffffff',
    cursor: '#00ffff',
    cursorAccent: '#000000',
    selectionBackground: '#333333',
    selectionForeground: '#ffffff',
    black: '#000000',
    red: '#ff0000',
    green: '#00ff00',
    yellow: '#ffff00',
    blue: '#00aaff',
    magenta: '#ff00ff',
    cyan: '#00ffff',
    white: '#ffffff',
    brightBlack: '#555555',
    brightRed: '#ff5555',
    brightGreen: '#55ff55',
    brightYellow: '#ffff55',
    brightBlue: '#55aaff',
    brightMagenta: '#ff55ff',
    brightCyan: '#55ffff',
    brightWhite: '#ffffff'
  },

  githubDark: {
    name: 'GitHub Dark',
    background: '#0d1117',
    foreground: '#c9d1d9',
    cursor: '#58a6ff',
    cursorAccent: '#0d1117',
    selectionBackground: '#1f6feb',
    selectionForeground: '#f0f6fc',
    black: '#161b22',
    red: '#ff7b72',
    green: '#3fb950',
    yellow: '#d29922',
    blue: '#58a6ff',
    magenta: '#bc8cff',
    cyan: '#39c5cf',
    white: '#b1bac4',
    brightBlack: '#6e7681',
    brightRed: '#ffa198',
    brightGreen: '#56d364',
    brightYellow: '#e3b341',
    brightBlue: '#79c0ff',
    brightMagenta: '#d2a8ff',
    brightCyan: '#56d4dd',
    brightWhite: '#f0f6fc'
  }
};

// Aliases for user convenience
THEMES['deep-ocean'] = THEMES.deepOcean;
THEMES['tokyo-night'] = THEMES.tokyoNight;
THEMES['catppuccin'] = THEMES.catppuccinMocha;
THEMES['gruvbox-dark'] = THEMES.gruvbox;

// ============================================================================
// 5. ANSI Parser & HTML Renderer
// ============================================================================

/**
 * Token produced by ANSI escape parser.
 */
export interface AnsiToken {
  text: string;
  bold?: boolean;
  dim?: boolean;
  italic?: boolean;
  underline?: boolean;
  inverse?: boolean;
  strikethrough?: boolean;
  color?: string;
  bgColor?: string;
}

/**
 * ANSI Escape Sequence Parser, Visible Width Calculator & HTML Renderer.
 */
export class AnsiParser {
  private static readonly ANSI_REGEX = /\x1b\[([0-9;]*)([a-zA-Z])/g;
  private static readonly OSC_REGEX = /\x1b\].*?(?:\x07|\x1b\\)/g;

  /**
   * Strips all ANSI escape sequences, OSC codes, and terminal controls from text.
   */
  static stripAnsi(text: string): string {
    return text
      .replace(AnsiParser.ANSI_REGEX, '')
      .replace(AnsiParser.OSC_REGEX, '')
      .replace(/\x1b[=>]/g, '');
  }

  /**
   * Computes visible character width of string ignoring ANSI escape codes.
   */
  static visibleLength(text: string): number {
    return AnsiParser.stripAnsi(text).length;
  }

  /**
   * Converts a Hex color code (#RRGGBB) to 24-bit TrueColor ANSI sequence.
   */
  static hexToAnsiRgb(hex: string, background = false): string {
    const clean = hex.replace('#', '');
    if (clean.length !== 6) return '';
    const r = parseInt(clean.substring(0, 2), 16);
    const g = parseInt(clean.substring(2, 4), 16);
    const b = parseInt(clean.substring(4, 6), 16);
    return background ? ANSI.bgRgb(r, g, b) : ANSI.rgb(r, g, b);
  }

  /**
   * Parses an ANSI-encoded string into structured style tokens.
   */
  static parse(text: string, theme: TerminalTheme = THEMES.tokyoNight): AnsiToken[] {
    const tokens: AnsiToken[] = [];
    const colorMap: Record<number, string> = {
      30: theme.black,
      31: theme.red,
      32: theme.green,
      33: theme.yellow,
      34: theme.blue,
      35: theme.magenta,
      36: theme.cyan,
      37: theme.white,
      90: theme.brightBlack,
      91: theme.brightRed,
      92: theme.brightGreen,
      93: theme.brightYellow,
      94: theme.brightBlue,
      95: theme.brightMagenta,
      96: theme.brightCyan,
      97: theme.brightWhite
    };

    const bgColorMap: Record<number, string> = {
      40: theme.black,
      41: theme.red,
      42: theme.green,
      43: theme.yellow,
      44: theme.blue,
      45: theme.magenta,
      46: theme.cyan,
      47: theme.white
    };

    let currentBold = false;
    let currentDim = false;
    let currentItalic = false;
    let currentUnderline = false;
    let currentInverse = false;
    let currentStrikethrough = false;
    let currentColor: string | undefined = undefined;
    let currentBgColor: string | undefined = undefined;

    let lastIndex = 0;
    const regex = /\x1b\[([0-9;]*)([a-zA-Z])/g;
    let match: RegExpExecArray | null;

    while ((match = regex.exec(text)) !== null) {
      if (match.index > lastIndex) {
        tokens.push({
          text: text.slice(lastIndex, match.index),
          bold: currentBold,
          dim: currentDim,
          italic: currentItalic,
          underline: currentUnderline,
          inverse: currentInverse,
          strikethrough: currentStrikethrough,
          color: currentColor,
          bgColor: currentBgColor
        });
      }

      const params = match[1] ? match[1].split(';').map(n => parseInt(n, 10)) : [0];
      const command = match[2];

      if (command === 'm') {
        let i = 0;
        while (i < params.length) {
          const code = isNaN(params[i]) ? 0 : params[i];
          if (code === 0) {
            currentBold = false;
            currentDim = false;
            currentItalic = false;
            currentUnderline = false;
            currentInverse = false;
            currentStrikethrough = false;
            currentColor = undefined;
            currentBgColor = undefined;
          } else if (code === 1) {
            currentBold = true;
          } else if (code === 2) {
            currentDim = true;
          } else if (code === 3) {
            currentItalic = true;
          } else if (code === 4) {
            currentUnderline = true;
          } else if (code === 7) {
            currentInverse = true;
          } else if (code === 9) {
            currentStrikethrough = true;
          } else if (code === 22) {
            currentBold = false;
            currentDim = false;
          } else if (code === 23) {
            currentItalic = false;
          } else if (code === 24) {
            currentUnderline = false;
          } else if (code === 27) {
            currentInverse = false;
          } else if (code === 29) {
            currentStrikethrough = false;
          } else if (code === 39) {
            currentColor = undefined;
          } else if (code === 49) {
            currentBgColor = undefined;
          } else if (code >= 30 && code <= 37) {
            currentColor = colorMap[code];
          } else if (code >= 90 && code <= 97) {
            currentColor = colorMap[code];
          } else if (code >= 40 && code <= 47) {
            currentBgColor = bgColorMap[code];
          } else if (code === 38 && params[i + 1] === 2 && i + 4 < params.length) {
            // TrueColor 24-bit FG: \x1b[38;2;R;G;Bm
            const r = params[i + 2];
            const g = params[i + 3];
            const b = params[i + 4];
            currentColor = `rgb(${r},${g},${b})`;
            i += 4;
          } else if (code === 48 && params[i + 1] === 2 && i + 4 < params.length) {
            // TrueColor 24-bit BG: \x1b[48;2;R;G;Bm
            const r = params[i + 2];
            const g = params[i + 3];
            const b = params[i + 4];
            currentBgColor = `rgb(${r},${g},${b})`;
            i += 4;
          }
          i++;
        }
      }

      lastIndex = regex.lastIndex;
    }

    if (lastIndex < text.length) {
      tokens.push({
        text: text.slice(lastIndex),
        bold: currentBold,
        dim: currentDim,
        italic: currentItalic,
        underline: currentUnderline,
        inverse: currentInverse,
        strikethrough: currentStrikethrough,
        color: currentColor,
        bgColor: currentBgColor
      });
    }

    return tokens;
  }

  /**
   * Converts an ANSI-formatted string to sanitized HTML spans styled for Web UI rendering.
   */
  static toHtml(text: string, theme: TerminalTheme | string = THEMES.tokyoNight): string {
    const activeTheme = typeof theme === 'string' ? (THEMES[theme] || THEMES.tokyoNight) : theme;
    const tokens = AnsiParser.parse(text, activeTheme);

    return tokens.map(token => {
      if (!token.text) return '';
      const styles: string[] = [];

      if (token.bold) styles.push('font-weight:bold');
      if (token.dim) styles.push('opacity:0.65');
      if (token.italic) styles.push('font-style:italic');
      if (token.underline) styles.push('text-decoration:underline');
      if (token.strikethrough) styles.push('text-decoration:line-through');

      let fg = token.color || activeTheme.foreground;
      let bg = token.bgColor;

      if (token.inverse) {
        const temp = fg;
        fg = bg || activeTheme.background;
        bg = temp;
      }

      if (fg) styles.push(`color:${fg}`);
      if (bg) styles.push(`background-color:${bg}`);

      const escapedText = token.text
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#039;');

      if (styles.length === 0) {
        return escapedText;
      }

      return `<span style="${styles.join(';')}">${escapedText}</span>`;
    }).join('');
  }

  /**
   * Wraps text containing ANSI codes at specified character width.
   */
  static wrapAnsi(text: string, columns: number): string {
    if (columns <= 0) return text;
    const lines = text.split('\n');
    const result: string[] = [];

    for (const line of lines) {
      if (AnsiParser.visibleLength(line) <= columns) {
        result.push(line);
        continue;
      }

      let currentLine = '';
      let currentLen = 0;
      const tokens = AnsiParser.parse(line);

      for (const token of tokens) {
        const words = token.text.split(/(\s+)/);
        for (const word of words) {
          if (currentLen + word.length > columns && currentLen > 0) {
            result.push(currentLine);
            currentLine = '';
            currentLen = 0;
          }
          currentLine += word;
          currentLen += word.length;
        }
      }

      if (currentLine) {
        result.push(currentLine);
      }
    }

    return result.join('\n');
  }
}

// ============================================================================
// 6. Key Event Encoder & Utilities
// ============================================================================

export class KeyEncoder {
  /**
   * Strips all ANSI escape codes from string.
   */
  static stripAnsi(text: string): string {
    return AnsiParser.stripAnsi(text);
  }

  /**
   * Computes visible character width of string.
   */
  static visibleLength(text: string): number {
    return AnsiParser.visibleLength(text);
  }

  /**
   * Encodes a DOM KeyboardEvent into standard VT100/Xterm ANSI escape sequences.
   */
  static encodeKeyEvent(e: KeyboardEvent): string | null {
    const { key, ctrlKey, altKey, shiftKey, metaKey } = e;

    // Do not intercept browser copy/paste shortcuts
    if ((metaKey || ctrlKey) && (key === 'c' || key === 'v' || key === 'a') && typeof window !== 'undefined' && window.getSelection?.()?.toString()) {
      return null;
    }

    // Shift + Enter: Multiline newline sequence
    if (key === 'Enter' && shiftKey) {
      return '\x1b[13;2u';
    }

    // Ctrl + J: Linefeed
    if (ctrlKey && (key === 'j' || key === 'J')) {
      return '\n';
    }

    // Control Key Combinations (Ctrl+A .. Ctrl+Z)
    if (ctrlKey && !altKey && !metaKey && key.length === 1) {
      const code = key.toLowerCase().charCodeAt(0);
      if (code >= 97 && code <= 122) { // 'a' to 'z'
        return String.fromCharCode(code - 96);
      }
      if (key === '@' || key === '`') return '\x00';
      if (key === '[') return '\x1b';
      if (key === '\\') return '\x1c';
      if (key === ']') return '\x1d';
      if (key === '^') return '\x1e';
      if (key === '_') return '\x1f';
      if (key === '?') return '\x7f';
    }

    // Alt + Key combinations (Escape prefix)
    if (altKey && !ctrlKey && !metaKey && key.length === 1) {
      return '\x1b' + key;
    }

    // Special Named Keys
    switch (key) {
      case 'Enter':
        return '\r';
      case 'Backspace':
        return altKey ? '\x1b\x7f' : '\x7f';
      case 'Tab':
        return shiftKey ? '\x1b[Z' : '\t';
      case 'Escape':
        return '\x1b';

      // Cursor Navigation
      case 'ArrowUp':
        if (ctrlKey || altKey) return '\x1b[1;5A';
        if (shiftKey) return '\x1b[1;2A';
        return '\x1b[A';

      case 'ArrowDown':
        if (ctrlKey || altKey) return '\x1b[1;5B';
        if (shiftKey) return '\x1b[1;2B';
        return '\x1b[B';

      case 'ArrowRight':
        if (ctrlKey || altKey) return '\x1b[1;5C'; // Word right
        if (shiftKey) return '\x1b[1;2C';
        return '\x1b[C';

      case 'ArrowLeft':
        if (ctrlKey || altKey) return '\x1b[1;5D'; // Word left
        if (shiftKey) return '\x1b[1;2D';
        return '\x1b[D';

      case 'Home':
        return ctrlKey ? '\x1b[1;5H' : '\x1b[H';
      case 'End':
        return ctrlKey ? '\x1b[1;5F' : '\x1b[F';
      case 'PageUp':
        return '\x1b[5~';
      case 'PageDown':
        return '\x1b[6~';
      case 'Insert':
        return '\x1b[2~';
      case 'Delete':
        return ctrlKey ? '\x1b[3;5~' : '\x1b[3~';

      // Function Keys
      case 'F1': return '\x1bOP';
      case 'F2': return '\x1bOQ';
      case 'F3': return '\x1bOR';
      case 'F4': return '\x1bOS';
      case 'F5': return '\x1b[15~';
      case 'F6': return '\x1b[17~';
      case 'F7': return '\x1b[18~';
      case 'F8': return '\x1b[19~';
      case 'F9': return '\x1b[20~';
      case 'F10': return '\x1b[21~';
      case 'F11': return '\x1b[23~';
      case 'F12': return '\x1b[24~';

      default:
        // Regular printable character
        if (key.length === 1 && !ctrlKey && !metaKey) {
          return key;
        }
        return null;
    }
  }
}

// ============================================================================
// 7. Formatting & Box Drawing Engine
// ============================================================================

export class AnsiFormatter {
  /**
   * Formats a titled unicode box around content.
   */
  static box(title: string, content: string, color = ANSI.purple): string {
    const lines = content.split('\n');
    const width = Math.max(title.length + 8, ...lines.map(l => AnsiParser.visibleLength(l))) + 4;
    const topBorder = `┌─ ${title} ` + '─'.repeat(Math.max(0, width - title.length - 5)) + '┐';
    const bottomBorder = '└' + '─'.repeat(width - 2) + '┘';

    const formattedLines = lines.map(line => {
      const padding = ' '.repeat(Math.max(0, width - 4 - AnsiParser.visibleLength(line)));
      return `${color}│${ANSI.reset} ${line}${padding} ${color}│${ANSI.reset}`;
    });

    return [
      `${color}${topBorder}${ANSI.reset}`,
      ...formattedLines,
      `${color}${bottomBorder}${ANSI.reset}`
    ].join('\r\n');
  }

  /**
   * Formats a horizontal divider with optional title.
   */
  static divider(title = '', width = 72, color = ANSI.darkGray): string {
    if (!title) {
      return `${color}${'─'.repeat(width)}${ANSI.reset}`;
    }
    const leftWidth = 3;
    const rightWidth = Math.max(0, width - leftWidth - AnsiParser.visibleLength(title) - 2);
    return `${color}${'─'.repeat(leftWidth)} ${ANSI.reset}${title} ${color}${'─'.repeat(rightWidth)}${ANSI.reset}`;
  }

  /**
   * Formats an ASCII/ANSI progress bar.
   */
  static progressBar(current: number, total: number, width = 24, color = ANSI.emerald): string {
    const ratio = Math.max(0, Math.min(1, total > 0 ? current / total : 0));
    const filled = Math.round(ratio * width);
    const empty = width - filled;
    const percentage = (ratio * 100).toFixed(1);

    return `${color}[${'█'.repeat(filled)}${ANSI.darkGray}${'░'.repeat(empty)}${color}] ${percentage}% (${current}/${total})${ANSI.reset}`;
  }

  /**
   * Formats a tool call invocation box.
   */
  static formatToolStart(name: string, input?: Record<string, unknown>): string {
    const header = `${ANSI.purple}┌── ⚙ Tool Call: ${ANSI.neonCyan}${name}${ANSI.reset}`;
    if (!input || Object.keys(input).length === 0) {
      return header;
    }
    const inputJson = JSON.stringify(input, null, 2).split('\n');
    const body = inputJson.map(line => `${ANSI.purple}│${ANSI.slate}  ${line}${ANSI.reset}`).join('\r\n');
    return `${header}\r\n${body}`;
  }

  /**
   * Formats a tool call result.
   */
  static formatToolResult(output: string, success = true, durationMs?: number): string {
    const statusColor = success ? ANSI.emerald : ANSI.rose;
    const statusSymbol = success ? '✔' : '✖';
    const durationStr = durationMs !== undefined ? ` ${ANSI.slate}(${durationMs}ms)${ANSI.reset}` : '';
    const footer = `${ANSI.purple}└── ${statusColor}${statusSymbol} Result:${ANSI.reset}${durationStr}`;

    const lines = output.trim().split('\n');
    if (lines.length === 1 && lines[0].length < 80) {
      return `${footer} ${ANSI.slate}${lines[0]}${ANSI.reset}`;
    }

    const preview = lines.slice(0, 15).map(l => `${ANSI.purple}│${ANSI.reset}  ${l}`).join('\r\n');
    const truncated = lines.length > 15 ? `\r\n${ANSI.purple}│${ANSI.slate}  ... [${lines.length - 15} lines elided]${ANSI.reset}` : '';
    return `${preview}${truncated}\r\n${footer}`;
  }

  /**
   * Formats advisor critique output.
   */
  static formatAdvisor(advisor: string, approved: boolean, critique: string): string {
    const icon = approved ? `${ANSI.emerald}🛡 [${advisor}] PASSED:${ANSI.reset}` : `${ANSI.amber}⚠️ [${advisor}] CRITIQUE:${ANSI.reset}`;
    return `${icon} ${critique}`;
  }
}

// ============================================================================
// 8. Inline Spinner Animation Helper
// ============================================================================

export const SPINNER_FRAMES: Record<SpinnerStyle, string[]> = {
  dots: ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'],
  line: ['-', '\\', '|', '/'],
  pulse: ['█', '▓', '▒', '░', '▒', '▓'],
  arrows: ['▹▹▹▹▹', '▸▹▹▹▹', '▹▸▹▹▹', '▹▹▸▹▹', '▹▹▹▸▹', '▹▹▹▹▸'],
  bounce: ['⠁', '⠂', '⠄', '⡀', '⢀', '⠠', '⠐', '⠈'],
  moon: ['🌑', '🌒', '🌓', '🌔', '🌕', '🌖', '🌗', '🌘']
};

/**
 * High-performance inline terminal spinner animation.
 */
export class TerminalSpinner implements IDisposable {
  private _terminal: XtermInstance | null = null;
  private _text: string;
  private _frames: string[];
  private _intervalMs: number;
  private _color: string;
  private _prefix: string;
  private _timer: TimerId | null = null;
  private _frameIndex = 0;
  private _isActive = false;

  constructor(
    terminal: XtermInstance | null,
    text = 'Processing...',
    options: SpinnerOptions = {}
  ) {
    this._terminal = terminal;
    this._text = text;
    const style = options.style || 'dots';
    this._frames = SPINNER_FRAMES[style] || SPINNER_FRAMES.dots;
    this._intervalMs = options.interval || 80;
    this._color = options.color || ANSI.neonCyan;
    this._prefix = options.prefix || '';
  }

  get isActive(): boolean {
    return this._isActive;
  }

  setTerminal(terminal: XtermInstance | null): void {
    this._terminal = terminal;
  }

  /**
   * Starts the inline spinner animation.
   */
  start(text?: string): this {
    if (text) this._text = text;
    if (this._isActive) return this;

    this._isActive = true;
    this._frameIndex = 0;
    this._render();

    this._timer = setInterval(() => {
      this._frameIndex = (this._frameIndex + 1) % this._frames.length;
      this._render();
    }, this._intervalMs);

    return this;
  }

  /**
   * Updates spinner status text without stopping the animation.
   */
  setText(text: string): this {
    this._text = text;
    if (this._isActive) {
      this._render();
    }
    return this;
  }

  /**
   * Stops animation and displays a success checkmark.
   */
  succeed(text?: string): void {
    this.stop();
    const msg = text || this._text;
    this._writeLine(`${ANSI.emerald}✔${ANSI.reset} ${this._prefix}${msg}`);
  }

  /**
   * Stops animation and displays a failure cross.
   */
  fail(text?: string): void {
    this.stop();
    const msg = text || this._text;
    this._writeLine(`${ANSI.rose}✖${ANSI.reset} ${this._prefix}${msg}`);
  }

  /**
   * Stops animation and displays a warning icon.
   */
  warn(text?: string): void {
    this.stop();
    const msg = text || this._text;
    this._writeLine(`${ANSI.amber}⚠️${ANSI.reset} ${this._prefix}${msg}`);
  }

  /**
   * Stops animation and displays an informational icon.
   */
  info(text?: string): void {
    this.stop();
    const msg = text || this._text;
    this._writeLine(`${ANSI.neonCyan}ℹ${ANSI.reset} ${this._prefix}${msg}`);
  }

  /**
   * Stops animation and clears the spinner line.
   */
  stop(clear = false): void {
    if (this._timer) {
      clearInterval(this._timer as number);
      this._timer = null;
    }
    this._isActive = false;

    if (clear && this._terminal) {
      this._terminal.write('\r\x1b[K');
    }
  }

  dispose(): void {
    this.stop(true);
    this._terminal = null;
  }

  private _render(): void {
    if (!this._terminal || !this._isActive) return;
    const frame = this._frames[this._frameIndex];
    const line = `\r\x1b[K${this._color}${frame}${ANSI.reset} ${this._prefix}${this._text}`;
    this._terminal.write(line);
  }

  private _writeLine(line: string): void {
    if (!this._terminal) return;
    this._terminal.write(`\r\x1b[K${line}\r\n`);
  }
}

// ============================================================================
// 9. Backend Implementations (WASM, WebSocket, Mock)
// ============================================================================

/**
 * WebAssembly Agent Backend connecting to WasmFusionAgent.
 */
export class WasmAgentBackend implements AgentBackend {
  readonly id = 'wasm';
  readonly name = 'Fusion WebAssembly Agent';
  private _agent: WasmFusionAgentInstance | null = null;

  constructor(wasmAgent?: WasmFusionAgentInstance) {
    if (wasmAgent) {
      this._agent = wasmAgent;
    }
  }

  get isConnected(): boolean {
    return this._agent !== null;
  }

  setAgent(agent: WasmFusionAgentInstance | null): void {
    this._agent = agent;
  }

  getAgent(): WasmFusionAgentInstance | null {
    return this._agent;
  }

  async promptTurn(prompt: string, onEvent: AgentEventCallback, signal?: AbortSignal): Promise<string> {
    if (!this._agent) {
      throw new Error('WasmFusionAgent is not initialized. Pass a valid WasmFusionAgent instance or load WASM bundle first.');
    }

    if (signal?.aborted) {
      throw new Error('Turn was aborted prior to execution.');
    }

    const deferred = createDeferred<string>();

    const abortHandler = () => {
      deferred.reject(new Error('Turn aborted by user.'));
    };

    if (signal) {
      signal.addEventListener('abort', abortHandler, { once: true });
    }

    try {
      const callback = (rawEvent: unknown) => {
        let ev: AgentEvent;
        if (typeof rawEvent === 'string') {
          try {
            ev = JSON.parse(rawEvent) as AgentEvent;
          } catch {
            ev = { type: 'text_delta', delta: rawEvent };
          }
        } else if (rawEvent && typeof rawEvent === 'object' && 'type' in rawEvent) {
          ev = rawEvent as AgentEvent;
        } else {
          ev = { type: 'text_delta', delta: String(rawEvent ?? '') };
        }
        onEvent(ev);
      };

      const result = this._agent.prompt_turn(prompt, callback);
      if (result && typeof (result as Promise<string>).then === 'function') {
        (result as Promise<string>).then((response: string) => {
          if (signal) signal.removeEventListener('abort', abortHandler);
          deferred.resolve(response);
        }).catch((err: unknown) => {
          if (signal) signal.removeEventListener('abort', abortHandler);
          deferred.reject(err instanceof Error ? err : new Error(String(err)));
        });
      } else {
        if (signal) signal.removeEventListener('abort', abortHandler);
        deferred.resolve(String(result ?? ''));
      }
    } catch (err: unknown) {
      if (signal) signal.removeEventListener('abort', abortHandler);
      deferred.reject(err instanceof Error ? err : new Error(String(err)));
    }

    return deferred.promise;
  }

  checkpoint(): string {
    if (this._agent && typeof this._agent.checkpoint === 'function') {
      return this._agent.checkpoint();
    }
    throw new Error('Checkpoint not supported on active WASM agent.');
  }

  restore(checkpointJson: string): void {
    if (this._agent && typeof this._agent.restore === 'function') {
      this._agent.restore(checkpointJson);
      return;
    }
    throw new Error('Restore not supported on active WASM agent.');
  }
}

/**
 * WebSocket Agent Backend connecting to Fusion ACP Server (JSON-RPC 2.0).
 */
export class WebSocketAgentBackend implements AgentBackend {
  readonly id = 'websocket';
  readonly name = 'Fusion WebSocket ACP Server';
  private _wsUrl: string;
  private _ws: WebSocket | null = null;
  private _connected = false;
  private _pendingRequests = new Map<number | string, Deferred<unknown>>();
  private _activeStreamingCallback: AgentEventCallback | null = null;
  private _reqId = 1;
  private _reconnectTimer: TimerId | null = null;
  private _autoReconnect = true;

  constructor(wsUrl = 'ws://127.0.0.1:3000/acp', autoReconnect = true) {
    this._wsUrl = wsUrl;
    this._autoReconnect = autoReconnect;
  }

  get isConnected(): boolean {
    return this._connected;
  }

  get url(): string {
    return this._wsUrl;
  }

  setUrl(url: string): void {
    this._wsUrl = url;
    if (this._connected) {
      this.disconnect().then(() => this.connect());
    }
  }

  async connect(): Promise<void> {
    if (this._ws && this._connected) return;

    const deferred = createDeferred<void>();

    try {
      this._ws = new WebSocket(this._wsUrl);

      this._ws.onopen = () => {
        this._connected = true;
        // Send initialize handshake
        this._sendRequest('initialize', {
          protocolVersion: 1,
          clientCapabilities: { terminal: true, session: {} },
          clientInfo: { name: 'fusion-xterm-adapter', version: '0.3.0' }
        }).catch(() => {});
        deferred.resolve();
      };

      this._ws.onmessage = (event: MessageEvent) => {
        this._handleMessage(typeof event.data === 'string' ? event.data : String(event.data));
      };

      this._ws.onerror = () => {
        if (!this._connected) {
          deferred.reject(new Error(`WebSocket connection error to ${this._wsUrl}`));
        }
      };

      this._ws.onclose = () => {
        this._connected = false;
        if (this._autoReconnect && !this._reconnectTimer) {
          this._reconnectTimer = setTimeout(() => {
            this._reconnectTimer = null;
            this.connect().catch(() => {});
          }, 3000);
        }
      };
    } catch (err: unknown) {
      deferred.reject(err instanceof Error ? err : new Error(String(err)));
    }

    return deferred.promise;
  }

  async disconnect(): Promise<void> {
    this._autoReconnect = false;
    if (this._reconnectTimer) {
      clearTimeout(this._reconnectTimer as number);
      this._reconnectTimer = null;
    }
    if (this._ws) {
      this._ws.close();
      this._ws = null;
    }
    this._connected = false;
  }

  async promptTurn(prompt: string, onEvent: AgentEventCallback, signal?: AbortSignal): Promise<string> {
    if (!this._connected || !this._ws) {
      await this.connect();
    }

    this._activeStreamingCallback = onEvent;

    if (signal) {
      signal.addEventListener('abort', () => {
        this.cancelTurn();
      }, { once: true });
    }

    const id = ++this._reqId;
    const deferred = createDeferred<string>();

    this._pendingRequests.set(id, deferred as Deferred<unknown>);

    const request = {
      jsonrpc: '2.0',
      id: id,
      method: 'session/prompt',
      params: { prompt }
    };

    try {
      this._ws!.send(JSON.stringify(request));
    } catch (err: unknown) {
      this._pendingRequests.delete(id);
      this._activeStreamingCallback = null;
      deferred.reject(err instanceof Error ? err : new Error(String(err)));
    }

    return deferred.promise;
  }

  async cancelTurn(): Promise<void> {
    if (this._ws && this._connected) {
      try {
        this._ws.send(JSON.stringify({
          jsonrpc: '2.0',
          method: 'session/cancel',
          params: {}
        }));
      } catch {}
    }
    this._activeStreamingCallback = null;
  }

  resizeTerminal(cols: number, rows: number): void {
    if (this._ws && this._connected) {
      try {
        this._ws.send(JSON.stringify({
          jsonrpc: '2.0',
          method: 'terminal/resize',
          params: { cols, rows }
        }));
      } catch {}
    }
  }

  private _sendRequest(method: string, params: Record<string, unknown>): Promise<unknown> {
    const id = ++this._reqId;
    const deferred = createDeferred<unknown>();
    this._pendingRequests.set(id, deferred);
    try {
      this._ws?.send(JSON.stringify({
        jsonrpc: '2.0',
        id,
        method,
        params
      }));
    } catch (err: unknown) {
      this._pendingRequests.delete(id);
      deferred.reject(err instanceof Error ? err : new Error(String(err)));
    }
    return deferred.promise;
  }

  private _handleMessage(dataStr: string): void {
    try {
      const msg = JSON.parse(dataStr) as Record<string, unknown>;

      // JSON-RPC Response matching request ID
      const msgId = msg.id as number | string | undefined;
      if (msgId !== undefined && this._pendingRequests.has(msgId)) {
        const pending = this._pendingRequests.get(msgId)!;
        this._pendingRequests.delete(msgId);
        this._activeStreamingCallback = null;

        if (msg.error && typeof msg.error === 'object') {
          const errObj = msg.error as Record<string, unknown>;
          pending.reject(new Error(typeof errObj.message === 'string' ? errObj.message : `RPC Error: ${String(errObj.code)}`));
        } else {
          const resObj = msg.result as Record<string, unknown> | undefined;
          pending.resolve(typeof resObj?.response === 'string' ? resObj.response : (msg.result ?? ''));
        }
        return;
      }

      // JSON-RPC Streaming Notifications
      if (this._activeStreamingCallback) {
        const method = msg.method as string | undefined;
        const params = (msg.params ?? {}) as Record<string, unknown>;

        if (method === 'turn/delta' || method === 'session/delta') {
          this._activeStreamingCallback({
            type: 'text_delta',
            delta: typeof params.delta === 'string' ? params.delta : (typeof params.text === 'string' ? params.text : '')
          });
        } else if (method === 'thinking/delta') {
          this._activeStreamingCallback({
            type: 'thinking_delta',
            delta: typeof params.delta === 'string' ? params.delta : ''
          });
        } else if (method === 'tool/call' || method === 'tool/started') {
          this._activeStreamingCallback({
            type: 'tool_started',
            id: typeof params.id === 'string' ? params.id : String(Date.now()),
            name: typeof params.name === 'string' ? params.name : (typeof params.tool === 'string' ? params.tool : 'tool'),
            args: (params.input ?? params.args ?? {}) as Record<string, unknown>
          });
        } else if (method === 'tool/result' || method === 'tool/finished') {
          this._activeStreamingCallback({
            type: 'tool_finished',
            id: typeof params.id === 'string' ? params.id : '',
            name: typeof params.name === 'string' ? params.name : (typeof params.tool === 'string' ? params.tool : ''),
            success: params.success !== false,
            output: typeof params.output === 'string' ? params.output : (typeof params.result === 'string' ? params.result : ''),
            duration_ms: typeof params.duration_ms === 'number' ? params.duration_ms : undefined
          });
        } else if (method === 'turn/finished') {
          this._activeStreamingCallback({
            type: 'finished',
            usage: params.usage as { prompt_tokens?: number; completion_tokens?: number; total_tokens?: number } | undefined
          });
        } else if (method === 'advisor/critique') {
          this._activeStreamingCallback({
            type: 'advisor_critique',
            advisor: typeof params.advisor === 'string' ? params.advisor : 'SecurityAdvisor',
            approved: params.approved !== false,
            critique: typeof params.critique === 'string' ? params.critique : ''
          });
        }
      }
    } catch {
      // Non-JSON raw text payload
      if (this._activeStreamingCallback) {
        this._activeStreamingCallback({
          type: 'text_delta',
          delta: dataStr
        });
      }
    }
  }
}

/**
 * In-Memory Mock Agent Backend for offline UI testing and zero-config browser previews.
 */
export class MockAgentBackend implements AgentBackend {
  readonly id = 'mock';
  readonly name = 'Fusion Simulated Backend';
  readonly isConnected = true;

  async promptTurn(prompt: string, onEvent: AgentEventCallback, signal?: AbortSignal): Promise<string> {
    const deferred = createDeferred<string>();
    let isCancelled = false;

    if (signal) {
      signal.addEventListener('abort', () => {
        isCancelled = true;
        deferred.reject(new Error('Turn aborted.'));
      }, { once: true });
    }

    onEvent({ type: 'status', message: 'Analyzing workspace...' });

    setTimeout(() => {
      if (isCancelled) return;

      // Simulate Tool Invocation
      onEvent({
        type: 'tool_started',
        id: 'tool_1',
        name: 'grep',
        args: { pattern: prompt.slice(0, 15), path: 'src/' }
      });

      setTimeout(() => {
        if (isCancelled) return;

        onEvent({
          type: 'tool_finished',
          id: 'tool_1',
          name: 'grep',
          success: true,
          output: 'src/lib.rs:42: pub struct FusionAgent\nsrc/main.rs:18: async fn main()',
          duration_ms: 12
        });

        // Simulate Streaming Output
        const responseText = `I analyzed your request: "${prompt}".\n\nFusion v2 executed the relevant tools in 12ms with zero allocations.\nCode synthesis completed successfully.`;
        const words = responseText.split(' ');
        let index = 0;

        const interval = setInterval(() => {
          if (isCancelled) {
            clearInterval(interval);
            return;
          }

          if (index < words.length) {
            onEvent({ type: 'text_delta', delta: (index === 0 ? '' : ' ') + words[index] });
            index++;
          } else {
            clearInterval(interval);
            onEvent({
              type: 'finished',
              usage: { prompt_tokens: 120, completion_tokens: 45, total_tokens: 165 }
            });
            deferred.resolve(responseText);
          }
        }, 35);
      }, 300);
    }, 200);

    return deferred.promise;
  }
}

// ============================================================================
// 10. XtermAdapter Main Class
// ============================================================================

/**
 * Official Fusion v2 xterm.js Adapter.
 *
 * Provides bidirectional interactive terminal control, key event encoding,
 * smart line editing, paste sanitization, history navigation, ANSI box drawing,
 * animated status spinners, and agent streaming.
 */
export class XtermAdapter implements IDisposable {
  private _terminal: XtermInstance | null = null;
  private _container: HTMLElement | null = null;
  private _fitAddon: FitAddonInstance | null = null;
  private _backend: AgentBackend;
  private _options: XtermAdapterOptions;

  // Terminal Line Editor State
  private _inputBuffer = '';
  private _cursorPos = 0;
  private _history: string[] = [];
  private _historyIndex = -1;
  private _promptStr: string | (() => string);
  private _multilinePromptStr: string | (() => string);
  private _mode: 'interactive' | 'raw' | 'hybrid';
  private _isStreaming = false;
  private _abortController: AbortController | null = null;
  private _activeModel: string;
  private _activeTheme: TerminalTheme;

  // Multiline Editor State
  private _isBracketedPaste = false;
  private _pasteBuffer = '';

  // Spinners
  private _activeSpinner: TerminalSpinner | null = null;

  // Slash Command Registry
  private _commands = new Map<string, SlashCommand>();

  // Token & Cost Statistics
  private _stats: SessionStats = {
    promptTokens: 0,
    completionTokens: 0,
    totalTokens: 0,
    totalTurns: 0,
    estimatedCost: 0
  };

  // Disposables
  private _listeners: Array<IDisposable> = [];
  private _resizeObserver: ResizeObserver | null = null;
  private _windowResizeHandler: (() => void) | null = null;

  constructor(options: XtermAdapterOptions = {}) {
    this._options = options;
    this._backend = options.backend || new MockAgentBackend();
    this._promptStr = options.prompt || `${ANSI.purple}⚡ fusion${ANSI.reset} ${ANSI.neonCyan}❯${ANSI.reset} `;
    this._multilinePromptStr = options.multilinePrompt || `${ANSI.slate}  ... ${ANSI.reset}`;
    this._mode = options.mode || 'interactive';
    this._activeModel = options.model || 'anthropic/claude-3-5-sonnet';

    // Resolve Initial Theme
    if (typeof options.theme === 'string') {
      this._activeTheme = THEMES[options.theme] || THEMES.tokyoNight;
    } else if (options.theme) {
      this._activeTheme = {
        ...THEMES.tokyoNight,
        ...options.theme
      };
    } else {
      this._activeTheme = THEMES.tokyoNight;
    }

    // Load persisted history if key specified
    if (options.historyStorageKey && typeof localStorage !== 'undefined') {
      try {
        const saved = localStorage.getItem(options.historyStorageKey);
        if (saved) {
          this._history = JSON.parse(saved) as string[];
        }
      } catch {}
    }

    // Register Default Slash Commands
    if (options.enableSlashCommands !== false) {
      this._registerDefaultCommands();
    }

    // Register Custom Slash Commands
    if (options.customCommands) {
      for (const cmd of options.customCommands) {
        this.registerCommand(cmd);
      }
    }

    // Attach if terminal or container is supplied
    if (options.terminal) {
      const containerEl = options.container
        ? (typeof options.container === 'string' ? (typeof document !== 'undefined' ? (document.querySelector(options.container) as HTMLElement | null) : null) : options.container)
        : undefined;
      this.attach(options.terminal, containerEl || undefined);
    }
  }

  // --------------------------------------------------------------------------
  // Lifecycle & Attachment
  // --------------------------------------------------------------------------

  /**
   * Attaches adapter to an xterm.js instance and DOM container.
   */
  attach(terminal: XtermInstance, container?: HTMLElement): this {
    this.detach();

    this._terminal = terminal;
    this._container = container || (terminal.element ? (terminal.element.parentElement || terminal.element) : null);

    if (this._options.fitAddon) {
      this._fitAddon = this._options.fitAddon;
    }

    // Apply Active Theme
    this.applyTheme(this._activeTheme);

    // Register onData keystroke handler
    const onDataDisposable = terminal.onData((data: string) => {
      this._handleTerminalInput(data);
    });
    this._listeners.push(onDataDisposable);

    // Register onKey handler for specialized shortcuts (Shift+Enter, etc.)
    if (terminal.onKey) {
      const onKeyDisposable = terminal.onKey(({ key, domEvent }) => {
        this._handleTerminalKey(key, domEvent);
      });
      this._listeners.push(onKeyDisposable);
    }

    // Register onResize handler
    if (terminal.onResize) {
      const onResizeDisposable = terminal.onResize(({ cols, rows }) => {
        this._backend.resizeTerminal?.(cols, rows);
        this._options.onResize?.(cols, rows);
      });
      this._listeners.push(onResizeDisposable);
    }

    // Setup Auto-Resizing
    if (this._options.autoFit !== false) {
      this._setupAutoResize();
    }

    // Initial Fit & Banner
    this.fit();

    if (this._options.welcomeBanner !== false) {
      this.printBanner(typeof this._options.welcomeBanner === 'string' ? this._options.welcomeBanner : undefined);
    }

    if (this._mode === 'interactive') {
      this.printPrompt();
    }

    return this;
  }

  /**
   * Detaches adapter and tears down all event listeners.
   */
  detach(): void {
    if (this._activeSpinner) {
      this._activeSpinner.dispose();
      this._activeSpinner = null;
    }

    for (const listener of this._listeners) {
      try { listener.dispose(); } catch {}
    }
    this._listeners = [];

    if (this._resizeObserver) {
      this._resizeObserver.disconnect();
      this._resizeObserver = null;
    }

    if (this._windowResizeHandler && typeof window !== 'undefined') {
      window.removeEventListener('resize', this._windowResizeHandler);
      this._windowResizeHandler = null;
    }

    this._terminal = null;
  }

  /**
   * Completely disposes the adapter and closes backend connections.
   */
  dispose(): void {
    this.detach();
    this._backend.disconnect?.().catch(() => {});
  }

  // --------------------------------------------------------------------------
  // Theme Management
  // --------------------------------------------------------------------------

  /**
   * Returns list of built-in theme preset identifiers.
   */
  listThemes(): string[] {
    return Object.keys(THEMES).filter(k => !k.includes('-'));
  }

  /**
   * Gets the active terminal color theme.
   */
  getTheme(): TerminalTheme {
    return this._activeTheme;
  }

  /**
   * Sets and applies a theme by name preset or custom theme object.
   */
  setTheme(theme: string | TerminalTheme): void {
    if (typeof theme === 'string') {
      const selected = THEMES[theme] || THEMES[theme.toLowerCase()] || THEMES.tokyoNight;
      this._activeTheme = selected;
    } else {
      this._activeTheme = {
        ...THEMES.tokyoNight,
        ...theme
      };
    }
    this.applyTheme(this._activeTheme);
  }

  /**
   * Applies the theme settings directly to the attached xterm instance.
   */
  applyTheme(theme: TerminalTheme): void {
    this._activeTheme = theme;
    if (this._terminal) {
      if (this._terminal.options) {
        this._terminal.options.theme = { ...theme };
      }
    }
  }

  // --------------------------------------------------------------------------
  // Auto-Resize & Dimensions
  // --------------------------------------------------------------------------

  private _setupAutoResize(): void {
    // Window Resize Handler
    if (typeof window !== 'undefined') {
      let resizeTimeout: TimerId | null = null;
      this._windowResizeHandler = () => {
        if (resizeTimeout) clearTimeout(resizeTimeout as number);
        resizeTimeout = setTimeout(() => {
          this.fit();
        }, 60);
      };
      window.addEventListener('resize', this._windowResizeHandler);
    }

    // Container ResizeObserver
    if (this._container && typeof ResizeObserver !== 'undefined') {
      this._resizeObserver = new ResizeObserver(() => {
        this.fit();
      });
      this._resizeObserver.observe(this._container);
    }
  }

  /**
   * Refits the terminal to container dimensions.
   */
  fit(): void {
    if (!this._terminal) return;

    try {
      if (this._fitAddon) {
        this._fitAddon.fit();
      } else if (this._container && this._terminal.resize) {
        // Fallback calculation using container bounding box
        const rect = this._container.getBoundingClientRect();
        const charWidth = 9;  // Monospace approximation
        const charHeight = 18;
        const cols = Math.max(20, Math.floor(rect.width / charWidth));
        const rows = Math.max(5, Math.floor(rect.height / charHeight));
        if (cols !== this._terminal.cols || rows !== this._terminal.rows) {
          this._terminal.resize(cols, rows);
        }
      }
    } catch {}
  }

  /**
   * Explicitly sets terminal dimensions.
   */
  resize(cols: number, rows: number): void {
    if (this._terminal) {
      this._terminal.resize(cols, rows);
      this._backend.resizeTerminal?.(cols, rows);
    }
  }

  // --------------------------------------------------------------------------
  // Terminal Writing & Output
  // --------------------------------------------------------------------------

  /**
   * Writes raw string or bytes to terminal output.
   */
  write(data: string | Uint8Array): void {
    if (this._terminal) {
      this._terminal.write(data);
    }
  }

  /**
   * Writes line to terminal output with CRLF.
   */
  writeln(data = ''): void {
    if (this._terminal) {
      this._terminal.writeln(data);
    }
  }

  /**
   * Clears terminal screen and moves cursor to home position.
   */
  clear(): void {
    if (this._terminal) {
      this._terminal.clear();
    }
  }

  /**
   * Resets terminal state completely.
   */
  reset(): void {
    if (this._terminal) {
      this._terminal.reset();
      this._inputBuffer = '';
      this._cursorPos = 0;
      if (this._mode === 'interactive') {
        this.printPrompt();
      }
    }
  }

  /**
   * Prints the Fusion Welcome Banner.
   */
  printBanner(customMessage?: string): void {
    if (!this._terminal) return;

    this.writeln('');
    this.writeln(`${ANSI.purple}⚡ FUSION v0.3.0${ANSI.reset} ─────────────────────────────────────────────────────────────`);
    this.writeln(`${ANSI.dim}Lightweight, fast, cross-platform AI coding assistant harness${ANSI.reset}`);
    this.writeln(`${ANSI.slate}Backend: ${ANSI.emerald}${this._backend.name}${ANSI.slate} | Model: ${ANSI.neonCyan}${this._activeModel}${ANSI.slate} | Theme: ${ANSI.yellow}${this._activeTheme.name || 'Custom'}${ANSI.reset}`);
    if (customMessage) {
      this.writeln(`${ANSI.slate}${customMessage}${ANSI.reset}`);
    } else {
      this.writeln(`${ANSI.slate}Type ${ANSI.amber}/help${ANSI.slate} for commands or enter your prompt below.${ANSI.reset}`);
    }
    this.writeln(`${ANSI.purple}─────────────────────────────────────────────────────────────────────────────${ANSI.reset}`);
    this.writeln('');
  }

  /**
   * Prints the active prompt line.
   */
  printPrompt(): void {
    if (!this._terminal) return;
    const prompt = typeof this._promptStr === 'function' ? this._promptStr() : this._promptStr;
    this._terminal.write(prompt);
    this._inputBuffer = '';
    this._cursorPos = 0;
  }

  // --------------------------------------------------------------------------
  // Key Input Handling, Sanitization & Line Editor
  // --------------------------------------------------------------------------

  /**
   * Sanitizes pasted text to remove dangerous terminal escape injections.
   */
  sanitizePaste(raw: string): string {
    // Strip OSC codes (\x1b] ... \x07), cursor reporting queries, device status controls
    return raw
      .replace(/\x1b\][^\x07\x1b]*(\x07|\x1b\\)/g, '')
      .replace(/\x1b\[[0-9;]*[a-zA-Z]/g, '')
      .replace(/[\x00-\x08\x0b\x0c\x0e-\x1a\x1c-\x1f]/g, '');
  }

  private _handleTerminalKey(_key: string, ev: KeyboardEvent): void {
    // Handle Shift+Enter for multiline prompt input
    if (ev.key === 'Enter' && ev.shiftKey && this._options.allowMultiline !== false) {
      ev.preventDefault();
      this._insertNewline();
    }
  }

  private _handleTerminalInput(data: string): void {
    // Check if custom interceptor handled data
    if (this._options.onData && this._options.onData(data) === true) {
      return;
    }

    // Bracketed Paste Mode Handling
    if (data.includes('\x1b[200~')) {
      this._isBracketedPaste = true;
      this._pasteBuffer = '';
      const startIdx = data.indexOf('\x1b[200~') + 6;
      const endIdx = data.indexOf('\x1b[201~');
      if (endIdx !== -1) {
        const payload = data.substring(startIdx, endIdx);
        this._isBracketedPaste = false;
        this._insertPastedContent(payload);
        return;
      } else {
        this._pasteBuffer = data.substring(startIdx);
        return;
      }
    }

    if (this._isBracketedPaste) {
      const endIdx = data.indexOf('\x1b[201~');
      if (endIdx !== -1) {
        this._pasteBuffer += data.substring(0, endIdx);
        this._isBracketedPaste = false;
        this._insertPastedContent(this._pasteBuffer);
        this._pasteBuffer = '';
      } else {
        this._pasteBuffer += data;
      }
      return;
    }

    // Raw mode: Forward directly to backend
    if (this._mode === 'raw') {
      if (this._isStreaming && data === '\x03') { // Ctrl+C
        this.abortTurn();
      }
      return;
    }

    // Interactive Mode
    if (this._isStreaming) {
      // Ctrl+C during streaming aborts turn
      if (data === '\x03') {
        this.abortTurn();
      }
      return;
    }

    // Handle multi-character paste or raw chunk
    if (data.length > 2 && !data.startsWith('\x1b')) {
      this._insertPastedContent(data);
      return;
    }

    // Handle Enter / Shift+Enter / Ctrl+J
    if (data === '\x1b[13;2u' || data === '\x1b[27;2;13~') {
      // Shift+Enter escape sequence
      this._insertNewline();
      return;
    }

    if (data === '\n' && (this._options.allowMultiline !== false)) {
      // Ctrl+J (Linefeed)
      this._insertNewline();
      return;
    }

    if (data === '\r' || data === '\r\n') {
      this._handleEnter();
      return;
    }

    switch (data) {
      case '\x7f': // Backspace
      case '\x08':
        this._handleBackspace();
        break;

      case '\x1b\x7f': // Alt+Backspace (Delete word backwards)
      case '\x17':     // Ctrl+W
        this._deleteWordBackwards();
        break;

      case '\x1bd': // Alt+D (Delete word forwards)
        this._deleteWordForwards();
        break;

      case '\x03': // Ctrl+C
        this.writeln('^C');
        this._inputBuffer = '';
        this._cursorPos = 0;
        this.printPrompt();
        break;

      case '\x04': // Ctrl+D (EOF on empty buffer, Delete forward on non-empty)
        if (this._inputBuffer.length === 0) {
          this.writeln('exit');
          this.printPrompt();
        } else {
          this._handleDeleteForward();
        }
        break;

      case '\x0c': // Ctrl+L (Clear screen)
        this.clear();
        this._redrawLine();
        break;

      case '\t': // Tab (Autocomplete)
        this._handleTabCompletion();
        break;

      case '\x01': // Ctrl+A (Home)
        this._moveCursorToStart();
        break;

      case '\x05': // Ctrl+E (End)
        this._moveCursorToEnd();
        break;

      case '\x0b': // Ctrl+K (Kill to end of line)
        this._killToEndOfLine();
        break;

      case '\x15': // Ctrl+U (Delete whole line)
        this._clearLine();
        break;

      // Escape Sequences (Arrows, PageUp/Down, Home/End, Delete)
      default:
        if (data.startsWith('\x1b')) {
          this._handleEscapeSequence(data);
        } else if (data.length === 1 && data.charCodeAt(0) >= 32) {
          // Normal printable ASCII / Unicode character
          this._insertChar(data);
        }
        break;
    }
  }

  private _insertPastedContent(raw: string): void {
    const sanitized = this._options.sanitizePaste !== false ? this.sanitizePaste(raw) : raw;
    if (this._options.allowMultiline !== false && sanitized.includes('\n')) {
      const lines = sanitized.split(/\r\n|[\r\n]/);
      for (let i = 0; i < lines.length; i++) {
        if (lines[i].length > 0) {
          this._insertString(lines[i]);
        }
        if (i < lines.length - 1) {
          this._insertNewline();
        }
      }
    } else {
      const cleanStr = sanitized.replace(/[\r\n]+/g, ' ');
      this._insertString(cleanStr);
    }
  }

  private _insertNewline(): void {
    const left = this._inputBuffer.slice(0, this._cursorPos);
    const right = this._inputBuffer.slice(this._cursorPos);
    this._inputBuffer = left + '\n' + right;
    this._cursorPos++;
    this._redrawLine();
  }

  private _handleEnter(): void {
    this.writeln('');
    const input = this._inputBuffer.trim();

    if (!input) {
      this.printPrompt();
      return;
    }

    // Save to History
    if (this._history.length === 0 || this._history[this._history.length - 1] !== input) {
      this._history.push(input);
      const limit = this._options.historyLimit || 1000;
      if (this._history.length > limit) {
        this._history.shift();
      }
      if (this._options.historyStorageKey && typeof localStorage !== 'undefined') {
        try {
          localStorage.setItem(this._options.historyStorageKey, JSON.stringify(this._history));
        } catch {}
      }
    }
    this._historyIndex = -1;

    // Check for Slash Commands
    if (input.startsWith('/') && this._options.enableSlashCommands !== false) {
      this._executeSlashCommand(input);
    } else {
      // Execute prompt submit
      this.submitPrompt(input);
    }
  }

  private _handleBackspace(): void {
    if (this._cursorPos > 0) {
      const left = this._inputBuffer.slice(0, this._cursorPos - 1);
      const right = this._inputBuffer.slice(this._cursorPos);
      this._inputBuffer = left + right;
      this._cursorPos--;
      this._redrawLine();
    }
  }

  private _handleDeleteForward(): void {
    if (this._cursorPos < this._inputBuffer.length) {
      const left = this._inputBuffer.slice(0, this._cursorPos);
      const right = this._inputBuffer.slice(this._cursorPos + 1);
      this._inputBuffer = left + right;
      this._redrawLine();
    }
  }

  private _insertChar(ch: string): void {
    const left = this._inputBuffer.slice(0, this._cursorPos);
    const right = this._inputBuffer.slice(this._cursorPos);
    this._inputBuffer = left + ch + right;
    this._cursorPos++;
    this._redrawLine();
  }

  private _insertString(str: string): void {
    const left = this._inputBuffer.slice(0, this._cursorPos);
    const right = this._inputBuffer.slice(this._cursorPos);
    this._inputBuffer = left + str + right;
    this._cursorPos += str.length;
    this._redrawLine();
  }

  private _handleEscapeSequence(seq: string): void {
    switch (seq) {
      case '\x1b[D': // Left Arrow
      case '\x1bOD':
        if (this._cursorPos > 0) {
          this._cursorPos--;
          this.write('\x1b[D');
        }
        break;

      case '\x1b[C': // Right Arrow
      case '\x1bOC':
        if (this._cursorPos < this._inputBuffer.length) {
          this._cursorPos++;
          this.write('\x1b[C');
        }
        break;

      case '\x1b[A': // Up Arrow (History back)
      case '\x1bOA':
      case '\x10':   // Ctrl+P
        this._navigateHistory(-1);
        break;

      case '\x1b[B': // Down Arrow (History forward)
      case '\x1bOB':
      case '\x0e':   // Ctrl+N
        this._navigateHistory(1);
        break;

      case '\x1b[H': // Home
      case '\x1b[1~':
      case '\x1bOH':
        this._moveCursorToStart();
        break;

      case '\x1b[F': // End
      case '\x1b[4~':
      case '\x1bOF':
        this._moveCursorToEnd();
        break;

      case '\x1b[3~': // Delete
        this._handleDeleteForward();
        break;

      // Word Navigation: Ctrl+Left / Alt+Left / Alt+B
      case '\x1b[1;5D':
      case '\x1b[1;3D':
      case '\x1bb':
        this._moveWordLeft();
        break;

      // Word Navigation: Ctrl+Right / Alt+Right / Alt+F
      case '\x1b[1;5C':
      case '\x1b[1;3C':
      case '\x1bf':
        this._moveWordRight();
        break;
    }
  }

  private _navigateHistory(direction: number): void {
    if (this._history.length === 0) return;

    if (direction < 0) {
      // Up
      if (this._historyIndex === -1) {
        this._historyIndex = this._history.length - 1;
      } else if (this._historyIndex > 0) {
        this._historyIndex--;
      }
    } else {
      // Down
      if (this._historyIndex !== -1) {
        if (this._historyIndex < this._history.length - 1) {
          this._historyIndex++;
        } else {
          this._historyIndex = -1;
          this._inputBuffer = '';
          this._cursorPos = 0;
          this._redrawLine();
          return;
        }
      }
    }

    if (this._historyIndex !== -1) {
      this._inputBuffer = this._history[this._historyIndex];
      this._cursorPos = this._inputBuffer.length;
      this._redrawLine();
    }
  }

  private _redrawLine(): void {
    if (!this._terminal) return;
    const prompt = typeof this._promptStr === 'function' ? this._promptStr() : this._promptStr;
    const multiPrompt = typeof this._multilinePromptStr === 'function' ? this._multilinePromptStr() : this._multilinePromptStr;

    // Handle multiline buffer rendering
    if (this._inputBuffer.includes('\n')) {
      const lines = this._inputBuffer.split('\n');
      this._terminal.write('\r\x1b[K');
      for (let i = 0; i < lines.length; i++) {
        const prefix = i === 0 ? prompt : multiPrompt;
        this._terminal.write(prefix + lines[i]);
        if (i < lines.length - 1) {
          this._terminal.write('\r\n\x1b[K');
        }
      }
      return;
    }

    // Single line rendering
    this._terminal.write('\r\x1b[K');
    this._terminal.write(prompt + this._inputBuffer);

    // Reposition cursor
    const backAmount = this._inputBuffer.length - this._cursorPos;
    if (backAmount > 0) {
      this._terminal.write(`\x1b[${backAmount}D`);
    }
  }

  private _moveCursorToStart(): void {
    if (this._cursorPos > 0) {
      this.write(`\x1b[${this._cursorPos}D`);
      this._cursorPos = 0;
    }
  }

  private _moveCursorToEnd(): void {
    const diff = this._inputBuffer.length - this._cursorPos;
    if (diff > 0) {
      this.write(`\x1b[${diff}C`);
      this._cursorPos = this._inputBuffer.length;
    }
  }

  private _moveWordLeft(): void {
    if (this._cursorPos === 0) return;
    let idx = this._cursorPos;
    while (idx > 0 && this._inputBuffer[idx - 1] === ' ') idx--;
    while (idx > 0 && this._inputBuffer[idx - 1] !== ' ') idx--;
    const diff = this._cursorPos - idx;
    if (diff > 0) {
      this._cursorPos = idx;
      this.write(`\x1b[${diff}D`);
    }
  }

  private _moveWordRight(): void {
    if (this._cursorPos >= this._inputBuffer.length) return;
    let idx = this._cursorPos;
    while (idx < this._inputBuffer.length && this._inputBuffer[idx] !== ' ') idx++;
    while (idx < this._inputBuffer.length && this._inputBuffer[idx] === ' ') idx++;
    const diff = idx - this._cursorPos;
    if (diff > 0) {
      this._cursorPos = idx;
      this.write(`\x1b[${diff}C`);
    }
  }

  private _clearLine(): void {
    this._inputBuffer = '';
    this._cursorPos = 0;
    this._redrawLine();
  }

  private _killToEndOfLine(): void {
    this._inputBuffer = this._inputBuffer.slice(0, this._cursorPos);
    this._redrawLine();
  }

  private _deleteWordBackwards(): void {
    if (this._cursorPos === 0) return;
    let idx = this._cursorPos;
    while (idx > 0 && this._inputBuffer[idx - 1] === ' ') idx--;
    while (idx > 0 && this._inputBuffer[idx - 1] !== ' ') idx--;
    this._inputBuffer = this._inputBuffer.slice(0, idx) + this._inputBuffer.slice(this._cursorPos);
    this._cursorPos = idx;
    this._redrawLine();
  }

  private _deleteWordForwards(): void {
    if (this._cursorPos >= this._inputBuffer.length) return;
    let idx = this._cursorPos;
    while (idx < this._inputBuffer.length && this._inputBuffer[idx] === ' ') idx++;
    while (idx < this._inputBuffer.length && this._inputBuffer[idx] !== ' ') idx++;
    this._inputBuffer = this._inputBuffer.slice(0, this._cursorPos) + this._inputBuffer.slice(idx);
    this._redrawLine();
  }

  private _handleTabCompletion(): void {
    const current = this._inputBuffer;
    if (!current.startsWith('/')) return;

    const available = Array.from(this._commands.keys());
    const matches = available.filter(cmd => cmd.startsWith(current) && cmd !== current);

    if (matches.length === 1) {
      this._inputBuffer = matches[0] + ' ';
      this._cursorPos = this._inputBuffer.length;
      this._redrawLine();
    } else if (matches.length > 1) {
      this.writeln('');
      this.writeln(`${ANSI.slate}Available commands:${ANSI.reset} ${matches.join('  ')}`);
      this.printPrompt();
      this._inputBuffer = current;
      this._cursorPos = current.length;
      this._redrawLine();
    }
  }

  // --------------------------------------------------------------------------
  // Spinner Integration
  // --------------------------------------------------------------------------

  /**
   * Starts an animated inline terminal spinner with the provided status text.
   */
  startSpinner(text = 'Processing...', style: SpinnerStyle = 'dots'): TerminalSpinner {
    if (this._activeSpinner) {
      this._activeSpinner.dispose();
    }
    this._activeSpinner = new TerminalSpinner(this._terminal, text, { style });
    this._activeSpinner.start();
    return this._activeSpinner;
  }

  /**
   * Stops the currently active terminal spinner.
   */
  stopSpinner(clear = true): void {
    if (this._activeSpinner) {
      this._activeSpinner.stop(clear);
      this._activeSpinner = null;
    }
  }

  /**
   * Creates a standalone spinner instance connected to this adapter.
   */
  createSpinner(text = 'Processing...', options: SpinnerOptions = {}): TerminalSpinner {
    return new TerminalSpinner(this._terminal, text, options);
  }

  // --------------------------------------------------------------------------
  // Slash Commands Engine
  // --------------------------------------------------------------------------

  /**
   * Registers a custom slash command.
   */
  registerCommand(cmd: SlashCommand): void {
    const name = cmd.name.startsWith('/') ? cmd.name : `/${cmd.name}`;
    this._commands.set(name.toLowerCase(), cmd);
    if (cmd.aliases) {
      for (const alias of cmd.aliases) {
        const aliasName = alias.startsWith('/') ? alias : `/${alias}`;
        this._commands.set(aliasName.toLowerCase(), cmd);
      }
    }
  }

  /**
   * Unregisters a slash command.
   */
  unregisterCommand(name: string): boolean {
    const cmdName = name.startsWith('/') ? name : `/${name}`;
    return this._commands.delete(cmdName.toLowerCase());
  }

  /**
   * Returns list of all registered slash commands.
   */
  getCommands(): SlashCommand[] {
    const unique = new Set<SlashCommand>();
    for (const cmd of this._commands.values()) {
      unique.add(cmd);
    }
    return Array.from(unique);
  }

  private _executeSlashCommand(input: string): void {
    const parts = input.split(' ');
    const cmdName = parts[0].toLowerCase();
    const args = parts.slice(1).join(' ').trim();

    const cmd = this._commands.get(cmdName);
    if (cmd) {
      try {
        const result = cmd.handler(args, this);
        if (result instanceof Promise) {
          result.then(() => {
            if (!this._isStreaming) this.printPrompt();
          }).catch((err: unknown) => {
            const msg = err instanceof Error ? err.message : String(err);
            this.writeln(`${ANSI.rose}Command error: ${msg}${ANSI.reset}`);
            this.printPrompt();
          });
          return;
        }
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        this.writeln(`${ANSI.rose}Command error: ${msg}${ANSI.reset}`);
      }
    } else {
      this.writeln(`${ANSI.rose}Unknown slash command: ${cmdName}. Type /help for available commands.${ANSI.reset}`);
    }

    if (!this._isStreaming) {
      this.printPrompt();
    }
  }

  private _registerDefaultCommands(): void {
    // /help
    this.registerCommand({
      name: '/help',
      description: 'Display available slash commands',
      handler: (_, adapter) => {
        adapter.writeln('');
        adapter.writeln(`${ANSI.bold}Fusion v2 Slash Commands:${ANSI.reset}`);
        const commands = adapter.getCommands();
        for (const c of commands) {
          const usage = c.usage ? ` ${ANSI.dim}${c.usage}${ANSI.reset}` : '';
          adapter.writeln(`  ${ANSI.neonCyan}${c.name.padEnd(16)}${ANSI.reset}${usage.padEnd(20)} ${c.description}`);
        }
        adapter.writeln('');
      }
    });

    // /clear
    this.registerCommand({
      name: '/clear',
      description: 'Clear terminal screen buffer',
      handler: (_, adapter) => {
        adapter.clear();
      }
    });

    // /version
    this.registerCommand({
      name: '/version',
      description: 'Display Fusion harness and runtime version',
      handler: (_, adapter) => {
        adapter.writeln('');
        adapter.writeln(`${ANSI.emerald}⚡ Fusion v0.3.0${ANSI.reset} (x86_64/aarch64 / wasm32)`);
        adapter.writeln(`Backend: ${ANSI.purple}${adapter.backend.name}${ANSI.reset}`);
        adapter.writeln(`Model: ${ANSI.neonCyan}${adapter.model}${ANSI.reset}`);
        adapter.writeln(`Theme: ${ANSI.yellow}${adapter.theme.name || 'Custom'}${ANSI.reset}`);
        adapter.writeln('');
      }
    });

    // /theme
    this.registerCommand({
      name: '/theme',
      usage: '[theme_name]',
      description: 'Switch terminal color theme (deepOcean, cyberpunk, monokai, nord, gruvbox, tokyoNight)',
      handler: (args, adapter) => {
        if (args) {
          const themeName = args.trim();
          adapter.setTheme(themeName);
          adapter.writeln(`${ANSI.emerald}✔ Active theme set to: ${ANSI.neonCyan}${adapter.theme.name || themeName}${ANSI.reset}`);
        } else {
          adapter.writeln('');
          adapter.writeln(`${ANSI.bold}Available Themes:${ANSI.reset}`);
          for (const th of adapter.listThemes()) {
            const isCurrent = (THEMES[th]?.name === adapter.theme.name);
            const marker = isCurrent ? `${ANSI.emerald}●${ANSI.reset}` : '○';
            adapter.writeln(`  ${marker} ${ANSI.neonCyan}${th.padEnd(16)}${ANSI.reset} ${THEMES[th]?.name || th}`);
          }
          adapter.writeln('');
        }
      }
    });

    // /model
    this.registerCommand({
      name: '/model',
      usage: '[model_name]',
      description: 'Display or switch active model',
      handler: (args, adapter) => {
        if (args) {
          adapter.setModel(args);
          adapter.writeln(`${ANSI.emerald}✔ Active model set to: ${ANSI.neonCyan}${args}${ANSI.reset}`);
        } else {
          adapter.writeln(`${ANSI.slate}Current model: ${ANSI.neonCyan}${adapter.model}${ANSI.reset}`);
        }
      }
    });

    // /cost
    this.registerCommand({
      name: '/cost',
      description: 'Inspect session token usage and estimated cost',
      handler: (_, adapter) => {
        const s = adapter.stats;
        adapter.writeln('');
        adapter.writeln(`${ANSI.bold}Session Token & Cost Metrics:${ANSI.reset}`);
        adapter.writeln(`  Active Model:       ${ANSI.neonCyan}${adapter.model}${ANSI.reset}`);
        adapter.writeln(`  Prompt Tokens:      ${s.promptTokens.toLocaleString()}`);
        adapter.writeln(`  Completion Tokens:  ${s.completionTokens.toLocaleString()}`);
        adapter.writeln(`  Total Tokens:       ${s.totalTokens.toLocaleString()}`);
        adapter.writeln(`  Total Turns:        ${s.totalTurns}`);
        adapter.writeln(`  Estimated Cost:     ${ANSI.emerald}$${s.estimatedCost.toFixed(4)}${ANSI.reset}`);
        adapter.writeln('');
      }
    });

    // /checkpoint
    this.registerCommand({
      name: '/checkpoint',
      description: 'Export serialized session state checkpoint',
      handler: async (_, adapter) => {
        try {
          const checkpointData = await adapter.backend.checkpoint?.();
          if (checkpointData) {
            adapter.writeln(`${ANSI.emerald}✔ Session checkpoint exported (${checkpointData.length} bytes)${ANSI.reset}`);
          } else {
            adapter.writeln(`${ANSI.amber}Checkpoint not supported by active backend.${ANSI.reset}`);
          }
        } catch (e: unknown) {
          const msg = e instanceof Error ? e.message : String(e);
          adapter.writeln(`${ANSI.rose}Checkpoint failed: ${msg}${ANSI.reset}`);
        }
      }
    });

    // /restore
    this.registerCommand({
      name: '/restore',
      usage: '<checkpoint_json>',
      description: 'Restore session state from checkpoint JSON',
      handler: async (args, adapter) => {
        if (!args) {
          adapter.writeln(`${ANSI.rose}Usage: /restore <checkpoint_json_data>${ANSI.reset}`);
          return;
        }
        try {
          await adapter.backend.restore?.(args);
          adapter.writeln(`${ANSI.emerald}✔ Session state restored from checkpoint.${ANSI.reset}`);
        } catch (e: unknown) {
          const msg = e instanceof Error ? e.message : String(e);
          adapter.writeln(`${ANSI.rose}Restore failed: ${msg}${ANSI.reset}`);
        }
      }
    });
  }

  // --------------------------------------------------------------------------
  // Agent Execution & Streaming
  // --------------------------------------------------------------------------

  /**
   * Inserts a single token delta into the streaming terminal output.
   */
  streamToken(delta: string, type: 'text' | 'thinking' | 'status' = 'text'): void {
    if (!this._terminal || !delta) return;

    if (this._activeSpinner) {
      this.stopSpinner(true);
    }

    switch (type) {
      case 'text':
        this.write(delta);
        this._stats.completionTokens += Math.round(delta.length / 3.5);
        this._stats.totalTokens = this._stats.promptTokens + this._stats.completionTokens;
        break;

      case 'thinking':
        this.write(`${ANSI.slate}${delta}${ANSI.reset}`);
        break;

      case 'status':
        this.writeln(`${ANSI.slate}⚙ ${delta}${ANSI.reset}`);
        break;
    }
  }

  /**
   * Helper to insert a styled stream token with optional usage tracking and delta hook.
   */
  insertStreamToken(token: string, options: StreamTokenOptions = {}): void {
    if (!token || !this._terminal) return;

    const style = options.style || '';
    const reset = style ? ANSI.reset : '';
    this.write(`${style}${token}${reset}`);

    if (options.trackUsage !== false) {
      this._stats.completionTokens += Math.round(token.length / 3.5);
      this._stats.totalTokens = this._stats.promptTokens + this._stats.completionTokens;
    }

    if (options.onDelta) {
      options.onDelta(token);
    }
  }

  /**
   * Submits a prompt turn to the active agent backend.
   */
  async submitPrompt(prompt: string): Promise<string> {
    if (this._options.onPromptSubmit) {
      const intercepted = await this._options.onPromptSubmit(prompt);
      if (intercepted === true) return '';
    }

    this._isStreaming = true;
    this._abortController = new AbortController();

    this._stats.totalTurns++;
    this._stats.promptTokens += Math.round(prompt.length / 3.5) + 60;
    this._stats.totalTokens = this._stats.promptTokens + this._stats.completionTokens;

    // Start inline status spinner
    this.startSpinner('Fusion thinking...');

    const deferred = createDeferred<string>();

    this._backend.promptTurn(
      prompt,
      (event: AgentEvent) => {
        this.handleAgentEvent(event);
      },
      this._abortController?.signal
    ).then((response: string) => {
      this.stopSpinner(true);
      this._isStreaming = false;
      this._abortController = null;
      this.writeln('');
      this.printPrompt();
      deferred.resolve(response);
    }).catch((err: unknown) => {
      this.stopSpinner(true);
      this._isStreaming = false;
      this._abortController = null;
      const msg = err instanceof Error ? err.message : String(err);
      this.writeln(`\r\n${ANSI.rose}Agent execution error: ${msg}${ANSI.reset}`);
      this.printPrompt();
      deferred.reject(err instanceof Error ? err : new Error(String(err)));
    });

    return deferred.promise;
  }

  /**
   * Cancels the currently running prompt turn.
   */
  abortTurn(): void {
    if (this._abortController) {
      this._abortController.abort();
      this._abortController = null;
    }
    this.stopSpinner(true);
    this._backend.cancelTurn?.().catch(() => {});
    this._isStreaming = false;
    this.writeln(`\r\n${ANSI.rose}^C Turn cancelled by user.${ANSI.reset}`);
    this.printPrompt();
  }

  /**
   * Dispatches and renders incoming real-time streaming events from agent.
   */
  handleAgentEvent(event: AgentEvent): void {
    if (!this._terminal) return;

    switch (event.type) {
      case 'text_delta':
        if (event.delta) {
          this.streamToken(event.delta, 'text');
        }
        break;

      case 'thinking_delta':
        if (event.delta) {
          this.streamToken(event.delta, 'thinking');
        }
        break;

      case 'status':
        if (event.message) {
          if (this._activeSpinner) {
            this._activeSpinner.setText(event.message);
          } else {
            this.writeln(`${ANSI.slate}⚙ ${event.message}${ANSI.reset}`);
          }
        }
        break;

      case 'tool_started':
        this.stopSpinner(true);
        this.writeln('');
        this.writeln(AnsiFormatter.formatToolStart(event.name, event.args));
        this.startSpinner(`Executing ${event.name}...`);
        break;

      case 'tool_finished':
        this.stopSpinner(true);
        this.writeln(AnsiFormatter.formatToolResult(event.output, event.success, event.duration_ms));
        this.writeln('');
        break;

      case 'advisor_critique':
        this.stopSpinner(true);
        this.writeln('');
        this.writeln(AnsiFormatter.formatAdvisor(event.advisor, event.approved, event.critique));
        this.writeln('');
        break;

      case 'finished':
        this.stopSpinner(true);
        if (event.usage) {
          if (event.usage.prompt_tokens) this._stats.promptTokens = event.usage.prompt_tokens;
          if (event.usage.completion_tokens) this._stats.completionTokens = event.usage.completion_tokens;
          this._stats.totalTokens = this._stats.promptTokens + this._stats.completionTokens;
        }
        break;

      case 'error':
        this.stopSpinner(true);
        this.writeln(`\r\n${ANSI.rose}Error: ${event.message}${ANSI.reset}`);
        break;
    }
  }

  // --------------------------------------------------------------------------
  // Properties & Getters / Setters
  // --------------------------------------------------------------------------

  get terminal(): XtermInstance | null {
    return this._terminal;
  }

  get container(): HTMLElement | null {
    return this._container;
  }

  get backend(): AgentBackend {
    return this._backend;
  }

  setBackend(backend: AgentBackend): void {
    this._backend = backend;
  }

  get model(): string {
    return this._activeModel;
  }

  setModel(model: string): void {
    this._activeModel = model;
  }

  get theme(): TerminalTheme {
    return this._activeTheme;
  }

  get mode(): 'interactive' | 'raw' | 'hybrid' {
    return this._mode;
  }

  setMode(mode: 'interactive' | 'raw' | 'hybrid'): void {
    this._mode = mode;
  }

  get isStreaming(): boolean {
    return this._isStreaming;
  }

  get stats(): Readonly<SessionStats> {
    return this._stats;
  }

  resetStats(): void {
    this._stats = {
      promptTokens: 0,
      completionTokens: 0,
      totalTokens: 0,
      totalTurns: 0,
      estimatedCost: 0
    };
  }
}

// ============================================================================
// 11. Standalone Factory Function
// ============================================================================

/**
 * Creates and initializes a new Fusion xterm.js adapter.
 */
export function createXtermAdapter(options: XtermAdapterOptions = {}): XtermAdapter {
  return new XtermAdapter(options);
}

export const FusionTerminalAdapter = XtermAdapter;
export type FusionTerminalAdapter = XtermAdapter;

export default XtermAdapter;
