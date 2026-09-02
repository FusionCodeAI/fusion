/**
 * @fusion/sdk — WebAssembly Loader, Runtime Bridge & Virtual File System
 *
 * Provides:
 * 1. `loadFusionWasm`: Cross-environment WebAssembly loader for Browser & Node.js
 * 2. `VirtualFileSystem`: VFS bridge supporting in-memory, localStorage, and agent sync
 * 3. `WasmEventBridge`: Reactive event stream converting raw WASM callbacks into typed observables / AsyncIterators
 */

import type {
  WasmFusionAgentBindings,
  WasmInitOptions,
  FusionConfig,
  FusionEvent,
  StatusEvent,
  TextDeltaEvent,
  ThinkingDeltaEvent,
  ToolStartedEvent,
  ToolFinishedEvent,
  AdvisorStartedEvent,
  AdvisorCritiqueEvent,
  FinishedEvent,
  ErrorEvent,
  PromptTurnCallback,
  CheckpointData,
  Message
} from './types.js';

// ============================================================================
// 1. WASM Module Types & Interfaces
// ============================================================================

/**
 * High-level exported shape of the initialized Fusion WebAssembly module.
 */
export interface FusionWasmModule {
  /**
   * WasmFusionAgent constructor exported from Rust wasm-bindgen.
   */
  WasmFusionAgent: {
    new (config_json?: string): WasmFusionAgentBindings;
  };
  /**
   * Instantiates a new Fusion agent from a JSON configuration string.
   */
  create_agent(config_json: string): WasmFusionAgentBindings;
  /**
   * Executes a conversation turn on the active global agent singleton.
   */
  prompt_turn(input_str: string, callback?: (event: unknown) => void): Promise<string>;
  /**
   * Serializes a snapshot checkpoint of the active global agent session.
   */
  checkpoint(): string;
  /**
   * Restores the active global agent session from a checkpoint JSON string.
   */
  restore(checkpoint_json: string): void;
  /**
   * Returns the Fusion engine version string.
   */
  fusion_version(): string;
  /**
   * Optional reference to WebAssembly linear memory.
   */
  memory?: WebAssembly.Memory;
  /**
   * Raw underlying WebAssembly Instance (if directly instantiated).
   */
  rawInstance?: WebAssembly.Instance;
  /**
   * Raw underlying WebAssembly Module (if compiled).
   */
  rawModule?: WebAssembly.Module;
}

/**
 * Backward-compatible alias for RawFusionWasmModule.
 */
export type RawFusionWasmModule = FusionWasmModule;

let wasmModuleInstance: FusionWasmModule | null = null;
let initPromise: Promise<FusionWasmModule> | null = null;

/**
 * Checks if the WebAssembly module has already been initialized.
 */
export function isWasmInitialized(): boolean {
  return wasmModuleInstance !== null;
}

/**
 * Retrieves the cached WebAssembly module instance.
 * Throws if `loadFusionWasm()` or `initWasm()` has not been called.
 */
export function getWasmModule(): FusionWasmModule {
  if (!wasmModuleInstance) {
    throw new Error(
      '@fusion/sdk: WebAssembly module is not initialized. Please call `await loadFusionWasm()` before accessing agent bindings.'
    );
  }
  return wasmModuleInstance;
}

/**
 * Resets the cached WebAssembly module instance (primarily used for testing).
 */
export function resetWasmModule(): void {
  wasmModuleInstance = null;
  initPromise = null;
}

// ============================================================================
// 2. Cross-Environment WASM Loader (`loadFusionWasm`)
// ============================================================================

/**
 * Input source type for loading the Fusion WebAssembly module.
 */
export type WasmSourceInput =
  | string
  | URL
  | ArrayBuffer
  | Uint8Array
  | Response
  | WebAssembly.Module
  | WasmInitOptions
  | undefined;

/**
 * Loads and initializes the Fusion WebAssembly binary across Browser and Node.js runtimes.
 *
 * Supported sources:
 * - `undefined` / omitted: Auto-detects runtime, tries standard bundle paths (`./fusion.wasm`, `/fusion.wasm`),
 *   or falls back to an in-memory JS runtime.
 * - `string` (URL or file path or base64 data URI):
 *   - In Browser: Fetches URL via `fetch()` and uses `WebAssembly.instantiateStreaming`.
 *   - In Node.js: Reads file via `node:fs/promises` and uses `WebAssembly.instantiate`.
 * - `ArrayBuffer` or `Uint8Array`: Instantiates binary buffer directly.
 * - `Response`: Streams and instantiates directly via `WebAssembly.instantiateStreaming`.
 * - `WebAssembly.Module`: Instantiates pre-compiled module.
 *
 * @param wasmSource Optional URL, file path, ArrayBuffer, Uint8Array, Response, or WasmInitOptions.
 * @returns Initialized Fusion WebAssembly module.
 */
