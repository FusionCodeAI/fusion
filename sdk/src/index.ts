/**
 * @theaungmyatmoe/fusion — Official TypeScript & WebAssembly SDK for Fusion AI Coding Assistant
 *
 * Provides in-browser and Node.js agent execution, virtual filesystem operations,
 * multi-model orchestration, checkpoint serialization, and xterm.js terminal integration.
 */

// Core Agent Controller
export { FusionAgent } from './agent.js';

// WebAssembly Loader & Low-Level Bindings
export {
  initWasm,
  loadFusionWasm,
  getWasmModule,
  isWasmInitialized
} from './wasm.js';

export type { RawFusionWasmModule } from './wasm.js';

// Xterm.js Terminal Adapter, Formatter & Backends
export {
  XtermAdapter,
  FusionTerminalAdapter,
  createXtermAdapter,
  KeyEncoder,
  AnsiFormatter,
  ANSI,
  THEMES,
  WasmAgentBackend,
  WebSocketAgentBackend,
  MockAgentBackend
} from './xterm-adapter.js';

export type {
  XtermInstance,
  FitAddonInstance,
  TerminalTheme,
  AgentEvent,
  AgentEventCallback,
  AgentBackend,
  WasmFusionAgentInstance,
  SessionStats,
  SlashCommand,
  XtermAdapterOptions
} from './xterm-adapter.js';

// Types and Interfaces — Re-export all type definitions
export * from './types.js';

/**
 * Returns the Fusion SDK version string.
 */
export const VERSION = '0.3.0';
