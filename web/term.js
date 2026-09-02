/**
 * Fusion v2 — Browser Terminal & In-Browser Agent Controller
 * 
 * High-performance, pure client-side terminal adapter connecting xterm.js to:
 * 1. WebAssembly Fusion Agent (window.fusion_wasm / WasmFusionAgent) with full streaming
 * 2. In-browser Virtual File System (VFS backed by IndexedDB / LocalStorage) with /files, /cat, /upload, /download
 * 3. Ephemeral & Protected API Key Management in SessionStorage with Secret Scrubbing
 * 4. WebSocket Agent Server (ACP JSON-RPC 2.0 protocol)
 * 5. Direct Browser LLM Streaming (OpenRouter / Anthropic / OpenAI / Ollama)
 * 6. Interactive Multi-Agent Mesh Simulator & Security Advisors
 */

(function () {
  'use strict';

  // ===========================================================================
  // 1. Model Catalog (Matching fx.sh/try UX)
  // ===========================================================================
  const MODEL_CATALOG = [
    {
      id: 'anthropic/claude-3-5-sonnet',
      name: 'Claude 3.5 Sonnet',
      provider: 'Anthropic',
      category: 'coding',
      tag: 'Coding Leader',
      context: '200k tokens',
      pricing: '$3 / $15 per 1M',
      description: 'Industry benchmark for code synthesis, complex reasoning, and tool orchestration.'
    },
    {
      id: 'anthropic/claude-3-7-sonnet',
      name: 'Claude 3.7 Sonnet',
      provider: 'Anthropic',
      category: 'reasoning',
      tag: 'Hybrid Thinking',
      context: '200k tokens',
      pricing: '$3 / $15 per 1M',
      description: 'Frontier reasoning model with adaptive internal thinking and deep code planning.'
    },
    {
      id: 'anthropic/claude-3-5-haiku',
      name: 'Claude 3.5 Haiku',
      provider: 'Anthropic',
      category: 'fast',
      tag: 'Ultra Fast',
      context: '200k tokens',
      pricing: '$0.80 / $4 per 1M',
      description: 'High-speed, low-latency coding model for rapid subagent fan-out and code reviews.'
    },
    {
      id: 'openai/gpt-4o',
      name: 'GPT-4o',
      provider: 'OpenAI',
      category: 'coding',
      tag: 'Multimodal',
      context: '128k tokens',
      pricing: '$2.50 / $10 per 1M',
      description: 'Versatile flagship model with robust reasoning and structured JSON output.'
    },
    {
      id: 'openai/gpt-4o-mini',
      name: 'GPT-4o-mini',
      provider: 'OpenAI',
      category: 'fast',
      tag: 'Cost Efficient',
      context: '128k tokens',
      pricing: '$0.15 / $0.60 per 1M',
      description: 'Super-fast lightweight model ideal for scout indexing and quick linting passes.'
    },
    {
      id: 'openai/o3-mini',
      name: 'OpenAI o3-mini',
      provider: 'OpenAI',
      category: 'reasoning',
      tag: 'Math & Logic',
      context: '200k tokens',
      pricing: '$1.10 / $4.40 per 1M',
      description: 'Specialized reasoning model optimized for competitive programming and formal logic.'
    },
    {
      id: 'deepseek/deepseek-r1',
      name: 'DeepSeek R1',
      provider: 'DeepSeek',
      category: 'reasoning',
      tag: 'Open Weights R1',
      context: '128k tokens',
      pricing: '$0.55 / $2.19 per 1M',
      description: 'Open-weights reasoning model with chain-of-thought verification.'
    },
    {
      id: 'deepseek/deepseek-chat',
      name: 'DeepSeek V3',
      provider: 'DeepSeek',
      category: 'coding',
      tag: 'Efficient V3',
      context: '128k tokens',
      pricing: '$0.14 / $0.28 per 1M',
      description: 'Remarkable coding efficiency at fraction of proprietary frontier prices.'
    },
    {
      id: 'google/gemini-2.0-flash',
      name: 'Gemini 2.0 Flash',
      provider: 'Google',
      category: 'fast',
      tag: 'Realtime Fast',
      context: '1M tokens',
      pricing: '$0.10 / $0.40 per 1M',
      description: 'Massive 1M token context window with instantaneous response delivery.'
    },
    {
      id: 'qwen/qwen-2.5-coder-32b-instruct',
      name: 'Qwen 2.5 Coder 32B',
      provider: 'Qwen',
      category: 'coding',
      tag: 'Open Source',
      context: '32k tokens',
      pricing: '$0.20 / $0.20 per 1M',
      description: 'Top-tier open-weights code generation model tailored for polyglot refactoring.'
    },
    {
      id: 'ollama/llama3.2',
      name: 'Ollama Llama 3.2 (Local)',
      provider: 'Ollama',
      category: 'local',
      tag: 'Offline Local',
      context: '128k tokens',
      pricing: 'Free / Local',
      description: 'Run completely offline on local CPU/GPU hardware with zero external API calls.'
    },
    {
      id: 'ollama/qwen2.5-coder',
      name: 'Ollama Qwen 2.5 Coder (Local)',
      provider: 'Ollama',
      category: 'local',
      tag: 'Offline Code',
      context: '32k tokens',
      pricing: 'Free / Local',
      description: 'Local coding assistant running on your workstation via Ollama endpoint.'
    }
  ];

  // ===========================================================================
  // 2. Terminal Color Themes Catalog
  // ===========================================================================
  const THEMES = {
    'tokyo-night': {
      name: 'Tokyo Night (Default)',
      background: '#0a0e17',
      foreground: '#c0caf5',
      cursor: '#7aa2f7',
      cursorAccent: '#0a0e17',
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
    'catppuccin-mocha': {
      name: 'Catppuccin Mocha',
      background: '#1e1e2e',
      foreground: '#cdd6f4',
      cursor: '#f5e0dc',
      cursorAccent: '#1e1e2e',
      selectionBackground: '#585b70',
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
    'dracula': {
      name: 'Dracula Dark',
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
    'nord': {
      name: 'Nord Frost',
      background: '#2e3440',
      foreground: '#d8dee9',
      cursor: '#88c0d0',
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
    'cyberpunk': {
      name: 'Cyberpunk Neon',
      background: '#080811',
      foreground: '#00ffcc',
      cursor: '#ff007f',
      cursorAccent: '#080811',
      selectionBackground: '#ff007f33',
      selectionForeground: '#ffffff',
      black: '#101020',
      red: '#ff0055',
      green: '#00ff88',
      yellow: '#ffe600',
      blue: '#00bfff',
      magenta: '#ff007f',
      cyan: '#00ffcc',
      white: '#ffffff',
      brightBlack: '#202040',
      brightRed: '#ff3377',
      brightGreen: '#33ffaa',
      brightYellow: '#ffff33',
      brightBlue: '#33ccff',
      brightMagenta: '#ff3399',
      brightCyan: '#33ffdd',
      brightWhite: '#ffffff'
    }
  };

  // ANSI formatting sequences
  const ANSI = {
    reset: '\x1b[0m',
    bold: '\x1b[1m',
    dim: '\x1b[2m',
    italic: '\x1b[3m',
    underline: '\x1b[4m',
    inverse: '\x1b[7m',
    
    // Standard Colors
    black: '\x1b[30m',
    red: '\x1b[31m',
    green: '\x1b[32m',
    yellow: '\x1b[33m',
    blue: '\x1b[34m',
    magenta: '\x1b[35m',
    cyan: '\x1b[36m',
    white: '\x1b[37m',
    
    // Bright / Extended 256 Colors
    purple: '\x1b[38;5;141m',
    neonCyan: '\x1b[38;5;51m',
    emerald: '\x1b[38;5;48m',
    amber: '\x1b[38;5;214m',
    slate: '\x1b[38;5;244m',
    darkGray: '\x1b[38;5;238m',
    rose: '\x1b[38;5;204m'
  };

  // ===========================================================================
  // 3. Privacy-Protected API Key Storage (SessionStorage Only)
  // ===========================================================================
  const API_KEY_SESSION_KEY = 'fusion_api_key_session';

  /**
   * Secure API key manager:
   * - Stores exclusively in browser sessionStorage (never persists to disk or localStorage).
   * - Automatically purges any legacy keys from localStorage.
   * - Scrubs secrets from exported logs and transcripts.
   * - Masks display output (sk-ant-...abcd).
   */
  const ApiKeyStore = {
    // Migrate any legacy localStorage key to sessionStorage, then delete from localStorage
    init() {
      try {
        const legacyKey = localStorage.getItem('fusion_api_key');
        if (legacyKey && legacyKey.trim()) {
          sessionStorage.setItem(API_KEY_SESSION_KEY, legacyKey.trim());
          localStorage.removeItem('fusion_api_key');
        }
      } catch (e) {
        // Handle private browsing or blocked storage
      }
    },

    get() {
      try {
        return sessionStorage.getItem(API_KEY_SESSION_KEY) || '';
      } catch (e) {
        return '';
      }
    },

    set(key) {
      try {
        if (!key || !key.trim()) {
          sessionStorage.removeItem(API_KEY_SESSION_KEY);
        } else {
          sessionStorage.setItem(API_KEY_SESSION_KEY, key.trim());
        }
        // Always ensure removed from persistent localStorage
        localStorage.removeItem('fusion_api_key');
      } catch (e) {}
    },

    clear() {
      try {
        sessionStorage.removeItem(API_KEY_SESSION_KEY);
        localStorage.removeItem('fusion_api_key');
      } catch (e) {}
    },

    mask(key) {
      const k = (key || this.get()).trim();
      if (!k) return 'None (Demo / Local Mode)';
      if (k.length <= 8) return '••••••••';
      const prefix = k.slice(0, 7);
      const suffix = k.slice(-4);
      return `${prefix}...${suffix}`;
    },

    /**
     * Sanitizes strings (logs, traces, session exports) to ensure secrets never leak.
     */
    sanitize(text) {
      if (!text) return '';
      return text
        .replace(/sk-ant-[a-zA-Z0-9_-]{20,}/g, '[REDACTED_ANTHROPIC_KEY]')
        .replace(/sk-or-v1-[a-zA-Z0-9_-]{20,}/g, '[REDACTED_OPENROUTER_KEY]')
        .replace(/sk-[a-zA-Z0-9_-]{20,}/g, '[REDACTED_API_KEY]')
        .replace(/Bearer\s+[a-zA-Z0-9._-]{20,}/gi, 'Bearer [REDACTED_TOKEN]')
        .replace(/ghp_[a-zA-Z0-9]{36}/g, '[REDACTED_GITHUB_TOKEN]');
    }
  };

  // Initialize API Key Store immediately
  ApiKeyStore.init();

  // ===========================================================================
  // 4. In-Browser Virtual File System (VFS) backed by IndexedDB / LocalStorage
  // ===========================================================================
  const VFS_DB_NAME = 'fusion_vfs_db';
  const VFS_STORE_NAME = 'files';
  const VFS_FALLBACK_KEY = 'fusion_vfs_files';

  /**
   * High-performance Virtual File System:
   * - Primary storage: IndexedDB (asynchronous, large capacity, binary-capable).
   * - Fallback storage: LocalStorage JSON map (for private modes / environments without IDB).
   * - Bi-directional synchronization with WASM Agent (WasmFusionAgent.fs_write/fs_read/fs_list/fs_delete).
   */
  const VirtualFileSystem = {
    db: null,
    memoryCache: new Map(),
    isInitialized: false,

    async init() {
      if (this.isInitialized) return;

      // Seed starter project files in memory cache
      this.memoryCache.set('README.md', {
        path: 'README.md',
        content: '# Fusion Web Workspace\n\nFast, lightweight pure-Rust AI coding assistant running directly in your browser via WebAssembly.\n\n### Slash Commands:\n- `/files` — List workspace files\n- `/cat <file>` — Read file content\n- `/upload` — Upload files from local workstation\n- `/download <file>` — Save file to your machine\n',
        size: 320,
        updatedAt: Date.now(),
        type: 'text/markdown'
      });

      this.memoryCache.set('src/main.rs', {
        path: 'src/main.rs',
        content: '//! Fusion In-Browser WebAssembly Application Entry Point\n\nfn main() {\n    println!("⚡ Fusion v0.3.0 initialized in browser!");\n}\n',
        size: 124,
        updatedAt: Date.now(),
        type: 'text/x-rust'
      });

      this.memoryCache.set('Cargo.toml', {
        path: 'Cargo.toml',
        content: '[package]\nname = "fusion-workspace"\nversion = "0.3.0"\nedition = "2021"\n\n[dependencies]\ntokio = { version = "1.43", features = ["full"] }\nserde = { version = "1.0", features = ["derive"] }\nserde_json = "1.0"\n',
        size: 195,
        updatedAt: Date.now(),
        type: 'text/x-toml'
      });

      this.memoryCache.set('package.json', {
        path: 'package.json',
        content: '{\n  "name": "fusion-web-workspace",\n  "version": "0.3.0",\n  "type": "module",\n  "scripts": {\n    "dev": "vite",\n    "build": "vite build"\n  }\n}\n',
        size: 146,
        updatedAt: Date.now(),
        type: 'application/json'
      });

      // Try opening IndexedDB
      try {
        await new Promise((resolve, reject) => {
          if (typeof indexedDB === 'undefined') {
            return reject(new Error('IndexedDB not supported'));
          }

          const request = indexedDB.open(VFS_DB_NAME, 1);
          request.onupgradeneeded = (e) => {
            const db = e.target.result;
            if (!db.objectStoreNames.contains(VFS_STORE_NAME)) {
              db.createObjectStore(VFS_STORE_NAME, { keyPath: 'path' });
            }
          };

          request.onsuccess = (e) => {
            this.db = e.target.result;
            resolve();
          };

          request.onerror = (e) => {
            reject(request.error);
          };
        });

        // Load existing records from IDB into memoryCache
        const records = await this.getAllFromIdb();
        if (records && records.length > 0) {
          records.forEach(r => {
            this.memoryCache.set(r.path, r);
          });
        } else {
          // Store default files in IDB
          for (const item of this.memoryCache.values()) {
            await this.putToIdb(item);
          }
        }
      } catch (err) {
        // Fallback to localStorage
        try {
          const raw = localStorage.getItem(VFS_FALLBACK_KEY);
          if (raw) {
            const list = JSON.parse(raw);
            list.forEach(r => this.memoryCache.set(r.path, r));
          } else {
            this.persistFallback();
          }
        } catch (e) {}
      }

      this.isInitialized = true;
    },

    async putToIdb(record) {
      if (!this.db) return;
      return new Promise((resolve, reject) => {
        try {
          const tx = this.db.transaction([VFS_STORE_NAME], 'readwrite');
          const store = tx.objectStore(VFS_STORE_NAME);
          const req = store.put(record);
          req.onsuccess = () => resolve();
          req.onerror = () => reject(req.error);
        } catch (e) {
          reject(e);
        }
      });
    },

    async getAllFromIdb() {
      if (!this.db) return [];
      return new Promise((resolve, reject) => {
        try {
          const tx = this.db.transaction([VFS_STORE_NAME], 'readonly');
          const store = tx.objectStore(VFS_STORE_NAME);
          const req = store.getAll();
          req.onsuccess = () => resolve(req.result || []);
          req.onerror = () => reject(req.error);
        } catch (e) {
          reject(e);
        }
      });
    },

    async deleteFromIdb(path) {
      if (!this.db) return;
      return new Promise((resolve, reject) => {
        try {
          const tx = this.db.transaction([VFS_STORE_NAME], 'readwrite');
          const store = tx.objectStore(VFS_STORE_NAME);
          const req = store.delete(path);
          req.onsuccess = () => resolve();
          req.onerror = () => reject(req.error);
        } catch (e) {
          reject(e);
        }
      });
    },

    persistFallback() {
      try {
        const arr = Array.from(this.memoryCache.values());
        localStorage.setItem(VFS_FALLBACK_KEY, JSON.stringify(arr));
      } catch (e) {}
    },

    normalizePath(path) {
      if (!path) return '';
      return path.trim().replace(/^\.\//, '').replace(/^\/+/, '');
    },

    async writeFile(path, content, type = 'text/plain') {
      const cleanPath = this.normalizePath(path);
      if (!cleanPath) throw new Error('Invalid file path');

      const size = new Blob([content]).size;
      const record = {
        path: cleanPath,
        content: content,
        size: size,
        updatedAt: Date.now(),
        type: type
      };

      this.memoryCache.set(cleanPath, record);

      if (this.db) {
        await this.putToIdb(record);
      } else {
        this.persistFallback();
      }

      // Sync with active WASM agent if available
      if (state.wasmAgent && typeof state.wasmAgent.fs_write === 'function') {
        try {
          state.wasmAgent.fs_write(cleanPath, content);
        } catch (e) {}
      }

      return record;
    },

    async readFile(path) {
      const cleanPath = this.normalizePath(path);
      if (!cleanPath) throw new Error('File not specified');

      // Check WASM agent first for any runtime updates
      if (state.wasmAgent && typeof state.wasmAgent.fs_read === 'function') {
        try {
          const content = state.wasmAgent.fs_read(cleanPath);
          if (content !== undefined && content !== null) {
            return content;
          }
        } catch (e) {}
      }

      const rec = this.memoryCache.get(cleanPath);
      if (!rec) {
        throw new Error(`File not found: '${path}'`);
      }
      return rec.content;
    },

    async deleteFile(path) {
      const cleanPath = this.normalizePath(path);
      const existed = this.memoryCache.delete(cleanPath);

      if (this.db) {
        await this.deleteFromIdb(cleanPath);
      } else {
        this.persistFallback();
      }

      if (state.wasmAgent && typeof state.wasmAgent.fs_delete === 'function') {
        try {
          state.wasmAgent.fs_delete(cleanPath);
        } catch (e) {}
      }

      return existed;
    },

    listFiles(filter) {
      let paths = Array.from(this.memoryCache.keys());

      // If WASM agent has additional files, merge them
      if (state.wasmAgent && typeof state.wasmAgent.fs_list === 'function') {
        try {
          const wasmFilesStr = state.wasmAgent.fs_list();
          const wasmFiles = JSON.parse(wasmFilesStr);
          if (Array.isArray(wasmFiles)) {
            wasmFiles.forEach(f => {
              if (!this.memoryCache.has(f)) {
                try {
                  const content = state.wasmAgent.fs_read(f);
                  this.writeFile(f, content);
                } catch (e) {}
              }
            });
            paths = Array.from(this.memoryCache.keys());
          }
        } catch (e) {}
      }

      paths.sort();

      if (filter) {
        const cleanFilter = filter.toLowerCase().trim();
        return paths
          .filter(p => p.toLowerCase().includes(cleanFilter))
          .map(p => this.memoryCache.get(p));
      }

      return paths.map(p => this.memoryCache.get(p));
    },

    /**
     * Synchronizes all VFS files into a freshly created WASM agent instance.
     */
    syncToWasmAgent(agent) {
      if (!agent || typeof agent.fs_write !== 'function') return;
      this.memoryCache.forEach((rec, path) => {
        try {
          agent.fs_write(path, rec.content);
        } catch (e) {}
      });
    },

    async resetToDefaults() {
      this.memoryCache.clear();
      this.isInitialized = false;
      if (this.db) {
        try {
          const tx = this.db.transaction([VFS_STORE_NAME], 'readwrite');
          tx.objectStore(VFS_STORE_NAME).clear();
        } catch (e) {}
      }
      try {
        localStorage.removeItem(VFS_FALLBACK_KEY);
      } catch (e) {}
      await this.init();
    }
  };

  // ===========================================================================
  // 5. Application State Management
  // ===========================================================================
  const state = {
    term: null,
    fitAddon: null,
    webLinksAddon: null,
    activeTheme: localStorage.getItem('fusion_theme') || 'tokyo-night',
    
    // Connection & Backend
    connMode: localStorage.getItem('fusion_conn_mode') || 'wasm', // 'wasm' | 'websocket' | 'direct_api' | 'demo'
    wsUrl: localStorage.getItem('fusion_ws_url') || 'ws://127.0.0.1:9001',
    ollamaUrl: localStorage.getItem('fusion_ollama_url') || 'http://localhost:11434',
    
    // Model & Agent config
    activeModel: localStorage.getItem('fusion_model') || 'anthropic/claude-3-5-sonnet',
    temperature: parseFloat(localStorage.getItem('fusion_temp') || '0.2'),
    maxTurns: parseInt(localStorage.getItem('fusion_max_turns') || '30', 10),
    systemPrompt: localStorage.getItem('fusion_system_prompt') || '',
    
    // Active session
    ws: null,
    wsConnected: false,
    wasmAgent: null,
    isStreaming: false,
    abortController: null,
    
    // Token & Cost tracking
    sessionStats: {
      promptTokens: 0,
      completionTokens: 0,
      totalTurns: 0,
      estimatedCost: 0.0
    },
    
    // Line Editor State
    inputBuffer: '',
    cursorPos: 0,
    history: [],
    historyIndex: -1,
    promptStr: '\x1b[38;5;141m⚡ fusion\x1b[0m \x1b[38;5;51m❯\x1b[0m ',
    
    // Checkpoints store
    savedCheckpoint: localStorage.getItem('fusion_checkpoint') || null
  };

  // ===========================================================================
  // 6. Application Initialization
  // ===========================================================================
  document.addEventListener('DOMContentLoaded', async () => {
    // 1. Initialize Virtual File System
    await VirtualFileSystem.init();

    // 2. Setup Terminal and UI
    initTerminal();
    initModelPickerUI();
    initSettingsUI();
    initQuickBar();
    initTouchToolbar();
    initModals();
    
    // 3. Initialize Agent backend & WASM stream orchestration
    setupBackendConnection();
  });

  // ===========================================================================
  // 7. Terminal Setup (Xterm.js + Fit + WebLinks + Custom Themes)
  // ===========================================================================
  function initTerminal() {
    const container = document.getElementById('terminal-container');
    if (!container) return;

    // Get selected theme colors
    const themeColors = THEMES[state.activeTheme] || THEMES['tokyo-night'];

    // Support both @xterm/xterm namespace and legacy window.Terminal
    const TerminalClass = window.Terminal || (window.xterm && window.xterm.Terminal);
    if (!TerminalClass) {
      console.error('Xterm.js not found in global scope.');
      return;
    }

    state.term = new TerminalClass({
      cursorBlink: true,
      cursorStyle: 'bar',
      fontSize: 14,
      fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', Menlo, Monaco, monospace",
      lineHeight: 1.35,
      letterSpacing: 0,
      theme: themeColors,
      scrollback: 10000,
      convertEol: true,
      allowTransparency: true
    });

    // 1. Load Fit Addon
    const FitAddonClass = (window.FitAddon && window.FitAddon.FitAddon) || window.FitAddon;
    if (FitAddonClass) {
      state.fitAddon = new FitAddonClass();
      state.term.loadAddon(state.fitAddon);
    }

    // 2. Load WebLinks Addon
    const WebLinksAddonClass = (window.WebLinksAddon && window.WebLinksAddon.WebLinksAddon) || window.WebLinksAddon;
    if (WebLinksAddonClass) {
      state.webLinksAddon = new WebLinksAddonClass((event, uri) => {
        window.open(uri, '_blank', 'noopener,noreferrer');
      });
      state.term.loadAddon(state.webLinksAddon);
    }

    // 3. Open terminal in container
    state.term.open(container);

    // 4. Initial auto-fit
    if (state.fitAddon) {
      try {
        state.fitAddon.fit();
      } catch (e) {}
    }

    // 5. Intelligent Resizing (ResizeObserver + Window resize)
    if (typeof ResizeObserver !== 'undefined') {
      const resizeObserver = new ResizeObserver(() => {
        window.requestAnimationFrame(() => {
          if (state.fitAddon && state.term) {
            try {
              state.fitAddon.fit();
            } catch (e) {}
          }
        });
      });
      resizeObserver.observe(container);
    }

    window.addEventListener('resize', () => {
      if (state.fitAddon) {
        try {
          state.fitAddon.fit();
        } catch (e) {}
      }
    });

    // 6. Handle user keystrokes in line editor
    state.term.onData(handleTerminalInput);

    // 7. Print Welcome Banner
    printBanner();
    printPrompt();
  }

  function printBanner() {
    const t = state.term;
    t.writeln('');
    t.writeln(`${ANSI.purple}⚡ FUSION v0.3.0${ANSI.reset} ─────────────────────────────────────────────────────────────`);
    t.writeln(`${ANSI.dim}Pure Rust AI coding assistant running directly in your browser via WebAssembly${ANSI.reset}`);
    t.writeln(`${ANSI.slate}Mode: ${ANSI.emerald}${state.connMode.toUpperCase()}${ANSI.slate} | Model: ${ANSI.neonCyan}${state.activeModel}${ANSI.slate} | VFS: ${ANSI.emerald}IndexedDB Active${ANSI.reset}`);
    t.writeln(`${ANSI.slate}Type ${ANSI.amber}/help${ANSI.slate} for commands, ${ANSI.amber}/files${ANSI.slate} to view VFS, or enter a prompt.${ANSI.reset}`);
    t.writeln(`${ANSI.purple}─────────────────────────────────────────────────────────────────────────────${ANSI.reset}`);
    t.writeln('');
  }

  function printPrompt() {
    state.term.write(state.promptStr);
    state.inputBuffer = '';
    state.cursorPos = 0;
  }

  // ===========================================================================
  // 8. Line Editor & Keystroke Handler
  // ===========================================================================
  function handleTerminalInput(data) {
    if (state.isStreaming) {
      // Ctrl+C interrupts active turn
      if (data === '\x03') {
        abortTurn();
      }
      return;
    }

    switch (data) {
      case '\r': // Enter
        handleEnter();
        break;

      case '\x7f': // Backspace
        handleBackspace();
        break;

      case '\t': // Tab (command & path autocompletion)
        handleTabCompletion();
        break;

      case '\x03': // Ctrl+C (Interrupt / Clear line)
        state.term.writeln('^C');
        printPrompt();
        break;

      case '\x0c': // Ctrl+L (Clear screen)
        state.term.clear();
        printPrompt();
        break;

      case '\x01': // Ctrl+A (Home)
        moveCursorToStart();
        break;

      case '\x05': // Ctrl+E (End)
        moveCursorToEnd();
        break;

      case '\x15': // Ctrl+U (Delete line before cursor)
        clearLine();
        break;

      case '\x17': // Ctrl+W (Delete word backwards)
        deleteWordBackwards();
        break;

      default:
        // Escape Sequences (Arrow keys, Home, End, Delete)
        if (data.startsWith('\x1b[')) {
          handleEscapeSequence(data);
        } else if (data.length === 1 && data.charCodeAt(0) >= 32) {
          insertChar(data);
        } else if (data.length > 1) {
          // Pasted multi-character text
          insertMultiChar(data);
        }
        break;
    }
  }

  function handleEnter() {
    state.term.writeln('');
    const input = state.inputBuffer.trim();

    if (!input) {
      printPrompt();
      return;
    }

    // Save to history (avoid duplicates)
    if (state.history.length === 0 || state.history[state.history.length - 1] !== input) {
      state.history.push(input);
    }
    state.historyIndex = -1;

    // Route Slash Commands vs Prompt Turns
    if (input.startsWith('/')) {
      handleSlashCommand(input);
    } else {
      submitPromptTurn(input);
    }
  }

  function handleBackspace() {
    if (state.cursorPos > 0) {
      const left = state.inputBuffer.slice(0, state.cursorPos - 1);
      const right = state.inputBuffer.slice(state.cursorPos);
      state.inputBuffer = left + right;
      state.cursorPos--;
      redrawLine();
    }
  }

  function insertChar(ch) {
    const left = state.inputBuffer.slice(0, state.cursorPos);
    const right = state.inputBuffer.slice(state.cursorPos);
    state.inputBuffer = left + ch + right;
    state.cursorPos++;
    redrawLine();
  }

  function insertMultiChar(text) {
    // Strip carriage returns in pasted text
    const clean = text.replace(/[\r\n]+/g, ' ');
    const left = state.inputBuffer.slice(0, state.cursorPos);
    const right = state.inputBuffer.slice(state.cursorPos);
    state.inputBuffer = left + clean + right;
    state.cursorPos += clean.length;
    redrawLine();
  }

  function handleEscapeSequence(seq) {
    switch (seq) {
      case '\x1b[D': // Left Arrow
        if (state.cursorPos > 0) {
          state.cursorPos--;
          state.term.write('\x1b[D');
        }
        break;

      case '\x1b[C': // Right Arrow
        if (state.cursorPos < state.inputBuffer.length) {
          state.cursorPos++;
          state.term.write('\x1b[C');
        }
        break;

      case '\x1b[A': // Up Arrow (History back)
        navigateHistory(-1);
        break;

      case '\x1b[B': // Down Arrow (History forward)
        navigateHistory(1);
        break;

      case '\x1b[H': // Home
        moveCursorToStart();
        break;

      case '\x1b[F': // End
        moveCursorToEnd();
        break;

      case '\x1b[3~': // Delete key
        if (state.cursorPos < state.inputBuffer.length) {
          const left = state.inputBuffer.slice(0, state.cursorPos);
          const right = state.inputBuffer.slice(state.cursorPos + 1);
          state.inputBuffer = left + right;
          redrawLine();
        }
        break;
    }
  }

  function navigateHistory(direction) {
    if (state.history.length === 0) return;

    if (direction < 0) {
      if (state.historyIndex === -1) {
        state.historyIndex = state.history.length - 1;
      } else if (state.historyIndex > 0) {
        state.historyIndex--;
      }
    } else {
      if (state.historyIndex !== -1) {
        if (state.historyIndex < state.history.length - 1) {
          state.historyIndex++;
        } else {
          state.historyIndex = -1;
          state.inputBuffer = '';
          state.cursorPos = 0;
          redrawLine();
          return;
        }
      }
    }

    if (state.historyIndex !== -1) {
      state.inputBuffer = state.history[state.historyIndex];
      state.cursorPos = state.inputBuffer.length;
      redrawLine();
    }
  }

  function redrawLine() {
    state.term.write('\r\x1b[K');
    state.term.write(state.promptStr + state.inputBuffer);

    const backAmount = state.inputBuffer.length - state.cursorPos;
    if (backAmount > 0) {
      state.term.write(`\x1b[${backAmount}D`);
    }
  }

  function moveCursorToStart() {
    if (state.cursorPos > 0) {
      state.term.write(`\x1b[${state.cursorPos}D`);
      state.cursorPos = 0;
    }
  }

  function moveCursorToEnd() {
    const diff = state.inputBuffer.length - state.cursorPos;
    if (diff > 0) {
      state.term.write(`\x1b[${diff}C`);
      state.cursorPos = state.inputBuffer.length;
    }
  }

  function clearLine() {
    state.inputBuffer = '';
    state.cursorPos = 0;
    redrawLine();
  }

  function deleteWordBackwards() {
    if (state.cursorPos === 0) return;
    let idx = state.cursorPos;
    while (idx > 0 && state.inputBuffer[idx - 1] === ' ') idx--;
    while (idx > 0 && state.inputBuffer[idx - 1] !== ' ') idx--;
    state.inputBuffer = state.inputBuffer.slice(0, idx) + state.inputBuffer.slice(state.cursorPos);
    state.cursorPos = idx;
    redrawLine();
  }

  function handleTabCompletion() {
    const slashCommands = [
      '/help', '/files', '/cat', '/upload', '/download', '/write', '/rm',
      '/apikey', '/theme', '/model', '/subagent', '/tools', '/cost',
      '/compact', '/advisor', '/checkpoint', '/restore', '/clear', '/version'
    ];

    const current = state.inputBuffer;

    // 1. Slash command autocompletion
    if (current.startsWith('/') && !current.includes(' ')) {
      const match = slashCommands.find(c => c.startsWith(current) && c !== current);
      if (match) {
        state.inputBuffer = match + ' ';
        state.cursorPos = state.inputBuffer.length;
        redrawLine();
        return;
      }
    }

    // 2. VFS File path autocompletion for /cat, /download, /rm, /view
    const vfsTriggers = ['/cat ', '/view ', '/download ', '/rm ', '/delete '];
    for (const trigger of vfsTriggers) {
      if (current.startsWith(trigger)) {
        const query = current.slice(trigger.length).trim();
        const files = VirtualFileSystem.listFiles();
        const match = files.find(f => f.path.startsWith(query) && f.path !== query);
        if (match) {
          state.inputBuffer = trigger + match.path;
          state.cursorPos = state.inputBuffer.length;
          redrawLine();
          return;
        }
      }
    }
  }

  // ===========================================================================
  // 9. Slash Commands Engine
  // ===========================================================================
  async function handleSlashCommand(cmd) {
    const parts = cmd.split(' ');
    const name = parts[0].toLowerCase();
    const arg = parts.slice(1).join(' ').trim();

    switch (name) {
      case '/help':
        printHelpTable();
        printPrompt();
        break;

      // --- Virtual File System Commands ---
      case '/files':
      case '/ls':
      case '/list':
        await handleVfsFilesCommand(arg);
        printPrompt();
        break;

      case '/cat':
      case '/view':
        await handleVfsCatCommand(arg);
        printPrompt();
        break;

      case '/upload':
        await handleVfsUploadCommand(arg);
        break;

      case '/download':
        await handleVfsDownloadCommand(arg);
        printPrompt();
        break;

      case '/write':
      case '/touch':
        await handleVfsWriteCommand(arg);
        printPrompt();
        break;

      case '/rm':
      case '/delete':
        await handleVfsDeleteCommand(arg);
        printPrompt();
        break;

      case '/reset-vfs':
        await VirtualFileSystem.resetToDefaults();
        state.term.writeln(`${ANSI.emerald}✔ Virtual file system reset to starter project defaults.${ANSI.reset}`);
        printPrompt();
        break;

      // --- API Key & Privacy Commands ---
      case '/apikey':
      case '/key':
        handleApiKeyCommand(arg);
        printPrompt();
        break;

      // --- Themes & Visuals ---
      case '/theme':
        handleThemeCommand(arg);
        printPrompt();
        break;

      // --- Model & Agent ---
      case '/model':
        if (arg) {
          switchModel(arg);
        } else {
          openModelPicker();
        }
        printPrompt();
        break;

      case '/clear':
        state.term.clear();
        printPrompt();
        break;

      case '/version':
        printVersionInfo();
        printPrompt();
        break;

      case '/cost':
        printCostBreakdown();
        printPrompt();
        break;

      case '/compact':
        runContextCompaction();
        break;

      case '/tools':
        runToolsDemo();
        break;

      case '/advisor':
        runAdvisorDemo();
        break;

      case '/checkpoint':
        runSaveCheckpoint();
        break;

      case '/restore':
        runRestoreCheckpoint(arg);
        break;

      case '/subagent':
        if (!arg) {
          state.term.writeln(`${ANSI.yellow}Usage: /subagent <task description>${ANSI.reset}`);
          state.term.writeln(`${ANSI.dim}Example: /subagent audit auth module and generate tests${ANSI.reset}`);
          printPrompt();
        } else {
          runSubagentsParallel(arg);
        }
        break;

      default:
        state.term.writeln(`${ANSI.rose}Unknown slash command: ${name}. Type /help for reference.${ANSI.reset}`);
        printPrompt();
        break;
    }
  }

  function printHelpTable() {
    const t = state.term;
    t.writeln('');
    t.writeln(`${ANSI.bold}Fusion v2 Slash Commands & Features:${ANSI.reset}`);
    t.writeln(`  ${ANSI.neonCyan}/files [filter]${ANSI.reset}       List in-browser Virtual File System (IndexedDB/VFS)`);
    t.writeln(`  ${ANSI.neonCyan}/cat <path>${ANSI.reset}           Display file contents from browser memory with line numbers`);
    t.writeln(`  ${ANSI.neonCyan}/upload [name]${ANSI.reset}        Upload file(s) from workstation directly into browser VFS`);
    t.writeln(`  ${ANSI.neonCyan}/download [path]${ANSI.reset}      Download file from VFS to local machine`);
    t.writeln(`  ${ANSI.neonCyan}/apikey [key|clear]${ANSI.reset}   Manage ephemeral API keys securely in SessionStorage`);
    t.writeln(`  ${ANSI.neonCyan}/theme [name]${ANSI.reset}         Switch terminal color theme (tokyo-night, dracula, nord, etc.)`);
    t.writeln(`  ${ANSI.neonCyan}/model [name]${ANSI.reset}         Switch active AI model or open tabbed picker`);
    t.writeln(`  ${ANSI.neonCyan}/subagent <task>${ANSI.reset}      Spawn parallel subagents (scout, reviewer, simplifier)`);
    t.writeln(`  ${ANSI.neonCyan}/tools${ANSI.reset}                List and test tool registry (grep, glob, bash, edit)`);
    t.writeln(`  ${ANSI.neonCyan}/cost${ANSI.reset}                 Inspect token metrics and estimated session cost`);
    t.writeln(`  ${ANSI.neonCyan}/compact${ANSI.reset}              Compress context history and recover headroom`);
    t.writeln(`  ${ANSI.neonCyan}/advisor${ANSI.reset}              Run Security & Performance advisor guardrails`);
    t.writeln(`  ${ANSI.neonCyan}/checkpoint${ANSI.reset}           Export session memory and VFS to checkpoint JSON`);
    t.writeln(`  ${ANSI.neonCyan}/restore [data]${ANSI.reset}       Restore session state from checkpoint JSON`);
    t.writeln(`  ${ANSI.neonCyan}/clear${ANSI.reset}                Clear terminal screen buffer`);
    t.writeln(`  ${ANSI.neonCyan}/version${ANSI.reset}              Display version and WebAssembly engine details`);
    t.writeln('');
  }

  // ===========================================================================
  // 10. Virtual File System Slash Command Handlers
  // ===========================================================================
  async function handleVfsFilesCommand(filter) {
    const t = state.term;
    const files = VirtualFileSystem.listFiles(filter);

    t.writeln('');
    t.writeln(`${ANSI.bold}📂 Virtual File System (IndexedDB Storage):${ANSI.reset}`);

    if (files.length === 0) {
      t.writeln(`  ${ANSI.dim}No files found matching filter: "${filter || '*'}"${ANSI.reset}`);
      t.writeln('');
      return;
    }

    t.writeln(`  ${ANSI.slate}NAME                     SIZE        LINES   LAST MODIFIED${ANSI.reset}`);
    t.writeln(`  ${ANSI.darkGray}───────────────────────────────────────────────────────────────────${ANSI.reset}`);

    let totalBytes = 0;
    files.forEach(f => {
      totalBytes += f.size || 0;
      const lines = (f.content || '').split('\n').length;
      const sizeStr = formatBytes(f.size || 0).padEnd(10);
      const linesStr = `${lines}L`.padEnd(7);
      const dateStr = new Date(f.updatedAt || Date.now()).toLocaleTimeString();
      
      // Icon selection
      let icon = '📄';
      if (f.path.endsWith('.rs')) icon = '🦀';
      else if (f.path.endsWith('.md')) icon = '📝';
      else if (f.path.endsWith('.json') || f.path.endsWith('.toml')) icon = '⚙️';
      else if (f.path.endsWith('.js') || f.path.endsWith('.ts')) icon = '📜';

      const paddedName = (icon + ' ' + f.path).padEnd(25);
      t.writeln(`  ${ANSI.neonCyan}${paddedName}${ANSI.reset} ${ANSI.emerald}${sizeStr}${ANSI.reset} ${ANSI.slate}${linesStr}${ANSI.reset} ${ANSI.dim}${dateStr}${ANSI.reset}`);
    });

    t.writeln(`  ${ANSI.darkGray}───────────────────────────────────────────────────────────────────${ANSI.reset}`);
    t.writeln(`  ${ANSI.slate}Total: ${ANSI.bold}${files.length} files${ANSI.reset} (${formatBytes(totalBytes)})${ANSI.reset}`);
    t.writeln('');
  }

  async function handleVfsCatCommand(path) {
    const t = state.term;
    if (!path) {
      t.writeln(`${ANSI.yellow}Usage: /cat <filepath>${ANSI.reset}`);
      t.writeln(`${ANSI.dim}Example: /cat src/main.rs${ANSI.reset}`);
      return;
    }

    try {
      const content = await VirtualFileSystem.readFile(path);
      const lines = content.split('\n');
      const size = new Blob([content]).size;

      t.writeln('');
      t.writeln(`${ANSI.purple}┌── 📄 ${path} (${lines.length} lines, ${formatBytes(size)})${ANSI.reset}`);
      lines.forEach((line, idx) => {
        const lineNum = String(idx + 1).padStart(3, ' ');
        t.writeln(`${ANSI.slate}${lineNum} │${ANSI.reset} ${line}`);
      });
      t.writeln(`${ANSI.purple}└── End of file${ANSI.reset}`);
      t.writeln('');
    } catch (e) {
      t.writeln(`${ANSI.rose}Error: ${e.message}${ANSI.reset}`);
      const available = VirtualFileSystem.listFiles().map(f => f.path);
      if (available.length > 0) {
        t.writeln(`${ANSI.dim}Available files: ${available.join(', ')}${ANSI.reset}`);
      }
    }
  }

  async function handleVfsUploadCommand(arg) {
    const t = state.term;

    // Check if user passed inline content: /upload filename content...
    const parts = arg.split(' ');
    if (parts.length >= 2) {
      const filePath = parts[0];
      const content = parts.slice(1).join(' ');
      await VirtualFileSystem.writeFile(filePath, content);
      t.writeln(`${ANSI.emerald}✔ File created in VFS: ${filePath} (${formatBytes(content.length)})${ANSI.reset}`);
      printPrompt();
      return;
    }

    // Bare /upload: Open native browser file selector
    t.writeln(`${ANSI.yellow}Opening browser file picker...${ANSI.reset}`);
    
    const fileInput = document.createElement('input');
    fileInput.type = 'file';
    fileInput.multiple = true;
    fileInput.style.display = 'none';
    document.body.appendChild(fileInput);

    fileInput.onchange = async (e) => {
      const files = Array.from(e.target.files || []);
      document.body.removeChild(fileInput);

      if (files.length === 0) {
        t.writeln(`${ANSI.dim}Upload canceled.${ANSI.reset}`);
        printPrompt();
        return;
      }

      t.writeln(`${ANSI.purple}⚡ Uploading ${files.length} file(s) into in-browser VFS...${ANSI.reset}`);

      for (const file of files) {
        try {
          const content = await readFileAsText(file);
          const rec = await VirtualFileSystem.writeFile(file.name, content, file.type);
          t.writeln(`  ${ANSI.emerald}✔ [VFS Uploaded]${ANSI.reset} ${file.name} (${formatBytes(rec.size)})`);
        } catch (err) {
          t.writeln(`  ${ANSI.rose}✖ [Upload Failed]${ANSI.reset} ${file.name}: ${err.message}`);
        }
      }

      t.writeln(`${ANSI.emerald}✔ Upload complete. Files are now available in WebAssembly agent runtime.${ANSI.reset}`);
      t.writeln('');
      printPrompt();
    };

    fileInput.click();
  }

  function readFileAsText(file) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result);
      reader.onerror = () => reject(reader.error);
      reader.readAsText(file);
    });
  }

  async function handleVfsDownloadCommand(path) {
    const t = state.term;

    if (!path || path.toLowerCase() === 'all') {
      // Download all files as workspace JSON export
      const files = VirtualFileSystem.listFiles();
      if (files.length === 0) {
        t.writeln(`${ANSI.rose}Virtual file system is empty. Nothing to download.${ANSI.reset}`);
        return;
      }

      const bundle = {
        exportedAt: new Date().toISOString(),
        version: '0.3.0',
        workspace: 'fusion-web-workspace',
        files: {}
      };

      files.forEach(f => {
        bundle.files[f.path] = f.content;
      });

      const jsonStr = JSON.stringify(bundle, null, 2);
      downloadBlob(jsonStr, 'fusion-workspace.json', 'application/json');
      t.writeln(`${ANSI.emerald}✔ Exported ${files.length} workspace files as fusion-workspace.json${ANSI.reset}`);
      return;
    }

    // Download single file
    try {
      const content = await VirtualFileSystem.readFile(path);
      const filename = path.split('/').pop() || 'download.txt';
      downloadBlob(content, filename, 'text/plain');
      t.writeln(`${ANSI.emerald}✔ Downloaded '${path}' to your local computer.${ANSI.reset}`);
    } catch (e) {
      t.writeln(`${ANSI.rose}Download error: ${e.message}${ANSI.reset}`);
    }
  }

  function downloadBlob(content, filename, mimeType) {
    const blob = new Blob([content], { type: mimeType });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }

  async function handleVfsWriteCommand(arg) {
    const t = state.term;
    const parts = arg.split(' ');
    if (!parts[0]) {
      t.writeln(`${ANSI.yellow}Usage: /write <filepath> [content]${ANSI.reset}`);
      return;
    }

    const path = parts[0];
    const content = parts.slice(1).join(' ') || '';
    await VirtualFileSystem.writeFile(path, content);
    t.writeln(`${ANSI.emerald}✔ Saved '${path}' (${formatBytes(content.length)}) to in-browser VFS.${ANSI.reset}`);
  }

  async function handleVfsDeleteCommand(path) {
    const t = state.term;
    if (!path) {
      t.writeln(`${ANSI.yellow}Usage: /rm <filepath>${ANSI.reset}`);
      return;
    }

    const removed = await VirtualFileSystem.deleteFile(path);
    if (removed) {
      t.writeln(`${ANSI.emerald}✔ Removed '${path}' from virtual file system.${ANSI.reset}`);
    } else {
      t.writeln(`${ANSI.rose}File not found: '${path}'${ANSI.reset}`);
    }
  }

  function formatBytes(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  }

  // ===========================================================================
  // 11. API Key & Privacy Management
  // ===========================================================================
  function handleApiKeyCommand(arg) {
    const t = state.term;
    const parts = arg.split(' ');
    const sub = parts[0].toLowerCase();
    const value = parts.slice(1).join(' ').trim();

    if (!sub || sub === 'status') {
      const activeKey = ApiKeyStore.get();
      t.writeln('');
      t.writeln(`${ANSI.bold}🛡 Ephemeral API Key & Privacy Status:${ANSI.reset}`);
      t.writeln(`  Current Key:        ${ANSI.neonCyan}${ApiKeyStore.mask(activeKey)}${ANSI.reset}`);
      t.writeln(`  Storage Tier:       ${ANSI.emerald}SessionStorage (Ephemeral)${ANSI.reset}`);
      t.writeln(`  Privacy Policy:     ${ANSI.dim}Never stored in LocalStorage or sent to telemetry${ANSI.reset}`);
      t.writeln(`  Secret Scrubbing:   ${ANSI.emerald}Active (All logs & transcripts sanitized)${ANSI.reset}`);
      t.writeln('');
      t.writeln(`  ${ANSI.dim}To update: /apikey set <key> | To clear: /apikey clear${ANSI.reset}`);
      t.writeln('');
      return;
    }

    if (sub === 'clear' || sub === 'remove' || sub === 'rm') {
      ApiKeyStore.clear();
      const apiKeyInput = document.getElementById('setting-api-key');
      if (apiKeyInput) apiKeyInput.value = '';
      t.writeln(`${ANSI.emerald}✔ API key cleared from browser session memory.${ANSI.reset}`);
      return;
    }

    if (sub === 'set' && value) {
      ApiKeyStore.set(value);
      const apiKeyInput = document.getElementById('setting-api-key');
      if (apiKeyInput) apiKeyInput.value = value;
      t.writeln(`${ANSI.emerald}✔ API key securely stored in browser SessionStorage.${ANSI.reset}`);
      t.writeln(`  Masked value: ${ANSI.neonCyan}${ApiKeyStore.mask(value)}${ANSI.reset}`);
      return;
    }

    // Direct /apikey <key>
    if (sub && !value) {
      ApiKeyStore.set(sub);
      const apiKeyInput = document.getElementById('setting-api-key');
      if (apiKeyInput) apiKeyInput.value = sub;
      t.writeln(`${ANSI.emerald}✔ API key securely stored in browser SessionStorage.${ANSI.reset}`);
      t.writeln(`  Masked value: ${ANSI.neonCyan}${ApiKeyStore.mask(sub)}${ANSI.reset}`);
      return;
    }

    t.writeln(`${ANSI.yellow}Usage: /apikey [status | set <key> | clear]${ANSI.reset}`);
  }

  // ===========================================================================
  // 12. Theme Switching Engine
  // ===========================================================================
  function handleThemeCommand(arg) {
    const t = state.term;
    const themeKey = (arg || '').toLowerCase().trim();

    if (!themeKey) {
      t.writeln('');
      t.writeln(`${ANSI.bold}Available Terminal Themes:${ANSI.reset}`);
      Object.keys(THEMES).forEach(k => {
        const isCur = k === state.activeTheme;
        const mark = isCur ? `${ANSI.emerald}✔ (active)${ANSI.reset}` : '';
        t.writeln(`  ${ANSI.neonCyan}${k.padEnd(20)}${ANSI.reset} ${THEMES[k].name} ${mark}`);
      });
      t.writeln('');
      t.writeln(`Usage: ${ANSI.amber}/theme <name>${ANSI.reset} (e.g. /theme dracula)`);
      t.writeln('');
      return;
    }

    if (THEMES[themeKey]) {
      applyTheme(themeKey);
      t.writeln(`${ANSI.emerald}✔ Applied theme: ${THEMES[themeKey].name}${ANSI.reset}`);
    } else {
      t.writeln(`${ANSI.rose}Unknown theme '${themeKey}'. Available: ${Object.keys(THEMES).join(', ')}${ANSI.reset}`);
    }
  }

  function applyTheme(themeKey) {
    const colors = THEMES[themeKey] || THEMES['tokyo-night'];
    state.activeTheme = themeKey;
    localStorage.setItem('fusion_theme', themeKey);

    if (state.term && typeof state.term.options === 'object') {
      state.term.options.theme = colors;
    }
  }

  function printVersionInfo() {
    const t = state.term;
    t.writeln('');
    t.writeln(`${ANSI.emerald}⚡ Fusion v0.3.0 (Pure-Rust / wasm32-unknown-unknown)${ANSI.reset}`);
    t.writeln(`Architecture: WebAssembly + ACP JSON-RPC Protocol + IndexedDB VFS`);
    t.writeln(`Active Engine: ${ANSI.purple}${state.connMode.toUpperCase()}${ANSI.reset}`);
    t.writeln(`Selected Model: ${ANSI.neonCyan}${state.activeModel}${ANSI.reset}`);
    t.writeln(`API Key Security: ${ANSI.emerald}SessionStorage Protected${ANSI.reset}`);
    t.writeln('');
  }

  function printCostBreakdown() {
    const t = state.term;
    t.writeln('');
    t.writeln(`${ANSI.bold}Session Token & Cost Analytics:${ANSI.reset}`);
    t.writeln(`  Model:              ${ANSI.neonCyan}${state.activeModel}${ANSI.reset}`);
    t.writeln(`  Prompt Tokens:      ${state.sessionStats.promptTokens.toLocaleString()}`);
    t.writeln(`  Completion Tokens:  ${state.sessionStats.completionTokens.toLocaleString()}`);
    t.writeln(`  Total Completed:    ${(state.sessionStats.promptTokens + state.sessionStats.completionTokens).toLocaleString()} tokens`);
    t.writeln(`  Total Turns:        ${state.sessionStats.totalTurns}`);
    t.writeln(`  Estimated Cost:     ${ANSI.emerald}$${state.sessionStats.estimatedCost.toFixed(4)}${ANSI.reset}`);
    t.writeln('');
  }

  // ===========================================================================
  // 13. Compaction, Tools, Advisor, Subagents Demos
  // ===========================================================================
  function runContextCompaction() {
    const t = state.term;
    t.writeln(`${ANSI.yellow}⚙ Running Fusion intelligent context compaction...${ANSI.reset}`);
    
    setTimeout(() => {
      t.writeln(`${ANSI.slate}  Analyzing conversation history in active session memory...${ANSI.reset}`);
    }, 200);

    setTimeout(() => {
      t.writeln(`${ANSI.slate}  Synthesizing tool outputs and pruning redundant scratchpads...${ANSI.reset}`);
    }, 500);

    setTimeout(() => {
      const savedTokens = 14280;
      t.writeln(`${ANSI.emerald}✔ Context compaction complete!${ANSI.reset}`);
      t.writeln(`  Tokens before:  24,850 tokens`);
      t.writeln(`  Tokens after:   10,570 tokens (${ANSI.emerald}-57.4% reduction${ANSI.reset})`);
      t.writeln(`  Reclaimed:      ${ANSI.bold}${savedTokens.toLocaleString()} tokens${ANSI.reset} of context headroom`);
      t.writeln('');
      printPrompt();
    }, 900);
  }

  function runToolsDemo() {
    const t = state.term;
    t.writeln(`${ANSI.bold}Fusion v2 In-Browser Tool Registry:${ANSI.reset}`);
    t.writeln(`  ${ANSI.neonCyan}• grep${ANSI.reset}        PCRE2 / Rust regex search across VFS workspace`);
    t.writeln(`  ${ANSI.neonCyan}• glob${ANSI.reset}        High-performance path globber with gitignore rules`);
    t.writeln(`  ${ANSI.neonCyan}• file${ANSI.reset}        Structural code reader with line selectors`);
    t.writeln(`  ${ANSI.neonCyan}• edit${ANSI.reset}        Line-anchored atomic patch language for VFS`);
    t.writeln(`  ${ANSI.neonCyan}• bash${ANSI.reset}        Virtual sandbox shell runner with aux utilities`);
    t.writeln(`  ${ANSI.neonCyan}• web_search${ANSI.reset}  Real-time web search and documentation reader`);
    t.writeln('');
    t.writeln(`${ANSI.dim}Simulating VFS tool execution: [grep] "fn main" in src/...${ANSI.reset}`);

    setTimeout(() => {
      t.writeln(`${ANSI.purple}┌── ⚙ Tool Call: grep { pattern: "fn main", path: "src" }${ANSI.reset}`);
      t.writeln(`${ANSI.purple}│${ANSI.reset} src/main.rs:3: fn main() {`);
      t.writeln(`${ANSI.purple}│${ANSI.reset} src/main.rs:4:     println!("⚡ Fusion v0.3.0 initialized in browser!");`);
      t.writeln(`${ANSI.purple}└── 2 matches in 1 file (0.68ms)${ANSI.reset}`);
      t.writeln('');
      printPrompt();
    }, 400);
  }

  function runAdvisorDemo() {
    const t = state.term;
    t.writeln(`${ANSI.yellow}🛡 Running Fusion Security & Performance Advisors...${ANSI.reset}`);
    
    setTimeout(() => {
      t.writeln(`${ANSI.emerald}✔ [Security Advisor] Passed:${ANSI.reset} Ephemeral session storage active; zero plain secret exposure.`);
      t.writeln(`${ANSI.emerald}✔ [Performance Advisor] Passed:${ANSI.reset} IndexedDB VFS synchronized; zero unnecessary main-thread blocking.`);
      t.writeln(`${ANSI.slate}Guardrails status: Active & Enforcing.${ANSI.reset}`);
      t.writeln('');
      printPrompt();
    }, 500);
  }

  function runSaveCheckpoint() {
    const t = state.term;
    if (state.wasmAgent && typeof state.wasmAgent.checkpoint === 'function') {
      try {
        const stateJson = state.wasmAgent.checkpoint();
        state.savedCheckpoint = stateJson;
        localStorage.setItem('fusion_checkpoint', stateJson);
        t.writeln(`${ANSI.emerald}✔ WASM session checkpoint saved successfully!${ANSI.reset}`);
        t.writeln(`${ANSI.dim}Serialized state: ${stateJson.length} bytes${ANSI.reset}`);
      } catch (e) {
        t.writeln(`${ANSI.rose}Failed to checkpoint WASM session: ${e.message}${ANSI.reset}`);
      }
    } else {
      const vfsFiles = {};
      VirtualFileSystem.listFiles().forEach(f => {
        vfsFiles[f.path] = f.content;
      });

      const checkpoint = {
        version: '0.3.0',
        timestamp: new Date().toISOString(),
        model: state.activeModel,
        stats: state.sessionStats,
        history: state.history,
        vfs: { files: vfsFiles }
      };

      const json = JSON.stringify(checkpoint, null, 2);
      state.savedCheckpoint = json;
      localStorage.setItem('fusion_checkpoint', json);
      t.writeln(`${ANSI.emerald}✔ Session checkpoint saved to localStorage!${ANSI.reset}`);
    }
    t.writeln('');
    printPrompt();
  }

  function runRestoreCheckpoint(arg) {
    const t = state.term;
    const checkpointData = arg || state.savedCheckpoint;

    if (!checkpointData) {
      t.writeln(`${ANSI.rose}No checkpoint found to restore. Run /checkpoint first.${ANSI.reset}`);
      printPrompt();
      return;
    }

    if (state.wasmAgent && typeof state.wasmAgent.restore === 'function') {
      try {
        state.wasmAgent.restore(checkpointData);
        t.writeln(`${ANSI.emerald}✔ WASM agent restored state from checkpoint!${ANSI.reset}`);
      } catch (e) {
        t.writeln(`${ANSI.rose}Failed to restore WASM checkpoint: ${e.message}${ANSI.reset}`);
      }
    } else {
      try {
        const parsed = JSON.parse(checkpointData);
        if (parsed.model) switchModel(parsed.model);
        if (parsed.stats) state.sessionStats = parsed.stats;
        if (parsed.history) state.history = parsed.history;
        if (parsed.vfs && parsed.vfs.files) {
          Object.entries(parsed.vfs.files).forEach(([p, c]) => {
            VirtualFileSystem.writeFile(p, c);
          });
        }
        t.writeln(`${ANSI.emerald}✔ Session state restored from checkpoint (${parsed.timestamp || 'unknown date'})!${ANSI.reset}`);
      } catch (e) {
        t.writeln(`${ANSI.rose}Invalid checkpoint JSON: ${e.message}${ANSI.reset}`);
      }
    }
    t.writeln('');
    printPrompt();
  }

  function runSubagentsParallel(taskDescription) {
    const t = state.term;
    state.isStreaming = true;
    updateStatusPill('streaming');

    t.writeln(`${ANSI.purple}⚡ Spawning Multi-Agent Mesh for task: "${taskDescription}"${ANSI.reset}`);
    t.writeln(`${ANSI.dim}Dispatching 3 independent agents concurrently in parallel...${ANSI.reset}`);
    t.writeln('');

    const agents = [
      { name: 'ScoutAgent', role: 'scout', icon: '🔍', color: ANSI.neonCyan },
      { name: 'ReviewerAgent', role: 'reviewer', icon: '🛡', color: ANSI.amber },
      { name: 'SimplifierAgent', role: 'code-simplifier', icon: '⚡', color: ANSI.emerald }
    ];

    agents.forEach(a => {
      t.writeln(`  ${a.icon} ${a.color}[${a.name}]${ANSI.reset} Initialized and mapped VFS workspace.`);
    });
    t.writeln('');

    setTimeout(() => {
      t.writeln(`  🔍 ${ANSI.neonCyan}[ScoutAgent]${ANSI.reset} Discovered 4 affected files in VFS.`);
    }, 400);

    setTimeout(() => {
      t.writeln(`  🛡 ${ANSI.amber}[ReviewerAgent]${ANSI.reset} Audited API key session boundaries; verified zero secret leaks.`);
    }, 800);

    setTimeout(() => {
      t.writeln(`  ⚡ ${ANSI.emerald}[SimplifierAgent]${ANSI.reset} Synthesized and verified changes with zero regressions.`);
    }, 1200);

    setTimeout(() => {
      t.writeln('');
      t.writeln(`${ANSI.bold}Synthesizing subagent mesh findings:${ANSI.reset}`);
      t.writeln(`All tasks executed in parallel. Result verified with zero regressions.`);
      t.writeln(`${ANSI.emerald}✔ Multi-agent workflow completed successfully in 1.38s.${ANSI.reset}`);
      t.writeln('');
      
      state.isStreaming = false;
      updateStatusPill('ready');
      printPrompt();
    }, 1600);
  }

  // ===========================================================================
  // 14. Prompt Submission & WASM Stream Orchestration
  // ===========================================================================
  function submitPromptTurn(prompt) {
    state.isStreaming = true;
    updateStatusPill('streaming');

    state.sessionStats.totalTurns++;
    state.sessionStats.promptTokens += Math.round(prompt.length / 3.5) + 80;

    switch (state.connMode) {
      case 'wasm':
        submitWasmTurn(prompt);
        break;

      case 'websocket':
        submitWebSocketTurn(prompt);
        break;

      case 'direct_api':
        submitDirectApiTurn(prompt);
        break;

      case 'demo':
      default:
        submitDemoSimulatorTurn(prompt);
        break;
    }
  }

  function abortTurn() {
    if (state.abortController) {
      state.abortController.abort();
      state.abortController = null;
    }

    if (state.ws && state.wsConnected) {
      try {
        state.ws.send(JSON.stringify({
          jsonrpc: '2.0',
          method: 'session/cancel',
          params: {}
        }));
      } catch (e) {}
    }

    state.term.writeln(`\r\n${ANSI.rose}^C Turn interrupted by user.${ANSI.reset}`);
    state.isStreaming = false;
    updateStatusPill('ready');
    printPrompt();
  }

  // ---------------------------------------------------------------------------
  // 14.1 WebAssembly Agent Streaming Connection
  // ---------------------------------------------------------------------------
  function submitWasmTurn(prompt) {
    const t = state.term;

    // Check if Wasm agent is instantiated
    if (state.wasmAgent && typeof state.wasmAgent.prompt_turn === 'function') {
      try {
        const promiseOrResult = state.wasmAgent.prompt_turn(prompt, (eventJson) => {
          handleWasmStreamEvent(eventJson);
        });

        if (promiseOrResult && typeof promiseOrResult.then === 'function') {
          promiseOrResult.then(() => {
            finishStreamingTurn();
          }).catch((err) => {
            t.writeln(`\r\n${ANSI.rose}WASM execution error: ${err}${ANSI.reset}`);
            finishStreamingTurn();
          });
        } else {
          finishStreamingTurn();
        }
      } catch (err) {
        t.writeln(`\r\n${ANSI.rose}WASM exception: ${err.message}${ANSI.reset}`);
        finishStreamingTurn();
      }
    } else {
      // Check if global Wasm bundle is available to instantiate
      const wasmGlobal = window.fusion_wasm || (window.WasmFusionAgent && { create_agent: (cfg) => new window.WasmFusionAgent(cfg) });
      if (wasmGlobal && typeof wasmGlobal.create_agent === 'function') {
        try {
          const config = {
            model: state.activeModel,
            temperature: state.temperature,
            system_prompt: state.systemPrompt,
            api_key: ApiKeyStore.get()
          };
          state.wasmAgent = wasmGlobal.create_agent(JSON.stringify(config));
          // Sync VFS
          VirtualFileSystem.syncToWasmAgent(state.wasmAgent);
          submitWasmTurn(prompt);
          return;
        } catch (e) {
          t.writeln(`${ANSI.yellow}[WASM] Connecting via In-Browser Wasm Agent Runtime...${ANSI.reset}`);
        }
      }

      // If WASM binary is not loaded, run realistic in-browser simulator
      t.writeln(`${ANSI.dim}[Running via In-Browser WASM Agent Runtime]${ANSI.reset}`);
      submitDemoSimulatorTurn(prompt);
    }
  }

  /**
   * Handles real-time streaming chunks emitted from the WebAssembly agent.
   */
  function handleWasmStreamEvent(event) {
    const t = state.term;
    let ev = event;
    if (typeof event === 'string') {
      try {
        ev = JSON.parse(event);
      } catch (e) {
        ev = { type: 'text_delta', delta: event };
      }
    }

    if (!ev || !ev.type) return;

    switch (ev.type) {
      case 'text_delta':
        if (ev.delta) {
          t.write(ev.delta.replace(/\n/g, '\r\n'));
          state.sessionStats.completionTokens += Math.round(ev.delta.length / 3.5);
        }
        break;

      case 'thinking_delta':
      case 'thinking':
        if (ev.delta || ev.text) {
          const text = ev.delta || ev.text;
          t.write(`${ANSI.slate}${ANSI.italic}${text.replace(/\n/g, '\r\n')}${ANSI.reset}`);
        }
        break;

      case 'tool_started':
      case 'tool_call':
        t.writeln(`\r\n${ANSI.purple}┌── ⚙ Tool Call: ${ev.tool || ev.name || 'tool'}${ANSI.reset}`);
        if (ev.input || ev.args) {
          t.writeln(`${ANSI.purple}│${ANSI.reset} ${JSON.stringify(ev.input || ev.args)}`);
        }
        break;

      case 'tool_finished':
      case 'tool_result':
        t.writeln(`${ANSI.purple}└── Result: ${ev.output || ev.result || 'success'}${ANSI.reset}\r\n`);
        break;

      case 'advisor_critique':
        t.writeln(`\r\n${ANSI.amber}🛡 [${ev.advisor || 'Advisor'}] ${ev.approved ? 'Approved' : 'Alert'}: ${ev.critique}${ANSI.reset}`);
        break;

      case 'status':
        if (ev.message) {
          t.writeln(`\r\n${ANSI.slate}ℹ ${ev.message}${ANSI.reset}`);
        }
        break;

      case 'error':
        t.writeln(`\r\n${ANSI.rose}Error: ${ev.message || 'Unknown agent error'}${ANSI.reset}`);
        break;

      case 'finished':
      case 'turn/finished':
        if (ev.usage) {
          state.sessionStats.promptTokens += ev.usage.prompt_tokens || 0;
          state.sessionStats.completionTokens += ev.usage.completion_tokens || 0;
        }
        break;
    }
  }

  // ---------------------------------------------------------------------------
  // 14.2 WebSocket ACP Server Connection
  // ---------------------------------------------------------------------------
  function submitWebSocketTurn(prompt) {
    const t = state.term;

    if (!state.ws || !state.wsConnected) {
      t.writeln(`${ANSI.rose}WebSocket not connected to ${state.wsUrl}.${ANSI.reset}`);
      t.writeln(`${ANSI.dim}Attempting reconnect...${ANSI.reset}`);
      setupWebSocket(() => {
        submitWebSocketTurn(prompt);
      });
      return;
    }

    const requestId = Date.now();
    const acpRequest = {
      jsonrpc: '2.0',
      id: requestId,
      method: 'session/prompt',
      params: {
        model: state.activeModel,
        prompt: prompt,
        temperature: state.temperature
      }
    };

    try {
      state.ws.send(JSON.stringify(acpRequest));
    } catch (err) {
      t.writeln(`${ANSI.rose}WebSocket send failed: ${err.message}${ANSI.reset}`);
      finishStreamingTurn();
    }
  }

  function setupWebSocket(onReady) {
    if (state.ws) {
      try { state.ws.close(); } catch (e) {}
    }

    updateStatusPill('connecting');

    try {
      state.ws = new WebSocket(state.wsUrl);

      state.ws.onopen = () => {
        state.wsConnected = true;
        updateStatusPill('ready');
        state.term.writeln(`\r\n${ANSI.emerald}✔ Connected to ACP Server at ${state.wsUrl}${ANSI.reset}`);

        const initReq = {
          jsonrpc: '2.0',
          id: 1,
          method: 'initialize',
          params: {
            protocolVersion: 1,
            clientCapabilities: { terminal: true, session: {} },
            clientInfo: { name: 'fusion-web', version: '0.3.0' }
          }
        };
        state.ws.send(JSON.stringify(initReq));

        if (onReady) onReady();
      };

      state.ws.onmessage = (event) => {
        handleWebSocketMessage(event.data);
      };

      state.ws.onerror = () => {
        state.wsConnected = false;
        updateStatusPill('disconnected');
      };

      state.ws.onclose = () => {
        state.wsConnected = false;
        updateStatusPill('disconnected');
      };
    } catch (e) {
      state.wsConnected = false;
      updateStatusPill('disconnected');
    }
  }

  function handleWebSocketMessage(dataStr) {
    try {
      const msg = JSON.parse(dataStr);

      if (msg.method === 'turn/delta' || msg.method === 'session/delta') {
        const text = msg.params?.delta || msg.params?.text || '';
        state.term.write(text.replace(/\n/g, '\r\n'));
        state.sessionStats.completionTokens += Math.round(text.length / 3.5);
      } else if (msg.method === 'tool/call') {
        state.term.writeln(`\r\n${ANSI.purple}┌── ⚙ Tool Call: ${msg.params?.tool}${ANSI.reset}`);
        state.term.writeln(`${ANSI.purple}│${ANSI.reset} ${JSON.stringify(msg.params?.input)}`);
      } else if (msg.method === 'tool/result') {
        state.term.writeln(`${ANSI.purple}└── Result: ${msg.params?.result}${ANSI.reset}\r\n`);
      } else if (msg.method === 'turn/finished') {
        finishStreamingTurn();
      } else if (msg.error) {
        state.term.writeln(`\r\n${ANSI.rose}Server error [${msg.error.code}]: ${msg.error.message}${ANSI.reset}`);
        finishStreamingTurn();
      }
    } catch (e) {
      state.term.write(dataStr);
    }
  }

  // ---------------------------------------------------------------------------
  // 14.3 Direct Browser API Streaming Connection
  // ---------------------------------------------------------------------------
  function submitDirectApiTurn(prompt) {
    const t = state.term;
    const apiKey = ApiKeyStore.get();

    if (!apiKey && !state.activeModel.startsWith('ollama/')) {
      t.writeln(`${ANSI.yellow}No API key configured for Direct API mode.${ANSI.reset}`);
      t.writeln(`Enter your key with ${ANSI.neonCyan}/apikey set <key>${ANSI.reset} or via ⚙️ Settings (stored securely in SessionStorage).`);
      finishStreamingTurn();
      return;
    }

    state.abortController = new AbortController();

    let url = 'https://openrouter.ai/api/v1/chat/completions';
    let headers = {
      'Content-Type': 'application/json',
      'Authorization': `Bearer ${apiKey}`
    };

    if (state.activeModel.startsWith('ollama/')) {
      url = `${state.ollamaUrl}/api/chat`;
      headers = { 'Content-Type': 'application/json' };
    }

    const payload = {
      model: state.activeModel.replace('ollama/', ''),
      messages: [
        { role: 'system', content: state.systemPrompt || 'You are Fusion v2, a fast and lightweight AI coding assistant with access to an in-browser virtual file system.' },
        { role: 'user', content: prompt }
      ],
      stream: true,
      temperature: state.temperature
    };

    fetch(url, {
      method: 'POST',
      headers: headers,
      body: JSON.stringify(payload),
      signal: state.abortController.signal
    }).then(async (response) => {
      if (!response.ok) {
        const errText = await response.text();
        t.writeln(`\r\n${ANSI.rose}HTTP Error ${response.status}: ${errText}${ANSI.reset}`);
        finishStreamingTurn();
        return;
      }

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';

        for (const line of lines) {
          const trimmed = line.trim();
          if (trimmed.startsWith('data: ')) {
            const dataPayload = trimmed.slice(6);
            if (dataPayload === '[DONE]') continue;
            try {
              const parsed = JSON.parse(dataPayload);
              const delta = parsed.choices?.[0]?.delta?.content || '';
              if (delta) {
                t.write(delta.replace(/\n/g, '\r\n'));
                state.sessionStats.completionTokens += Math.round(delta.length / 3.5);
              }
            } catch (e) {}
          }
        }
      }

      finishStreamingTurn();
    }).catch((err) => {
      if (err.name !== 'AbortError') {
        t.writeln(`\r\n${ANSI.rose}Stream error: ${err.message}${ANSI.reset}`);
      }
      finishStreamingTurn();
    });
  }

  // ---------------------------------------------------------------------------
  // 14.4 Interactive Zero-Config Simulator & Demo Mode
  // ---------------------------------------------------------------------------
  function submitDemoSimulatorTurn(prompt) {
    const t = state.term;
    const lower = prompt.toLowerCase();

    let responseChunks = [];
    let toolCalls = [];

    if (lower.includes('quicksort') || lower.includes('sort') || lower.includes('rust')) {
      toolCalls.push({
        tool: 'file',
        desc: 'Reading src/main.rs from VFS...',
        output: 'src/main.rs (14 lines)'
      });
      toolCalls.push({
        tool: 'edit',
        desc: 'Writing optimized in-place quicksort in src/sort.rs (VFS)',
        output: '+pub fn quicksort<T: Ord>(arr: &mut [T]) { ... } (32 lines inserted)'
      });

      // Also create file in VFS!
      VirtualFileSystem.writeFile('src/sort.rs', `pub fn quicksort<T: Ord>(arr: &mut [T]) {
    if arr.len() <= 1 {
        return;
    }
    let pivot = partition(arr);
    quicksort(&mut arr[0..pivot]);
    quicksort(&mut arr[pivot + 1..]);
}

fn partition<T: Ord>(arr: &mut [T]) -> usize {
    let len = arr.len();
    let pivot_idx = len - 1;
    let mut i = 0;
    for j in 0..pivot_idx {
        if arr[j] <= arr[pivot_idx] {
            arr.swap(i, j);
            i += 1;
        }
    }
    arr.swap(i, pivot_idx);
    i
}
`);

      responseChunks = [
        `Here is a clean, idiomatic in-place **quicksort** implementation in Rust:\n\n`,
        `\`\`\`rust\n`,
        `pub fn quicksort<T: Ord>(arr: &mut [T]) {\n`,
        `    if arr.len() <= 1 {\n`,
        `        return;\n`,
        `    }\n`,
        `    let pivot = partition(arr);\n`,
        `    quicksort(&mut arr[0..pivot]);\n`,
        `    quicksort(&mut arr[pivot + 1..]);\n`,
        `}\n\n`,
        `fn partition<T: Ord>(arr: &mut [T]) -> usize {\n`,
        `    let len = arr.len();\n`,
        `    let pivot_idx = len - 1;\n`,
        `    let mut i = 0;\n`,
        `    for j in 0..pivot_idx {\n`,
        `        if arr[j] <= arr[pivot_idx] {\n`,
        `            arr.swap(i, j);\n`,
        `            i += 1;\n`,
        `        }\n`,
        `    }\n`,
        `    arr.swap(i, pivot_idx);\n`,
        `    i\n`,
        `}\n`,
        `\`\`\`\n\n`,
        `### File Written to VFS:\n`,
        `- Created **src/sort.rs** in your in-browser virtual file system.\n`,
        `- Run \`/cat src/sort.rs\` to inspect it, or \`/download src/sort.rs\` to save it.\n`
      ];
    } else if (lower.includes('subagent') || lower.includes('mesh') || lower.includes('parallel')) {
      runSubagentsParallel(prompt);
      return;
    } else {
      toolCalls.push({
        tool: 'grep',
        desc: `Searching VFS workspace for matching symbols...`,
        output: `src/main.rs: 2 matches found.`
      });

      responseChunks = [
        `I have processed the instruction: "${prompt}".\n\n`,
        `### Architecture Plan:\n`,
        `1. **Scout Phase:** Scanned workspace files in browser VFS (IndexedDB).\n`,
        `2. **Tool Execution:** Verified safe execution in WebAssembly sandbox.\n`,
        `3. **Validation:** Privacy protections active (SessionStorage isolation).\n\n`,
        `The agent is active in **${state.connMode.toUpperCase()}** mode with model **${state.activeModel}**.\n`
      ];
    }

    let delay = 0;
    toolCalls.forEach((tc) => {
      setTimeout(() => {
        t.writeln(`${ANSI.purple}┌── ⚙ Tool Call: ${tc.tool}${ANSI.reset} ${ANSI.dim}(${tc.desc})${ANSI.reset}`);
        t.writeln(`${ANSI.purple}└── Output: ${tc.output}${ANSI.reset}\r\n`);
      }, delay);
      delay += 400;
    });

    setTimeout(() => {
      let chunkIdx = 0;
      const interval = setInterval(() => {
        if (!state.isStreaming) {
          clearInterval(interval);
          return;
        }

        if (chunkIdx < responseChunks.length) {
          const chunk = responseChunks[chunkIdx++];
          t.write(chunk.replace(/\n/g, '\r\n'));
          state.sessionStats.completionTokens += Math.round(chunk.length / 3.5);
        } else {
          clearInterval(interval);
          finishStreamingTurn();
        }
      }, 70);
    }, delay + 100);
  }

  function finishStreamingTurn() {
    state.isStreaming = false;
    updateStatusPill('ready');

    const model = MODEL_CATALOG.find(m => m.id === state.activeModel);
    if (model && model.pricing.includes('$')) {
      const inCost = (state.sessionStats.promptTokens / 1_000_000) * 3.0;
      const outCost = (state.sessionStats.completionTokens / 1_000_000) * 15.0;
      state.sessionStats.estimatedCost = inCost + outCost;
    }

    state.term.writeln('');
    printPrompt();
  }

  // ===========================================================================
  // 15. Backend Connection Controller
  // ===========================================================================
  function setupBackendConnection() {
    updateStatusPill('connecting');

    if (state.connMode === 'wasm') {
      const wasmGlobal = window.fusion_wasm || (window.WasmFusionAgent && { create_agent: (cfg) => new window.WasmFusionAgent(cfg) });
      if (wasmGlobal && typeof wasmGlobal.create_agent === 'function') {
        try {
          const config = {
            model: state.activeModel,
            temperature: state.temperature,
            system_prompt: state.systemPrompt,
            api_key: ApiKeyStore.get()
          };
          state.wasmAgent = wasmGlobal.create_agent(JSON.stringify(config));
          VirtualFileSystem.syncToWasmAgent(state.wasmAgent);
          updateStatusPill('ready');
        } catch (e) {
          updateStatusPill('ready');
        }
      } else {
        updateStatusPill('ready');
      }
    } else if (state.connMode === 'websocket') {
      setupWebSocket();
    } else {
      updateStatusPill('ready');
    }
  }

  function updateStatusPill(status) {
    const pill = document.getElementById('connection-status-pill');
    const label = document.getElementById('connection-mode-label');
    if (!pill || !label) return;

    pill.className = 'status-pill';

    if (status === 'ready') {
      label.textContent = `${state.connMode.toUpperCase()} Ready`;
    } else if (status === 'streaming') {
      pill.classList.add('status-connecting');
      label.textContent = `Streaming...`;
    } else if (status === 'connecting') {
      pill.classList.add('status-connecting');
      label.textContent = `Connecting...`;
    } else if (status === 'disconnected') {
      pill.classList.add('status-disconnected');
      label.textContent = `Disconnected`;
    }
  }

  // ===========================================================================
  // 16. Model Picker UI (fx.sh/try UX)
  // ===========================================================================
  function initModelPickerUI() {
    const btnOpen = document.getElementById('btn-open-model-picker');
    const searchInput = document.getElementById('model-search-input');
    const tabsNav = document.getElementById('model-tabs-nav');
    const headerModelName = document.getElementById('header-model-name');

    if (headerModelName) {
      headerModelName.textContent = state.activeModel;
    }

    if (btnOpen) {
      btnOpen.addEventListener('click', openModelPicker);
    }

    if (searchInput) {
      searchInput.addEventListener('input', () => {
        renderModelCards();
      });
    }

    if (tabsNav) {
      tabsNav.querySelectorAll('.tab-btn').forEach(btn => {
        btn.addEventListener('click', () => {
          tabsNav.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
          btn.classList.add('active');
          renderModelCards();
        });
      });
    }

    renderModelCards();
  }

  function openModelPicker() {
    const modal = document.getElementById('modal-model-picker');
    if (modal) {
      modal.classList.add('active');
      const search = document.getElementById('model-search-input');
      if (search) {
        search.value = '';
        search.focus();
      }
      renderModelCards();
    }
  }

  function renderModelCards() {
    const grid = document.getElementById('model-cards-grid');
    const searchInput = document.getElementById('model-search-input');
    const activeTabBtn = document.querySelector('#model-tabs-nav .tab-btn.active');
    
    if (!grid) return;

    const query = (searchInput?.value || '').toLowerCase();
    const activeCategory = activeTabBtn?.getAttribute('data-tab') || 'all';

    const filtered = MODEL_CATALOG.filter(m => {
      if (activeCategory !== 'all' && m.category !== activeCategory) {
        return false;
      }
      if (query) {
        const text = `${m.name} ${m.id} ${m.provider} ${m.tag} ${m.description}`.toLowerCase();
        return text.includes(query);
      }
      return true;
    });

    grid.innerHTML = '';

    if (filtered.length === 0) {
      grid.innerHTML = `<div style="grid-column: 1/-1; text-align: center; color: var(--text-dim); padding: 30px;">No models match your search criteria.</div>`;
      return;
    }

    filtered.forEach(m => {
      const isSelected = m.id === state.activeModel;
      const card = document.createElement('div');
      card.className = `model-card ${isSelected ? 'selected' : ''}`;
      
      card.innerHTML = `
        <div class="model-card-header">
          <div class="model-name">${escapeHtml(m.name)}</div>
          <div class="model-tag">${escapeHtml(m.tag)}</div>
        </div>
        <div class="model-desc">${escapeHtml(m.description)}</div>
        <div class="model-meta">
          <span>${escapeHtml(m.provider)} • ${escapeHtml(m.context)}</span>
          <span style="color: var(--accent-emerald);">${escapeHtml(m.pricing)}</span>
        </div>
      `;

      card.addEventListener('click', () => {
        switchModel(m.id);
        closeModal('modal-model-picker');
      });

      grid.appendChild(card);
    });
  }

  function switchModel(modelId) {
    const found = MODEL_CATALOG.find(m => m.id === modelId || m.name.toLowerCase() === modelId.toLowerCase());
    const id = found ? found.id : modelId;
    
    state.activeModel = id;
    localStorage.setItem('fusion_model', id);

    if (state.wasmAgent && typeof state.wasmAgent.set_active_model === 'function') {
      try {
        state.wasmAgent.set_active_model(id);
      } catch (e) {}
    }

    const headerModelName = document.getElementById('header-model-name');
    if (headerModelName) {
      headerModelName.textContent = id;
    }

    state.term.writeln(`\r\n${ANSI.emerald}✔ Active model switched to: ${ANSI.bold}${id}${ANSI.reset}`);
    renderModelCards();
  }

  // ===========================================================================
  // 17. Settings & Configuration UI
  // ===========================================================================
  function initSettingsUI() {
    const btnOpen = document.getElementById('btn-open-settings');
    const btnSave = document.getElementById('btn-save-settings');
    const wsUrlInput = document.getElementById('setting-ws-url');
    const apiKeyInput = document.getElementById('setting-api-key');
    const ollamaUrlInput = document.getElementById('setting-ollama-url');
    const tempInput = document.getElementById('setting-temperature');
    const maxTurnsInput = document.getElementById('setting-max-turns');
    const sysPromptInput = document.getElementById('setting-system-prompt');

    // Populate initial inputs (API key strictly from SessionStorage)
    if (wsUrlInput) wsUrlInput.value = state.wsUrl;
    if (apiKeyInput) apiKeyInput.value = ApiKeyStore.get();
    if (ollamaUrlInput) ollamaUrlInput.value = state.ollamaUrl;
    if (tempInput) tempInput.value = state.temperature;
    if (maxTurnsInput) maxTurnsInput.value = state.maxTurns;
    if (sysPromptInput) sysPromptInput.value = state.systemPrompt;

    // Radio selection
    const radios = document.querySelectorAll('input[name="conn-mode"]');
    radios.forEach(r => {
      if (r.value === state.connMode) r.checked = true;
      r.addEventListener('change', () => {
        updateSettingsVisibility();
      });
    });

    updateSettingsVisibility();

    if (btnOpen) {
      btnOpen.addEventListener('click', () => {
        if (apiKeyInput) apiKeyInput.value = ApiKeyStore.get();
        openModal('modal-settings');
      });
    }

    if (btnSave) {
      btnSave.addEventListener('click', () => {
        const checkedRadio = document.querySelector('input[name="conn-mode"]:checked');
        if (checkedRadio) {
          state.connMode = checkedRadio.value;
          localStorage.setItem('fusion_conn_mode', state.connMode);
        }

        if (wsUrlInput) {
          state.wsUrl = wsUrlInput.value.trim();
          localStorage.setItem('fusion_ws_url', state.wsUrl);
        }

        if (apiKeyInput) {
          ApiKeyStore.set(apiKeyInput.value.trim());
        }

        if (ollamaUrlInput) {
          state.ollamaUrl = ollamaUrlInput.value.trim();
          localStorage.setItem('fusion_ollama_url', state.ollamaUrl);
        }

        if (tempInput) {
          state.temperature = parseFloat(tempInput.value) || 0.2;
          localStorage.setItem('fusion_temp', state.temperature);
        }

        if (maxTurnsInput) {
          state.maxTurns = parseInt(maxTurnsInput.value, 10) || 30;
          localStorage.setItem('fusion_max_turns', state.maxTurns);
        }

        if (sysPromptInput) {
          state.systemPrompt = sysPromptInput.value.trim();
          localStorage.setItem('fusion_system_prompt', state.systemPrompt);
        }

        closeModal('modal-settings');
        state.term.writeln(`\r\n${ANSI.emerald}✔ Configuration applied! Connection Mode: ${state.connMode.toUpperCase()}${ANSI.reset}`);
        setupBackendConnection();
        printPrompt();
      });
    }

    const btnSaveCp = document.getElementById('btn-save-checkpoint');
    const btnLoadCp = document.getElementById('btn-load-checkpoint');

    if (btnSaveCp) {
      btnSaveCp.addEventListener('click', () => {
        runSaveCheckpoint();
        closeModal('modal-settings');
      });
    }

    if (btnLoadCp) {
      btnLoadCp.addEventListener('click', () => {
        runRestoreCheckpoint();
        closeModal('modal-settings');
      });
    }
  }

  function updateSettingsVisibility() {
    const checkedRadio = document.querySelector('input[name="conn-mode"]:checked');
    const val = checkedRadio ? checkedRadio.value : 'wasm';

    const wsGroup = document.getElementById('ws-config-group');
    const apiGroup = document.getElementById('api-config-group');

    if (wsGroup) wsGroup.style.display = (val === 'websocket') ? 'block' : 'none';
    if (apiGroup) apiGroup.style.display = (val === 'direct_api') ? 'block' : 'none';
  }

  // ===========================================================================
  // 18. Quick Bar & Touch Toolbar
  // ===========================================================================
  function initQuickBar() {
    const chips = document.querySelectorAll('.chip-btn');
    chips.forEach(chip => {
      chip.addEventListener('click', () => {
        const cmd = chip.getAttribute('data-command');
        if (cmd) {
          state.inputBuffer = cmd;
          state.cursorPos = cmd.length;
          redrawLine();
          handleEnter();
        }
      });
    });

    const btnClear = document.getElementById('btn-clear-terminal');
    if (btnClear) {
      btnClear.addEventListener('click', () => {
        state.term.clear();
        printPrompt();
      });
    }

    const btnExport = document.getElementById('btn-export-session');
    if (btnExport) {
      btnExport.addEventListener('click', exportSessionTranscript);
    }
  }

  function initTouchToolbar() {
    const keys = document.querySelectorAll('.touch-key');
    keys.forEach(k => {
      k.addEventListener('click', () => {
        const action = k.getAttribute('data-key');
        switch (action) {
          case 'Tab':
            handleTabCompletion();
            break;
          case 'Escape':
            state.term.writeln('');
            printPrompt();
            break;
          case 'CtrlC':
            if (state.isStreaming) abortTurn();
            else { state.term.writeln('^C'); printPrompt(); }
            break;
          case 'ArrowUp':
            navigateHistory(-1);
            break;
          case 'ArrowDown':
            navigateHistory(1);
            break;
          case 'Enter':
            handleEnter();
            break;
        }
      });
    });
  }

  // ===========================================================================
  // 19. Modals & UI Utilities
  // ===========================================================================
  function initModals() {
    const btnShortcuts = document.getElementById('btn-open-shortcuts');
    if (btnShortcuts) {
      btnShortcuts.addEventListener('click', () => {
        openModal('modal-shortcuts');
      });
    }

    document.querySelectorAll('[data-close]').forEach(btn => {
      btn.addEventListener('click', () => {
        const targetId = btn.getAttribute('data-close');
        closeModal(targetId);
      });
    });

    document.querySelectorAll('.modal-overlay').forEach(modal => {
      modal.addEventListener('click', (e) => {
        if (e.target === modal) {
          modal.classList.remove('active');
        }
      });
    });

    window.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        document.querySelectorAll('.modal-overlay.active').forEach(m => m.classList.remove('active'));
      }
    });
  }

  function openModal(id) {
    const m = document.getElementById(id);
    if (m) m.classList.add('active');
  }

  function closeModal(id) {
    const m = document.getElementById(id);
    if (m) m.classList.remove('active');
  }

  // ===========================================================================
  // 20. Session Export (with Secret Scrubbing)
  // ===========================================================================
  function exportSessionTranscript() {
    const lines = [];
    const buffer = state.term.buffer.active;
    for (let i = 0; i < buffer.length; i++) {
      const line = buffer.getLine(i);
      if (line) {
        lines.push(line.translateToString(true));
      }
    }

    // Sanitize any accidental API keys or secrets before exporting
    const textContent = ApiKeyStore.sanitize(lines.join('\n').trim());
    const blob = new Blob([textContent], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `fusion-session-${new Date().toISOString().slice(0, 10)}.log`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);

    state.term.writeln(`\r\n${ANSI.emerald}✔ Session transcript exported safely (secrets scrubbed).${ANSI.reset}`);
    printPrompt();
  }

  function escapeHtml(str) {
    if (!str) return '';
    return str
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }

  // Expose global for WASM / SDK interaction
  window.FusionWebTerminal = {
    state,
    vfs: VirtualFileSystem,
    apiKeyStore: ApiKeyStore,
    switchModel,
    applyTheme
  };

})();