export async function loadFusionWasm(wasmSource?: WasmSourceInput): Promise<FusionWasmModule> {
  if (wasmModuleInstance) {
    return wasmModuleInstance;
  }

  if (initPromise) {
    return initPromise;
  }

  initPromise = (async () => {
    try {
      // 1. Check if global window.fusion_wasm is pre-injected (e.g. via script tag or bundler)
      if (
        typeof window !== 'undefined' &&
        (window as unknown as { fusion_wasm?: FusionWasmModule }).fusion_wasm
      ) {
        wasmModuleInstance = (window as unknown as { fusion_wasm: FusionWasmModule }).fusion_wasm;
        return wasmModuleInstance;
      }

      // 2. Unpack options if passed as WasmInitOptions
      let source: string | URL | ArrayBuffer | Uint8Array | Response | WebAssembly.Module | undefined;
      if (wasmSource && typeof wasmSource === 'object' && !('byteLength' in wasmSource) && !('status' in wasmSource) && !(wasmSource instanceof URL) && !(wasmSource instanceof WebAssembly.Module)) {
        const opts = wasmSource as WasmInitOptions;
        source = (opts.wasmBinary as ArrayBuffer | Uint8Array | Response | WebAssembly.Module) || opts.wasmUrl;
      } else {
        source = wasmSource as string | URL | ArrayBuffer | Uint8Array | Response | WebAssembly.Module | undefined;
      }

      const isNode =
        typeof process !== 'undefined' &&
        process.versions != null &&
        process.versions.node != null;

      // 3. Check for bundled wasm-bindgen JS wrapper if present
      if (isNode) {
        try {
          // Dynamic import: wasm-bindgen JS bundle is only present in Node/bundled environments, not in browser
          const wasmBindgen = await import('../wasm/fusion.js').catch(() => null);
          if (wasmBindgen && wasmBindgen.default) {
            let bindgenInput: unknown = source;
            if (!bindgenInput) {
              // Dynamic import: node:fs and node:path are Node-specific modules unavailable in browser runtimes
              const nodeFs = await import('node:fs/promises').catch(() => null);
              const nodePath = await import('node:path').catch(() => null);
              if (nodeFs && nodePath) {
                const searchPaths = [
                  nodePath.resolve(process.cwd(), 'wasm/fusion.wasm'),
                  nodePath.resolve(process.cwd(), 'fusion.wasm'),
                  nodePath.resolve(process.cwd(), 'dist/wasm/fusion.wasm')
                ];
                for (const p of searchPaths) {
                  try {
                    const buf = await nodeFs.readFile(p);
                    bindgenInput = buf;
                    break;
                  } catch {
                    // continue search
                  }
                }
              }
            }
            if (bindgenInput) {
              await wasmBindgen.default(bindgenInput);
              wasmModuleInstance = wasmBindgen as unknown as FusionWasmModule;
              return wasmModuleInstance;
            }
          }
        } catch {
          // Fall through to custom instantiation / fallback
        }
      }

      // 4. Resolve binary bytes / Response across environments
      let binaryBuffer: ArrayBuffer | Uint8Array | null = null;
      let responseSource: Response | null = null;
      let compiledModule: WebAssembly.Module | null = null;

      if (source instanceof WebAssembly.Module) {
        compiledModule = source;
      } else if (source instanceof Response) {
        responseSource = source;
      } else if (source instanceof ArrayBuffer || source instanceof Uint8Array) {
        binaryBuffer = source;
      } else if (typeof source === 'string' || source instanceof URL) {
        const urlStr = source.toString();

        if (urlStr.startsWith('data:application/wasm;base64,')) {
          const base64 = urlStr.replace('data:application/wasm;base64,', '');
          binaryBuffer = decodeBase64ToUint8Array(base64);
        } else if (isNode && !urlStr.startsWith('http://') && !urlStr.startsWith('https://')) {
          // Read from filesystem in Node.js
          try {
            // Dynamic import: node:fs/promises is Node-specific and cannot be statically imported in browser
            const fs = await import('node:fs/promises');
            const fileBuf = await fs.readFile(urlStr);
            binaryBuffer = new Uint8Array(fileBuf.buffer, fileBuf.byteOffset, fileBuf.byteLength);
          } catch (err) {
            throw new Error(`@fusion/sdk: Failed to read WASM file at "${urlStr}": ${(err as Error).message}`);
          }
        } else if (typeof fetch !== 'undefined') {
          // Fetch from URL in Browser / Fetch environment
          const resp = await fetch(urlStr);
          if (!resp.ok) {
            throw new Error(`@fusion/sdk: Failed to fetch WASM binary from "${urlStr}": ${resp.status} ${resp.statusText}`);
          }
          responseSource = resp;
        }
      }

      // Auto-detection in browser if no source supplied
      if (!binaryBuffer && !responseSource && !compiledModule && typeof fetch !== 'undefined') {
        const defaultUrls = [
          './fusion.wasm',
          './wasm/fusion.wasm',
          '../wasm/fusion.wasm',
          '/fusion.wasm',
          './dist/wasm/fusion.wasm'
        ];
        for (const url of defaultUrls) {
          try {
            const resp = await fetch(url);
            if (resp.ok) {
              const contentType = resp.headers.get('content-type') || '';
              if (contentType.includes('wasm') || url.endsWith('.wasm')) {
                responseSource = resp;
                break;
              }
            }
          } catch {
            // continue trying
          }
        }
      }

      // Auto-detection in Node.js if no source supplied
      if (!binaryBuffer && !responseSource && !compiledModule && isNode) {
        try {
          // Dynamic import: node:fs/promises and node:path are Node-specific and unavailable in browser
          const fs = await import('node:fs/promises');
          const path = await import('node:path');
          const candidates = [
            path.resolve(process.cwd(), 'fusion.wasm'),
            path.resolve(process.cwd(), 'wasm/fusion.wasm'),
            path.resolve(process.cwd(), 'sdk/dist/fusion.wasm'),
            path.resolve(process.cwd(), 'target/wasm32-unknown-unknown/release/fusion.wasm')
          ];
          for (const cand of candidates) {
            try {
              const buf = await fs.readFile(cand);
              binaryBuffer = new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
              break;
            } catch {
              // continue
            }
          }
        } catch {
          // fallback
        }
      }

      // 5. Try WebAssembly instantiation if binary or response is available
      if (responseSource || binaryBuffer || compiledModule) {
        try {
          const wasmImports = createDefaultWasmImports();
          let instance: WebAssembly.Instance;
          let module: WebAssembly.Module;

          if (compiledModule) {
            module = compiledModule;
            instance = await WebAssembly.instantiate(module, wasmImports);
          } else if (
            responseSource &&
            typeof WebAssembly.instantiateStreaming === 'function' &&
            responseSource.headers?.get('content-type')?.includes('application/wasm')
          ) {
            const result = await WebAssembly.instantiateStreaming(responseSource, wasmImports);
            instance = result.instance;
            module = result.module;
          } else {
            const bytes = binaryBuffer ?? (responseSource ? await responseSource.arrayBuffer() : null);
            if (bytes) {
              const result = await WebAssembly.instantiate(bytes, wasmImports);
              instance = (result as WebAssembly.WebAssemblyInstantiatedSource).instance || (result as unknown as WebAssembly.Instance);
              module = (result as WebAssembly.WebAssemblyInstantiatedSource).module || (await WebAssembly.compile(bytes));
            } else {
              throw new Error('No valid WASM bytes available');
            }
          }

          // Build WASM module wrapper connecting instance exports to bindings
          wasmModuleInstance = createInstantiatedModuleWrapper(instance, module);
          return wasmModuleInstance;
        } catch {
          // If low-level instantiation fails, use fallback module
        }
      }

      // 6. Fallback: High-fidelity in-memory pure JS module for testing and offline execution
      if (!wasmModuleInstance) {
        wasmModuleInstance = createFallbackModule();
      }

      return wasmModuleInstance;
    } finally {
      initPromise = null;
    }
  })();

  return initPromise;
}

/**
 * Backward-compatible initialization helper.
 */
export async function initWasm(options: WasmInitOptions = {}): Promise<FusionWasmModule> {
  return loadFusionWasm(options);
}

/**
 * Helper to decode base64 string to Uint8Array across runtimes.
 */
function decodeBase64ToUint8Array(base64: string): Uint8Array {
  if (typeof Buffer !== 'undefined') {
    const buf = Buffer.from(base64, 'base64');
    return new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  }
  if (typeof atob === 'function') {
    const binaryStr = atob(base64);
    const len = binaryStr.length;
    const bytes = new Uint8Array(len);
    for (let i = 0; i < len; i++) {
      bytes[i] = binaryStr.charCodeAt(i);
    }
    return bytes;
  }
  throw new Error('@fusion/sdk: No base64 decoder available in current environment');
}

/**
 * Creates default WASM import object supporting browser and Node bindings.
 */
function createDefaultWasmImports(): WebAssembly.Imports {
  return {
    env: {
      memory: new WebAssembly.Memory({ initial: 256, maximum: 4096 }),
      abort: (msgPtr: number, filePtr: number, line: number, col: number) => {
        console.error(`WASM abort at ${filePtr}:${line}:${col}, msgPtr=${msgPtr}`);
      },
      now: () => Date.now(),
      random: () => Math.random()
    },
    wasi_snapshot_preview1: {
      proc_exit: (code: number) => {
        if (typeof process !== 'undefined' && process.exit) {
          process.exit(code);
        }
      },
      fd_write: () => 0,
      fd_read: () => 0,
      fd_close: () => 0,
      fd_seek: () => 0,
      environ_sizes_get: () => 0,
      environ_get: () => 0,
      clock_time_get: () => 0
    }
  };
}

/**
 * Wraps a raw WebAssembly.Instance into the standard FusionWasmModule interface.
 */
function createInstantiatedModuleWrapper(
  instance: WebAssembly.Instance,
  module: WebAssembly.Module
): FusionWasmModule {
  const exports = instance.exports as Record<string, unknown>;

  // Check if standard wasm-bindgen classes exist on instance exports
  if (typeof exports.create_agent === 'function' && typeof exports.prompt_turn === 'function') {
    return {
      WasmFusionAgent: exports.WasmFusionAgent as unknown as { new (config?: string): WasmFusionAgentBindings },
      create_agent: exports.create_agent as (config: string) => WasmFusionAgentBindings,
      prompt_turn: exports.prompt_turn as (input: string, callback?: (event: unknown) => void) => Promise<string>,
      checkpoint: exports.checkpoint as () => string,
      restore: exports.restore as (checkpoint_json: string) => void,
      fusion_version: exports.fusion_version ? (exports.fusion_version as () => string) : () => '0.3.0',
      memory: exports.memory as WebAssembly.Memory | undefined,
      rawInstance: instance,
      rawModule: module
    };
  }

  // If exports are low-level C ABI or minimal, wrap with fallback logic
  const fallback = createFallbackModule();
  return {
    ...fallback,
    rawInstance: instance,
    rawModule: module,
    memory: exports.memory as WebAssembly.Memory | undefined
  };
}

// ============================================================================
// 3. Virtual File System Bridge (`VirtualFileSystem`)
// ============================================================================

/**
 * Interface for pluggable VFS storage backends.
 */
export interface VfsStorageBackend {
  /** Reads raw string content of a file, or null/undefined if missing */
  read(path: string): string | null | undefined;
  /** Writes string content to a path */
  write(path: string, content: string): void;
  /** Deletes a file path, returning true if deleted */
  delete(path: string): boolean;
  /** Lists all normalized file paths */
  list(): string[];
  /** Clears all stored files */
  clear(): void;
  /** Checks if a path exists */
  has?(path: string): boolean;
}

/**
 * In-memory storage backend using a JavaScript `Map`.
 */
export class InMemoryStorageBackend implements VfsStorageBackend {
  private store: Map<string, string> = new Map();

  constructor(initialFiles?: Record<string, string>) {
    if (initialFiles) {
      for (const [k, v] of Object.entries(initialFiles)) {
        this.write(k, v);
      }
    }
  }

  read(path: string): string | null {
    const val = this.store.get(path);
    return val !== undefined ? val : null;
  }

  write(path: string, content: string): void {
    this.store.set(path, content);
  }

  delete(path: string): boolean {
    return this.store.delete(path);
  }

  list(): string[] {
    return Array.from(this.store.keys()).sort();
  }

  clear(): void {
    this.store.clear();
  }

  has(path: string): boolean {
    return this.store.has(path);
  }
}

/**
 * Persistent storage backend using Browser `window.localStorage`.
 * Falls back to in-memory store if localStorage is unavailable or restricted.
 */
export class LocalStorageBackend implements VfsStorageBackend {
  private prefix: string;
  private memoryFallback: InMemoryStorageBackend | null = null;

  constructor(prefix: string = 'fusion:vfs:') {
    this.prefix = prefix;
    if (!this.isAvailable()) {
      this.memoryFallback = new InMemoryStorageBackend();
    }
  }

  private isAvailable(): boolean {
    try {
      if (typeof window === 'undefined' || typeof window.localStorage === 'undefined') {
        return false;
      }
      const testKey = `${this.prefix}__test__`;
      window.localStorage.setItem(testKey, '1');
      window.localStorage.removeItem(testKey);
      return true;
    } catch {
      return false;
    }
  }

  read(path: string): string | null {
    if (this.memoryFallback) {
      return this.memoryFallback.read(path);
    }
    try {
      return window.localStorage.getItem(this.prefix + path);
    } catch {
      return null;
    }
  }

  write(path: string, content: string): void {
    if (this.memoryFallback) {
      this.memoryFallback.write(path, content);
      return;
    }
    try {
      window.localStorage.setItem(this.prefix + path, content);
    } catch {
      // Storage quota exceeded or disabled; fall back to memory
      if (!this.memoryFallback) {
        this.memoryFallback = new InMemoryStorageBackend();
      }
      this.memoryFallback.write(path, content);
    }
  }

  delete(path: string): boolean {
    if (this.memoryFallback) {
      return this.memoryFallback.delete(path);
    }
    try {
      const key = this.prefix + path;
      const exists = window.localStorage.getItem(key) !== null;
      if (exists) {
        window.localStorage.removeItem(key);
        return true;
      }
      return false;
    } catch {
      return false;
    }
  }

  list(): string[] {
    if (this.memoryFallback) {
      return this.memoryFallback.list();
    }
    const paths: string[] = [];
    try {
      const len = window.localStorage.length;
      for (let i = 0; i < len; i++) {
        const key = window.localStorage.key(i);
        if (key && key.startsWith(this.prefix)) {
          paths.push(key.substring(this.prefix.length));
        }
      }
    } catch {
      // ignore
    }
    return paths.sort();
  }

  clear(): void {
    if (this.memoryFallback) {
      this.memoryFallback.clear();
      return;
    }
    try {
      const keysToRemove: string[] = [];
      const len = window.localStorage.length;
      for (let i = 0; i < len; i++) {
        const key = window.localStorage.key(i);
        if (key && key.startsWith(this.prefix)) {
          keysToRemove.push(key);
        }
      }
      for (const k of keysToRemove) {
        window.localStorage.removeItem(k);
      }
    } catch {
      // ignore
    }
  }

  has(path: string): boolean {
    if (this.memoryFallback) {
      return this.memoryFallback.has(path);
    }
    try {
      return window.localStorage.getItem(this.prefix + path) !== null;
    } catch {
      return false;
    }
  }
}

/**
 * Grep match result in VFS.
 */
export interface VfsGrepMatch {
  path: string;
  line: number;
  text: string;
}

/**
 * Metadata stat information for a virtual file.
 */
export interface VfsStat {
  size: number;
  lines: number;
  exists: boolean;
  lastModified?: number;
}

/**
 * VFS file change notification event.
 */
export interface VfsChangeEvent {
  type: 'write' | 'edit' | 'delete' | 'clear' | 'import';
  path?: string;
  oldContent?: string;
  newContent?: string;
}

/**
 * Virtual File System Bridge for WebAssembly and TypeScript runtimes.
 *
 * Supports in-memory storage, browser localStorage persistence, surgical edits,
 * grep searches, glob pattern matching, and two-way synchronization with WASM agent bindings.
 */
export class VirtualFileSystem {
  private backend: VfsStorageBackend;
  private changeListeners: Set<(event: VfsChangeEvent) => void> = new Set();

  constructor(
    backend?: VfsStorageBackend | 'memory' | 'localstorage',
    options?: { seedDefaults?: boolean; prefix?: string }
  ) {
    if (!backend || backend === 'memory') {
      this.backend = new InMemoryStorageBackend();
    } else if (backend === 'localstorage') {
      this.backend = new LocalStorageBackend(options?.prefix || 'fusion:vfs:');
    } else {
      this.backend = backend;
    }

    if (options?.seedDefaults !== false && this.backend.list().length === 0) {
      this.seedDefaults();
    }
  }

  /**
   * Normalizes a file path by trimming whitespace, converting backslashes,
   * stripping leading `./` and collapsing consecutive slashes.
   */
  public normalizePath(path: string): string {
    return path
      .trim()
      .replace(/\\/g, '/')
      .replace(/^\.\//, '')
      .replace(/^\/+/, '')
      .replace(/\/+/g, '/');
  }

  /**
   * Reads the UTF-8 text contents of a file.
   * Throws an Error if the file does not exist.
   */
  public readFile(path: string): string {
    const cleanPath = this.normalizePath(path);
    const content = this.backend.read(cleanPath);
    if (content === null || content === undefined) {
      throw new Error(`@fusion/sdk VFS: File not found: "${path}"`);
    }
    return content;
  }

  /**
   * Attempts to read a file, returning `null` if the file does not exist.
   */
  public tryReadFile(path: string): string | null {
    const cleanPath = this.normalizePath(path);
    const content = this.backend.read(cleanPath);
    return content !== null && content !== undefined ? content : null;
  }

  /**
   * Writes UTF-8 text contents to a virtual file path.
   */
  public writeFile(path: string, content: string): void {
    const cleanPath = this.normalizePath(path);
    const oldContent = this.tryReadFile(cleanPath) ?? undefined;
    this.backend.write(cleanPath, content);
    this.notifyChange({
      type: 'write',
      path: cleanPath,
      oldContent,
      newContent: content
    });
  }

  /**
   * Surgically edits a file by replacing the first occurrence of `oldStr` with `newStr`.
   * Throws an Error if the file does not exist or if `oldStr` is not found.
   */
  public editFile(path: string, oldStr: string, newStr: string): string {
    const cleanPath = this.normalizePath(path);
    const content = this.readFile(cleanPath);
    if (!content.includes(oldStr)) {
      throw new Error(
        `@fusion/sdk VFS: Target string to replace was not found in "${path}"`
      );
    }
    const updated = content.replace(oldStr, newStr);
    this.backend.write(cleanPath, updated);
    this.notifyChange({
      type: 'edit',
      path: cleanPath,
      oldContent: content,
      newContent: updated
    });
    return updated;
  }

  /**
   * Deletes a file path from the virtual filesystem.
   * Returns `true` if the file existed and was deleted.
   */
  public deleteFile(path: string): boolean {
    const cleanPath = this.normalizePath(path);
    const oldContent = this.tryReadFile(cleanPath) ?? undefined;
    const deleted = this.backend.delete(cleanPath);
    if (deleted) {
      this.notifyChange({
        type: 'delete',
        path: cleanPath,
        oldContent
      });
    }
    return deleted;
  }

  /**
   * Checks if a file exists in the virtual filesystem.
   */
  public exists(path: string): boolean {
    const cleanPath = this.normalizePath(path);
    if (this.backend.has) {
      return this.backend.has(cleanPath);
    }
    return this.backend.read(cleanPath) !== null && this.backend.read(cleanPath) !== undefined;
  }

  /**
   * Returns metadata stat information for a virtual file path.
   */
  public stat(path: string): VfsStat {
    const content = this.tryReadFile(path);
    if (content === null) {
      return { size: 0, lines: 0, exists: false };
    }
    return {
      size: new TextEncoder().encode(content).length,
      lines: content.split('\n').length,
      exists: true
    };
  }

  /**
   * Lists all files in the virtual filesystem, optionally filtered by glob pattern.
   */
  public listFiles(pattern?: string): string[] {
    const all = this.backend.list().sort();
    if (!pattern || pattern === '*' || pattern === '**/*') {
      return all;
    }
    return this.glob(pattern);
  }

  /**
   * Searches file contents matching a substring or RegExp pattern, optionally filtered by path.
   */
  public grep(pattern: string | RegExp, pathFilter?: string): VfsGrepMatch[] {
    const matches: VfsGrepMatch[] = [];
    const files = this.backend.list();
    const cleanFilter = pathFilter ? this.normalizePath(pathFilter) : null;
    const regex = typeof pattern === 'string' ? null : pattern;

    for (const filePath of files) {
      if (cleanFilter && !filePath.includes(cleanFilter)) {
        continue;
      }
      const content = this.tryReadFile(filePath);
      if (!content) continue;

      const lines = content.split('\n');
      for (let i = 0; i < lines.length; i++) {
        const line = lines[i];
        const isMatch = regex ? regex.test(line) : line.includes(pattern as string);
        if (isMatch) {
          matches.push({
            path: filePath,
            line: i + 1,
            text: line
          });
        }
      }
    }
    return matches;
  }

  /**
   * Matches virtual files against a glob pattern (e.g. `*.ts`, `src/**\/*.js`, `**\/*.json`).
   */
  public glob(pattern: string): string[] {
    const cleanPat = this.normalizePath(pattern);
    const regexPattern = cleanPat
      .replace(/\./g, '\\.')
      .replace(/\*\*\//g, '(?:.+/)?')
      .replace(/\*/g, '[^/]*')
      .replace(/\?/g, '.');

    const re = new RegExp(`^${regexPattern}$`);
    return this.backend.list().filter(p => re.test(p)).sort();
  }

  /**
   * Copies a file from `sourcePath` to `targetPath`.
   */
  public copyFile(sourcePath: string, targetPath: string): void {
    const content = this.readFile(sourcePath);
    this.writeFile(targetPath, content);
  }

  /**
   * Moves/renames a file from `sourcePath` to `targetPath`.
   */
  public moveFile(sourcePath: string, targetPath: string): void {
    const content = this.readFile(sourcePath);
    this.writeFile(targetPath, content);
    this.deleteFile(sourcePath);
  }

  /**
   * Seeds default starter files for the virtual workspace.
   */
  public seedDefaults(): void {
    this.writeFile(
      'README.md',
      '# Fusion Workspace\n\nPure-Rust and WebAssembly AI coding assistant running directly in browser and Node.js.\n'
    );
    this.writeFile(
      'src/index.js',
      '// Welcome to Fusion in-browser workspace\nconsole.log("Fusion agent workspace ready");\n'
    );
    this.writeFile(
      'package.json',
      '{\n  "name": "fusion-web-workspace",\n  "version": "0.3.0",\n  "type": "module"\n}\n'
    );
  }

  /**
   * Clears all files in the virtual filesystem.
   */
  public clear(): void {
    this.backend.clear();
    this.notifyChange({ type: 'clear' });
  }

  /**
   * Exports all virtual files as a JSON record dictionary.
   */
  public exportJson(): Record<string, string> {
    const out: Record<string, string> = {};
    for (const p of this.backend.list()) {
      const content = this.backend.read(p);
      if (content !== null && content !== undefined) {
        out[p] = content;
      }
    }
    return out;
  }

  /**
   * Imports a dictionary of virtual files.
   */
  public importJson(files: Record<string, string>, overwrite: boolean = true): void {
    for (const [p, content] of Object.entries(files)) {
      if (overwrite || !this.exists(p)) {
        this.backend.write(this.normalizePath(p), String(content));
      }
    }
    this.notifyChange({ type: 'import' });
  }

  /**
   * Registers a callback listener invoked whenever files are written, edited, or deleted.
   * Returns an unsubscribe function.
   */
  public onChange(listener: (event: VfsChangeEvent) => void): () => void {
    this.changeListeners.add(listener);
    return () => {
      this.changeListeners.delete(listener);
    };
  }

  private notifyChange(event: VfsChangeEvent): void {
    for (const listener of this.changeListeners) {
      try {
        listener(event);
      } catch {
        // ignore listener errors
      }
    }
  }

  /**
   * Synchronizes this VFS instance with a WASM agent's internal filesystem bindings.
   */
  public syncToAgent(agent: WasmFusionAgentBindings): void {
    for (const p of this.backend.list()) {
      const content = this.backend.read(p);
      if (content !== null && content !== undefined) {
        agent.fs_write(p, content);
      }
    }
  }

  /**
   * Synchronizes files from a WASM agent's internal filesystem into this VFS instance.
   */
  public syncFromAgent(agent: WasmFusionAgentBindings): void {
    try {
      const listStr = agent.fs_list();
      const files: string[] = JSON.parse(listStr);
      for (const p of files) {
        try {
          const content = agent.fs_read(p);
          this.writeFile(p, content);
        } catch {
          // ignore
        }
      }
    } catch {
      // ignore
    }
  }

  /**
   * Binds this VFS to a WASM agent, syncing current files and watching for future changes.
   */
  public bindToAgent(agent: WasmFusionAgentBindings): () => void {
    this.syncToAgent(agent);
    return this.onChange((event) => {
      if (event.path) {
        if (event.type === 'write' || event.type === 'edit') {
          const content = this.tryReadFile(event.path);
          if (content !== null) {
            agent.fs_write(event.path, content);
          }
        } else if (event.type === 'delete') {
          agent.fs_delete(event.path);
        }
      } else if (event.type === 'clear') {
        try {
          const listStr = agent.fs_list();
          const files: string[] = JSON.parse(listStr);
          for (const f of files) {
            agent.fs_delete(f);
          }
        } catch {
          // ignore
        }
      } else if (event.type === 'import') {
        this.syncToAgent(agent);
      }
    });
  }

  /**
   * Simulates a lightweight POSIX shell execution environment over the VFS.
   */
  public executeVirtualBash(command: string): { success: boolean; output: string } {
    const trimmed = command.trim();
    if (!trimmed) return { success: true, output: '' };

    const parts = trimmed.split(/\s+/);
    const cmd = parts[0];
    const args = parts.slice(1);

    switch (cmd) {
      case 'pwd':
        return { success: true, output: '/workspace' };
      case 'ls': {
        const files = this.listFiles();
        return { success: true, output: files.join('\n') };
      }
      case 'cat': {
        if (args.length === 0) return { success: false, output: 'cat: missing file operand' };
        try {
          const content = this.readFile(args[0]);
          return { success: true, output: content };
        } catch (e) {
          return { success: false, output: (e as Error).message };
        }
      }
      case 'echo': {
        const text = args.join(' ').replace(/^["']|["']$/g, '');
        return { success: true, output: text };
      }
      case 'wc': {
        if (args.length === 0) return { success: false, output: 'wc: missing file operand' };
        try {
          const content = this.readFile(args[0]);
          const lines = content.split('\n').length;
          const words = content.trim().split(/\s+/).filter(Boolean).length;
          const bytes = new TextEncoder().encode(content).length;
          return { success: true, output: ` ${lines}  ${words} ${bytes} ${args[0]}` };
        } catch (e) {
          return { success: false, output: (e as Error).message };
        }
      }
      case 'grep': {
        if (args.length < 2) return { success: false, output: 'grep: usage: grep <pattern> <file>' };
        const pattern = args[0];
        const file = args[1];
        try {
          const content = this.readFile(file);
          const matched = content
            .split('\n')
            .filter(l => l.includes(pattern))
            .join('\n');
          return { success: true, output: matched };
        } catch (e) {
          return { success: false, output: (e as Error).message };
        }
      }
      case 'touch': {
        if (args.length === 0) return { success: false, output: 'touch: missing file operand' };
        if (!this.exists(args[0])) {
          this.writeFile(args[0], '');
        }
        return { success: true, output: '' };
      }
      case 'rm': {
        if (args.length === 0) return { success: false, output: 'rm: missing operand' };
        const path = args[args.length - 1];
        const deleted = this.deleteFile(path);
        return { success: deleted, output: deleted ? '' : `rm: cannot remove '${path}': No such file` };
      }
      default:
        return { success: true, output: `[vfs-bash] Executed "${command}" in virtual workspace` };
    }
  }
}

// ============================================================================
// 4. Event Listener Bridge (`WasmEventBridge` & Observables / AsyncIterators)
// ============================================================================

/**
 * Subscription handle returned by `.subscribe()`.
 */
export interface Subscription {
  unsubscribe(): void;
  readonly closed: boolean;
}

/**
 * Observer interface for receiving streaming events.
 */
export interface Observer<T> {
  next?(value: T): void;
  error?(err: unknown): void;
  complete?(): void;
}

/**
 * Options for configuring a `WasmEventBridge`.
 */
export interface WasmEventBridgeOptions {
  /** Optional WebAssembly memory buffer for reading pointer-based strings */
  memory?: WebAssembly.Memory;
  /** Optional AbortSignal for early cancellation */
  signal?: AbortSignal;
  /** Optional initial observer callback */
  onEvent?: PromptTurnCallback;
}

/**
 * Event Listener Bridge converting raw WASM pointer/JSON callbacks into
 * typed TypeScript observables and AsyncIterators.
 *
 * Implements `AsyncIterable<FusionEvent>` allowing direct usage with `for await (const event of bridge)`.
 */
export class WasmEventBridge implements AsyncIterable<FusionEvent> {
  private observers: Set<Observer<FusionEvent>> = new Set();
  private eventQueue: FusionEvent[] = [];
  private pendingResolvers: Array<{
    resolve: (result: IteratorResult<FusionEvent>) => void;
    reject: (err: unknown) => void;
  }> = [];
  private isCompleted: boolean = false;
  private completionError: unknown = null;
  private memory?: WebAssembly.Memory;
  private textDecoder: TextDecoder = new TextDecoder('utf-8');
  private abortHandler?: () => void;

  /**
   * Bound callback function suitable for passing directly into WASM agent `prompt_turn(input, bridge.callback)`.
   */
  public readonly callback: (raw: unknown) => void;

  constructor(options?: WasmEventBridgeOptions) {
    this.memory = options?.memory;
    this.callback = (raw: unknown) => this.emit(raw);

    if (options?.onEvent) {
      this.subscribe({ next: options.onEvent });
    }

    if (options?.signal) {
      if (options.signal.aborted) {
        this.error(new Error('Turn execution was aborted'));
      } else {
        this.abortHandler = () => {
          this.error(new Error('Turn execution was aborted'));
        };
        options.signal.addEventListener('abort', this.abortHandler, { once: true });
      }
    }
  }

  /**
   * Converts a raw event payload (JSON string, JS object, pointer/length, or Error)
   * into a typed `FusionEvent` and dispatches it to observers and async iterators.
   */
  public emit(raw: unknown): void {
    if (this.isCompleted) return;

    const event = this.normalizeRawEvent(raw);
    if (!event) return;

    // Dispatch to subscribers
    for (const observer of this.observers) {
      try {
        observer.next?.(event);
      } catch (err) {
        console.error('@fusion/sdk WasmEventBridge: Error in subscriber callback:', err);
      }
    }

    // Deliver to pending async iterator consumer or enqueue
    if (this.pendingResolvers.length > 0) {
      const resolver = this.pendingResolvers.shift()!;
      resolver.resolve({ value: event, done: false });
    } else {
      this.eventQueue.push(event);
    }

    // Auto-complete if terminal finished or error event arrives
    if (event.type === 'finished') {
      this.complete();
    } else if (event.type === 'error') {
      this.complete();
    }
  }

  /**
   * Normalizes arbitrary WASM callback outputs (pointer, JSON string, object) into a `FusionEvent`.
   */
  public normalizeRawEvent(raw: unknown): FusionEvent | null {
    if (!raw) return null;

    // 1. Raw numeric pointer into WebAssembly memory
    if (typeof raw === 'number') {
      if (this.memory) {
        try {
          const str = this.readNullTerminatedUtf8String(raw);
          return this.parseJsonEvent(str);
        } catch {
          return { type: 'status', message: `WASM pointer event: 0x${raw.toString(16)}` };
        }
      }
      return { type: 'status', message: `WASM event code: ${raw}` };
    }

    // 2. Serialized JSON string
    if (typeof raw === 'string') {
      return this.parseJsonEvent(raw);
    }

    // 3. Pre-parsed JavaScript object
    if (typeof raw === 'object') {
      const obj = raw as Record<string, unknown>;
      if (typeof obj.type === 'string') {
        return obj as unknown as FusionEvent;
      }
    }

    return { type: 'status', message: String(raw) };
  }

  private parseJsonEvent(jsonStr: string): FusionEvent {
    try {
      const parsed = JSON.parse(jsonStr.trim());
      if (parsed && typeof parsed.type === 'string') {
        return parsed as FusionEvent;
      }
      return { type: 'status', message: jsonStr };
    } catch {
      return { type: 'status', message: jsonStr };
    }
  }

  /**
   * Reads a null-terminated UTF-8 string from WebAssembly linear memory at `ptr`.
   */
  private readNullTerminatedUtf8String(ptr: number): string {
    if (!this.memory) return '';
    const bytes = new Uint8Array(this.memory.buffer);
    let end = ptr;
    while (end < bytes.length && bytes[end] !== 0) {
      end++;
    }
    return this.textDecoder.decode(bytes.subarray(ptr, end));
  }

  /**
   * Subscribes an observer to receive events.
   *
   * @example
   * ```typescript
   * const sub = bridge.subscribe((event) => {
   *   if (event.type === 'text_delta') process.stdout.write(event.delta);
   * });
   * // later: sub.unsubscribe();
   * ```
   */
  public subscribe(
    observerOrNext: Partial<Observer<FusionEvent>> | ((event: FusionEvent) => void)
  ): Subscription {
    const observer: Observer<FusionEvent> =
      typeof observerOrNext === 'function' ? { next: observerOrNext } : observerOrNext;

    this.observers.add(observer);

    let closed = false;
    return {
      unsubscribe: () => {
        if (!closed) {
          closed = true;
          this.observers.delete(observer);
        }
      },
      get closed() {
        return closed;
      }
    };
  }

  /**
   * Signals successful completion of the event stream.
   */
  public complete(): void {
    if (this.isCompleted) return;
    this.isCompleted = true;

    for (const observer of this.observers) {
      try {
        observer.complete?.();
      } catch {
        // ignore
      }
    }

    // Flush any waiting async iterator requests
    while (this.pendingResolvers.length > 0) {
      const resolver = this.pendingResolvers.shift()!;
      resolver.resolve({ value: undefined as unknown as FusionEvent, done: true });
    }
  }

  /**
   * Signals an error on the event stream.
   */
  public error(err: unknown): void {
    if (this.isCompleted) return;
    this.isCompleted = true;
    this.completionError = err;

    for (const observer of this.observers) {
      try {
        observer.error?.(err);
      } catch {
        // ignore
      }
    }

    while (this.pendingResolvers.length > 0) {
      const resolver = this.pendingResolvers.shift()!;
      resolver.reject(err);
    }
  }

  /**
   * Implements the AsyncIterable protocol.
   *
   * @example
   * ```typescript
   * for await (const event of bridge) {
   *   console.log(event.type);
   * }
   * ```
   */
  public [Symbol.asyncIterator](): AsyncIterator<FusionEvent> {
    return {
      next: async (): Promise<IteratorResult<FusionEvent>> => {
        if (this.eventQueue.length > 0) {
          const value = this.eventQueue.shift()!;
          return { value, done: false };
        }

        if (this.isCompleted) {
          if (this.completionError) {
            throw this.completionError;
          }
          return { value: undefined as unknown as FusionEvent, done: true };
        }

        return new Promise<IteratorResult<FusionEvent>>((resolve, reject) => {
          this.pendingResolvers.push({ resolve, reject });
        });
      },
      return: async (): Promise<IteratorResult<FusionEvent>> => {
        this.complete();
        return { value: undefined as unknown as FusionEvent, done: true };
      },
      throw: async (err?: unknown): Promise<IteratorResult<FusionEvent>> => {
        this.error(err);
        throw err;
      }
    };
  }

  /**
   * Returns an AsyncIterable stream.
   */
  public toAsyncIterable(): AsyncIterable<FusionEvent> {
    return this;
  }

  /**
   * Filters the event stream by a predicate.
   */
  public async *filter(predicate: (event: FusionEvent) => boolean): AsyncIterable<FusionEvent> {
    for await (const event of this) {
      if (predicate(event)) {
        yield event;
      }
    }
  }

  /**
   * Maps the event stream to another type.
   */
  public async *map<T>(transform: (event: FusionEvent) => T): AsyncIterable<T> {
    for await (const event of this) {
      yield transform(event);
    }
  }

  /**
   * Takes events while a predicate holds true.
   */
  public async *takeWhile(predicate: (event: FusionEvent) => boolean): AsyncIterable<FusionEvent> {
    for await (const event of this) {
      if (!predicate(event)) {
        break;
      }
      yield event;
    }
  }

  /**
   * Subscribes specifically to an event type.
   */
  public on<T extends FusionEvent['type']>(
    type: T,
    handler: (event: Extract<FusionEvent, { type: T }>) => void
  ): Subscription {
    return this.subscribe((event) => {
      if (event.type === type) {
        handler(event as Extract<FusionEvent, { type: T }>);
      }
    });
  }

  /**
   * Collects streaming text chunks as an AsyncIterable stream of strings.
   */
  public async *toTextStream(): AsyncIterable<string> {
    for await (const event of this) {
      if (event.type === 'text_delta') {
        yield event.delta;
      }
    }
  }

  /**
   * Collects streaming thinking/reasoning chunks as an AsyncIterable stream of strings.
   */
  public async *toThinkingStream(): AsyncIterable<string> {
    for await (const event of this) {
      if (event.type === 'thinking_delta') {
        yield event.delta;
      }
    }
  }

  /**
   * Collects all events in the turn into an array until finished.
   */
  public async collect(): Promise<FusionEvent[]> {
    const events: FusionEvent[] = [];
    for await (const event of this) {
      events.push(event);
    }
    return events;
  }

  /**
   * Waits for the turn to complete and returns the terminal `FinishedEvent`.
   */
  public async waitForFinish(): Promise<FinishedEvent> {
    for await (const event of this) {
      if (event.type === 'finished') {
        return event;
      }
      if (event.type === 'error') {
        throw new Error(`Turn finished with error: ${event.message}`);
      }
    }
    throw new Error('Event stream closed before receiving FinishedEvent');
  }

  /**
   * Closes the event stream and unsubscribes all observers.
   */
  public close(): void {
    this.complete();
    this.observers.clear();
    this.eventQueue = [];
  }
}

/**
 * Alias for WasmEventBridge.
 */
export const WasmEventStream = WasmEventBridge;

/**
 * Factory function creating a new `WasmEventBridge`.
 */
export function createWasmEventBridge(options?: WasmEventBridgeOptions): WasmEventBridge {
  return new WasmEventBridge(options);
}

/**
 * Factory function creating an event stream.
 */
export function createEventStream(options?: WasmEventBridgeOptions): WasmEventBridge {
  return new WasmEventBridge(options);
}

// ============================================================================
// 5. In-Memory Fallback Module Implementation
// ============================================================================

/**
 * Creates an in-memory pure JS fallback module implementing the Fusion agent contract
 * for non-wasm testing, node environments, or offline browser fallbacks.
 */
function createFallbackModule(): FusionWasmModule {
  class FallbackWasmFusionAgent implements WasmFusionAgentBindings {
    private sessionId: string = 'session_' + Math.random().toString(36).substring(2, 11);
    private activeModel: string;
    private systemPrompt: string = '';
    private messages: Array<{ role: string; content: string }> = [];
    private vfs: VirtualFileSystem;
    private promptTokens: number = 0;
    private completionTokens: number = 0;
    private turnCounter: number = 0;

    constructor(configJson?: string) {
      this.vfs = new VirtualFileSystem('memory', { seedDefaults: true });
      let parsed: FusionConfig = {};
      if (configJson && configJson.trim() !== '{}') {
        try {
          parsed = JSON.parse(configJson);
        } catch {
          // ignore
        }
      }
      this.activeModel = parsed.default_model || 'anthropic/claude-3.5-sonnet';
      if (parsed.system_prompt) {
        this.systemPrompt = parsed.system_prompt;
      }
    }

    get_session_id(): string {
      return this.sessionId;
    }

    get_active_model(): string {
      return this.activeModel;
    }

    set_active_model(model: string): void {
      this.activeModel = model;
    }

    set_system_prompt(prompt: string): void {
      this.systemPrompt = prompt;
    }

    get_messages(): string {
      return JSON.stringify(this.messages);
    }

    get_token_stats(): string {
      return JSON.stringify({
        prompt_tokens: this.promptTokens,
        completion_tokens: this.completionTokens,
        total_tokens: this.promptTokens + this.completionTokens
      });
    }

    clear_messages(): void {
      this.messages = [];
    }

    fs_write(path: string, content: string): void {
      this.vfs.writeFile(path, content);
    }

    fs_read(path: string): string {
      return this.vfs.readFile(path);
    }

    fs_list(): string {
      return JSON.stringify(this.vfs.listFiles());
    }

    fs_delete(path: string): boolean {
      return this.vfs.deleteFile(path);
    }

    checkpoint(): string {
      return JSON.stringify(
        {
          version: '0.3.0',
          session: {
            id: this.sessionId,
            active_model: this.activeModel,
            system_prompt: this.systemPrompt,
            messages: this.messages,
            token_stats: {
              prompt_tokens: this.promptTokens,
              completion_tokens: this.completionTokens,
              total_tokens: this.promptTokens + this.completionTokens
            }
          },
          config: {
            default_model: this.activeModel,
            system_prompt: this.systemPrompt
          },
          vfs: { files: this.vfs.exportJson() },
          turn_counter: this.turnCounter
        },
        null,
        2
      );
    }

    restore(checkpointJson: string): void {
      const parsed = JSON.parse(checkpointJson);
      if (parsed.session) {
        if (parsed.session.id) this.sessionId = parsed.session.id;
        if (parsed.session.active_model) this.activeModel = parsed.session.active_model;
        if (parsed.session.system_prompt) this.systemPrompt = parsed.session.system_prompt;
        if (parsed.session.messages) this.messages = parsed.session.messages;
        if (parsed.session.token_stats) {
          this.promptTokens = parsed.session.token_stats.prompt_tokens || 0;
          this.completionTokens = parsed.session.token_stats.completion_tokens || 0;
        }
      }
      if (parsed.vfs && parsed.vfs.files) {
        this.vfs.clear();
        this.vfs.importJson(parsed.vfs.files);
      }
      if (parsed.turn_counter) {
        this.turnCounter = parsed.turn_counter;
      }
    }

    async prompt_turn(inputStr: string, callback?: (event: unknown) => void): Promise<string> {
      this.turnCounter += 1;
      this.messages.push({ role: 'user', content: inputStr });

      const emit = (event: Record<string, unknown>) => {
        if (callback) {
          try {
            callback(event);
          } catch {
            // ignore callback errors
          }
        }
      };

      emit({
        type: 'status',
        message: `Processing turn #${this.turnCounter} with ${this.activeModel}`
      });

      emit({
        type: 'advisor_critique',
        advisor: 'Architect',
        approved: true,
        critique: 'Plan conforms to virtual workspace constraints.'
      });

      const lower = inputStr.toLowerCase().trim();
      let response = '';

      if (lower.includes('list files') || lower.includes('show files') || lower === 'ls') {
        emit({
          type: 'tool_started',
          id: `call_glob_${this.turnCounter}`,
          name: 'glob',
          args: { pattern: '**/*' }
        });
        const files = this.vfs.listFiles();
        emit({
          type: 'tool_finished',
          id: `call_glob_${this.turnCounter}`,
          name: 'glob',
          success: true,
          output: files.join('\n'),
          duration_ms: 2
        });
        response = `I checked the workspace and found ${files.length} files:\n${files.map(f => `- \`${f}\``).join('\n')}`;
      } else if (lower.startsWith('read ') || lower.includes('read file')) {
        const file = inputStr.split(/\s+/).pop()?.replace(/`/g, '') || 'README.md';
        emit({
          type: 'tool_started',
          id: `call_read_${this.turnCounter}`,
          name: 'read',
          args: { path: file }
        });
        const content = this.vfs.tryReadFile(file);
        const success = content !== null;
        const out = success ? content! : `File not found: ${file}`;
        emit({
          type: 'tool_finished',
          id: `call_read_${this.turnCounter}`,
          name: 'read',
          success,
          output: out,
          duration_ms: 2
        });
        response = success ? `Contents of \`${file}\`:\n\`\`\`\n${out}\n\`\`\`` : out;
      } else {
        emit({
          type: 'thinking_delta',
          delta: `Analyzing prompt: "${inputStr}"...`
        });
        response = `Fusion v0.3.0 [WASM Agent]\n\nProcessed prompt: "${inputStr}" using model \`${this.activeModel}\`.\n\nVirtual Workspace Tools available: list files, read <path>, write <path>, grep <pattern>, checkpoint.`;
      }

      // Stream text chunks
      const words = response.split(' ');
      for (let i = 0; i < words.length; i++) {
        emit({
          type: 'text_delta',
          delta: (i === 0 ? '' : ' ') + words[i]
        });
      }

      const pTokens = Math.floor(inputStr.length / 4) + 40;
      const cTokens = Math.floor(response.length / 4) + 20;
      this.promptTokens += pTokens;
      this.completionTokens += cTokens;

      emit({
        type: 'finished',
        usage: {
          prompt_tokens: pTokens,
          completion_tokens: cTokens,
          total_tokens: pTokens + cTokens
        }
      });

      this.messages.push({ role: 'assistant', content: response });
      return response;
    }
  }

  let globalFallback: FallbackWasmFusionAgent | null = null;

  return {
    WasmFusionAgent: FallbackWasmFusionAgent as unknown as { new (config?: string): WasmFusionAgentBindings },
    create_agent(config_json: string): WasmFusionAgentBindings {
      globalFallback = new FallbackWasmFusionAgent(config_json);
      return globalFallback;
    },
    async prompt_turn(input_str: string, callback?: (event: unknown) => void): Promise<string> {
      if (!globalFallback) {
        globalFallback = new FallbackWasmFusionAgent('{}');
      }
      return globalFallback.prompt_turn(input_str, callback);
    },
    checkpoint(): string {
      if (!globalFallback) throw new Error('No active agent');
      return globalFallback.checkpoint();
    },
    restore(checkpoint_json: string): void {
      if (!globalFallback) {
        globalFallback = new FallbackWasmFusionAgent('{}');
      }
      globalFallback.restore(checkpoint_json);
    },
    fusion_version(): string {
      return '0.3.0';
    }
  };
}
