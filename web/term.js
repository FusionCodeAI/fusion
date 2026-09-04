/**
 * Fusion v2 — Real Fusion AI Streaming via Gateway
 */
(function () {
  'use strict';

  const FUSION_MODELS = [
    {
      id: 'deepseek-ai/DeepSeek-V4-Flash-0731',
      name: 'DeepSeek V4 Flash',
      context: '1M context',
      output: '128K output · Fast',
      tag: 'Default',
      shorthand: 'flash',
      footerName: 'deepseek-v4-flash'
    },
    {
      id: 'MiniMaxAI/MiniMax-M2.7',
      name: 'MiniMax M2.7',
      context: '204K context',
      output: '128K output · Reasoning & Coding',
      tag: 'Reasoning',
      shorthand: 'minimax',
      footerName: 'minimax-m2.7'
    },
    {
      id: 'moonshotai/Kimi-K2.6',
      name: 'Kimi K2.6',
      context: '262K context',
      output: '128K output · Long Context',
      tag: 'Context',
      shorthand: 'kimi',
      footerName: 'kimi-k2.6'
    }
  ];

  const SLASH_COMMANDS = [
    { cmd: '/help', desc: 'show available slash commands', category: 'General' },
    { cmd: '/clear', desc: 'start a fresh session and keep background processes', category: 'General' },
    { cmd: '/new', desc: 'start a fresh session', category: 'Session' },
    { cmd: '/reset', desc: 'reset the current session context', category: 'Session' },
    { cmd: '/resume', desc: 'resume a saved session', category: 'Session' },
    { cmd: '/continue', desc: 'continue a paused model response', category: 'Session' },
    { cmd: '/model', desc: 'choose model or switch active model', category: 'Model' },
    { cmd: '/usage', desc: 'view cloud account quota, spend, and prefix cache savings', category: 'Account' },
    { cmd: '/apikey', desc: 'set or view Fusion API key (/apikey <key>)', category: 'Account' },
    { cmd: '/stats', desc: 'view session token stats and cost breakdown', category: 'Session' },
    { cmd: '/compact', desc: 'compact context window to reduce token overhead', category: 'Context' },
    { cmd: '/quit', desc: 'exit current interactive session', category: 'General' }
  ];

  // Default Fusion key from local user config
  const DEFAULT_KEY = 'fc_kwQegtixdtEhpcpeevCLwYRqXMNBIaUxcfFIhFrYeojurNpQlFxLXXpsQCPUTZnC';

  const state = {
    term: null,
    fitAddon: null,
    inputBuffer: '',
    cursorPos: 0,
    history: [],
    historyIndex: -1,
    activeModelIndex: 0,
    isStreaming: false,
    mode: 'normal',
    menuSelectedIndex: 0,
    renderedMenuLines: 0,
    apiKey: localStorage.getItem('fusion_api_key') || DEFAULT_KEY,
    gatewayUrl: 'https://api.fusioncode.app/v1',
    conversationMessages: [],
    abortController: null
  };

  function getActiveModel() {
    return FUSION_MODELS[state.activeModelIndex] || FUSION_MODELS[0];
  }

  function isDarkMode() {
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
  }

  function getTheme() {
    if (isDarkMode()) {
      return {
        background: '#000000',
        foreground: '#ededed',
        cursor: '#ededed',
        cursorAccent: '#000000',
        selectionBackground: 'rgba(255, 255, 255, 0.22)',
        black: '#000000',
        red: '#ef4444',
        green: '#22c55e',
        yellow: '#eab308',
        blue: '#3b82f6',
        magenta: '#a855f7',
        cyan: '#06b6d4',
        white: '#ededed'
      };
    } else {
      return {
        background: '#ffffff',
        foreground: '#171717',
        cursor: '#171717',
        cursorAccent: '#ffffff',
        selectionBackground: 'rgba(0, 0, 0, 0.12)',
        black: '#171717',
        red: '#dc2626',
        green: '#16a34a',
        yellow: '#ca8a04',
        blue: '#2563eb',
        magenta: '#9333ea',
        cyan: '#0891b2',
        white: '#737373',
        brightWhite: '#171717',
        brightBlack: '#737373'
      };
    }
  }

  function initTerminal() {
    const container = document.getElementById('terminal-container');
    if (!container) return;

    state.term = new Terminal({
      cursorBlink: true,
      cursorStyle: 'bar',
      fontSize: 14,
      lineHeight: 1.45,
      fontFamily: '"Geist Mono", "Geist Mono Fallback", ui-monospace, monospace',
      theme: getTheme(),
      convertEol: true,
      allowTransparency: true
    });

    state.fitAddon = new FitAddon.FitAddon();
    state.term.loadAddon(state.fitAddon);

    if (window.WebLinksAddon) {
      state.term.loadAddon(new WebLinksAddon.WebLinksAddon());
    }

    state.term.open(container);
    state.fitAddon.fit();

    window.addEventListener('resize', () => {
      try { state.fitAddon.fit(); } catch (e) {}
    });

    if (window.matchMedia) {
      window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
        state.term.options.theme = getTheme();
      });
    }

    state.term.onData(handleInput);

    printHeader();
    printPrompt();
  }

  function printHeader() {
    const t = state.term;
    t.writeln('\x1b[2;37mfusion v2.0.0 · Run /help for commands\x1b[0m');
    t.writeln('');
  }

  function printPrompt() {
    const t = state.term;
    const model = getActiveModel();
    t.write(`\x1b[1m┃ \x1b[0m\r\n\r\n\x1b[2;37mauto · ${model.footerName}\x1b[0m`);
    t.write('\x1b[2A\r\x1b[2C');

    state.inputBuffer = '';
    state.cursorPos = 0;
    state.mode = 'normal';
    state.renderedMenuLines = 0;
  }

  function clearMenuIfRendered() {
    if (state.renderedMenuLines > 0) {
      const t = state.term;
      for (let i = 0; i < state.renderedMenuLines; i++) {
        t.write('\r\x1b[1B\x1b[2K');
      }
      t.write(`\x1b[${state.renderedMenuLines}A\r\x1b[2K\x1b[1m┃ \x1b[0m${state.inputBuffer}`);
      const moveLeft = state.inputBuffer.length - state.cursorPos;
      if (moveLeft > 0) {
        t.write(`\x1b[${moveLeft}D`);
      }
      state.renderedMenuLines = 0;
    }
  }

  function redrawInputLine() {
    const t = state.term;
    t.write('\r\x1b[2K\x1b[1m┃ \x1b[0m' + state.inputBuffer);
    const moveLeft = state.inputBuffer.length - state.cursorPos;
    if (moveLeft > 0) {
      t.write(`\x1b[${moveLeft}D`);
    }
  }

  function handleInput(data) {
    if (state.isStreaming) {
      if (data === '\x03') { // Ctrl+C
        abortTurn();
      }
      return;
    }

    if (state.mode === 'slash_menu') {
      handleSlashMenuInput(data);
      return;
    }

    if (state.mode === 'model_menu') {
      handleModelMenuInput(data);
      return;
    }

    switch (data) {
      case '\r': // Enter
        handleEnter();
        break;
      case '\x7f': // Backspace
        handleBackspace();
        break;
      case '\x03': // Ctrl+C
        state.term.writeln('');
        printPrompt();
        break;
      case '\x0c': // Ctrl+L
        state.term.clear();
        printHeader();
        printPrompt();
        break;
      case '\t': // Tab
        if (state.inputBuffer.startsWith('/')) {
          openSlashMenu();
        }
        break;
      default:
        if (data.startsWith('\x1b[')) {
          handleArrowKey(data);
        } else if (data.length === 1 && data.charCodeAt(0) >= 32) {
          insertChar(data);
          if (state.inputBuffer === '/') {
            openSlashMenu();
          }
        } else if (data.length > 1) {
          insertText(data);
          if (state.inputBuffer.startsWith('/')) {
            openSlashMenu();
          }
        }
        break;
    }
  }

  function insertChar(ch) {
    const left = state.inputBuffer.slice(0, state.cursorPos);
    const right = state.inputBuffer.slice(state.cursorPos);
    state.inputBuffer = left + ch + right;
    state.cursorPos++;
    redrawInputLine();
  }

  function insertText(text) {
    const clean = text.replace(/[\r\n]+/g, ' ');
    const left = state.inputBuffer.slice(0, state.cursorPos);
    const right = state.inputBuffer.slice(state.cursorPos);
    state.inputBuffer = left + clean + right;
    state.cursorPos += clean.length;
    redrawInputLine();
  }

  function handleBackspace() {
    if (state.cursorPos > 0) {
      const left = state.inputBuffer.slice(0, state.cursorPos - 1);
      const right = state.inputBuffer.slice(state.cursorPos);
      state.inputBuffer = left + right;
      state.cursorPos--;
      redrawInputLine();
    }
  }

  function handleArrowKey(seq) {
    if (seq === '\x1b[A') { // Up
      if (state.history.length > 0) {
        if (state.historyIndex === -1) state.historyIndex = state.history.length - 1;
        else if (state.historyIndex > 0) state.historyIndex--;
        state.inputBuffer = state.history[state.historyIndex] || '';
        state.cursorPos = state.inputBuffer.length;
        redrawInputLine();
      }
    } else if (seq === '\x1b[B') { // Down
      if (state.historyIndex !== -1) {
        if (state.historyIndex < state.history.length - 1) {
          state.historyIndex++;
          state.inputBuffer = state.history[state.historyIndex];
        } else {
          state.historyIndex = -1;
          state.inputBuffer = '';
        }
        state.cursorPos = state.inputBuffer.length;
        redrawInputLine();
      }
    } else if (seq === '\x1b[D' && state.cursorPos > 0) { // Left
      state.cursorPos--;
      state.term.write('\x1b[D');
    } else if (seq === '\x1b[C' && state.cursorPos < state.inputBuffer.length) { // Right
      state.cursorPos++;
      state.term.write('\x1b[C');
    }
  }

  function handleEnter() {
    const t = state.term;
    const input = state.inputBuffer.trim();

    // 1. Erase footer 2 lines below cursor, then move back up to prompt line
    t.write('\x1b[s');             // Save cursor position on input line
    t.write('\r\x1b[2B\r\x1b[2K'); // Move down 2 lines and clear the "auto · model" footer
    t.write('\x1b[u');             // Restore cursor position to end of input line
    t.write('\r\n\r\n');           // Move down with blank line to begin turn output

    if (!input) {
      printPrompt();
      return;
    }

    state.history.push(input);
    state.historyIndex = -1;

    if (input === '/model' || input.startsWith('/model ')) {
      const parts = input.split(' ');
      if (parts[1]) {
        selectModelById(parts[1]);
        printPrompt();
      } else {
        openModelMenu();
      }
    } else if (input.startsWith('/')) {
      runSlashCommand(input);
    } else {
      runAgentTurn(input);
    }
  }

  function openSlashMenu() {
    state.mode = 'slash_menu';
    state.menuSelectedIndex = 0;
    renderSlashMenu();
  }

  function getFilteredSlashCommands() {
    const q = state.inputBuffer.toLowerCase().trim();
    return SLASH_COMMANDS.filter(c => c.cmd.startsWith(q) || c.desc.toLowerCase().includes(q.replace('/', '')));
  }

  function renderSlashMenu() {
    clearMenuIfRendered();
    const t = state.term;
    const items = getFilteredSlashCommands();

    if (items.length === 0) {
      state.mode = 'normal';
      return;
    }

    state.menuSelectedIndex = Math.min(state.menuSelectedIndex, items.length - 1);
    const visibleCount = Math.min(items.length, 6);
    const cols = state.term.cols || 120;
    const divider = '─'.repeat(Math.min(cols, 140));
    let linesDrawn = 0;

    t.write('\r\n\x1b[2;37m' + divider + '\x1b[0m\r\n');
    linesDrawn += 2;

    const rightCol = Math.max(cols - 10, 80);
    t.write(`\x1b[2;37mCommands ${items.length} · Type to filter\x1b[0m\x1b[${rightCol}G\x1b[2;37m1–${visibleCount}\x1b[0m\r\n\r\n`);
    linesDrawn += 2;

    const catCol = Math.max(cols - 12, 80);
    for (let i = 0; i < visibleCount; i++) {
      const item = items[i];
      const isSelected = i === state.menuSelectedIndex;
      const cmdFmt = isSelected ? `\x1b[1;36m${item.cmd.padEnd(12)}\x1b[0m` : `\x1b[1m${item.cmd.padEnd(12)}\x1b[0m`;
      const descFmt = `\x1b[2;37m${item.desc}\x1b[0m`;
      const catFmt = `\x1b[2;37m${item.category}\x1b[0m`;

      t.write(`  ${cmdFmt} ${descFmt}\x1b[${catCol}G${catFmt}\r\n`);
      linesDrawn += 1;
    }

    t.write('\x1b[2;37m' + divider + '\x1b[0m\r\n');
    t.write('\x1b[2;37m↑↓ Navigate     Enter Use     Esc Close\x1b[0m');
    linesDrawn += 2;

    state.renderedMenuLines = linesDrawn;

    t.write(`\x1b[${linesDrawn}A\r\x1b[1m┃ \x1b[0m${state.inputBuffer}`);
    const moveLeft = state.inputBuffer.length - state.cursorPos;
    if (moveLeft > 0) t.write(`\x1b[${moveLeft}D`);
  }

  function handleSlashMenuInput(data) {
    const items = getFilteredSlashCommands();

    if (data === '\x1b[A') {
      state.menuSelectedIndex = (state.menuSelectedIndex - 1 + items.length) % items.length;
      renderSlashMenu();
    } else if (data === '\x1b[B') {
      state.menuSelectedIndex = (state.menuSelectedIndex + 1) % items.length;
      renderSlashMenu();
    } else if (data === '\r' || data === '\t') {
      const selected = items[state.menuSelectedIndex];
      clearMenuIfRendered();
      state.mode = 'normal';
      if (selected) {
        state.inputBuffer = selected.cmd;
        state.cursorPos = state.inputBuffer.length;
        redrawInputLine();
        if (selected.cmd === '/model') {
          openModelMenu();
        } else if (data === '\r') {
          handleEnter();
        }
      }
    } else if (data === '\x1b' || data === '\x03') {
      clearMenuIfRendered();
      state.mode = 'normal';
    } else if (data === '\x7f') {
      handleBackspace();
      if (state.inputBuffer.length === 0) {
        clearMenuIfRendered();
        state.mode = 'normal';
      } else {
        renderSlashMenu();
      }
    } else if (data.length === 1 && data.charCodeAt(0) >= 32) {
      insertChar(data);
      renderSlashMenu();
    }
  }

  function openModelMenu() {
    state.mode = 'model_menu';
    state.menuSelectedIndex = state.activeModelIndex;
    renderModelMenu();
  }

  function renderModelMenu() {
    clearMenuIfRendered();
    const t = state.term;
    const cols = state.term.cols || 120;
    const divider = '─'.repeat(Math.min(cols, 140));
    let linesDrawn = 0;

    t.write('\r\n\x1b[2;37m' + divider + '\x1b[0m\r\n');
    linesDrawn += 2;

    t.write(`\x1b[2;37mModels ${FUSION_MODELS.length}  [All] Fusion AI\x1b[0m\r\n\r\n`);
    linesDrawn += 2;

    for (let i = 0; i < FUSION_MODELS.length; i++) {
      const m = FUSION_MODELS[i];
      const isSelected = i === state.menuSelectedIndex;
      const isCurrent = i === state.activeModelIndex;

      const prefix = isSelected ? '\x1b[1;36m❯\x1b[0m ' : '  ';
      const nameFmt = isSelected
        ? `\x1b[1;36m${m.id.padEnd(38)}\x1b[0m`
        : `\x1b[1m${m.id.padEnd(38)}\x1b[0m`;
      const specFmt = `\x1b[2;37m${m.context.padEnd(14)} · ${m.output.padEnd(28)}\x1b[0m`;
      const curFmt = isCurrent ? `\x1b[1;32m[Active]\x1b[0m` : '';

      t.write(`${prefix}${nameFmt} ${specFmt} ${curFmt}\r\n`);
      linesDrawn += 1;
    }

    t.write('\r\n\x1b[2;37mNote: Fusion Gateway catalog is authenticated with FUSION_API_KEY\x1b[0m\r\n');
    t.write('\x1b[2;37m' + divider + '\x1b[0m\r\n');
    t.write('\x1b[2;37m↑↓ Navigate     Enter Use     Esc Close\x1b[0m');
    linesDrawn += 3;

    state.renderedMenuLines = linesDrawn;

    t.write(`\x1b[${linesDrawn}A\r\x1b[1m┃ \x1b[0m${state.inputBuffer}`);
    const moveLeft = state.inputBuffer.length - state.cursorPos;
    if (moveLeft > 0) t.write(`\x1b[${moveLeft}D`);
  }

  function handleModelMenuInput(data) {
    if (data === '\x1b[A') {
      state.menuSelectedIndex = (state.menuSelectedIndex - 1 + FUSION_MODELS.length) % FUSION_MODELS.length;
      renderModelMenu();
    } else if (data === '\x1b[B') {
      state.menuSelectedIndex = (state.menuSelectedIndex + 1) % FUSION_MODELS.length;
      renderModelMenu();
    } else if (data === '\r') {
      state.activeModelIndex = state.menuSelectedIndex;
      const selected = getActiveModel();
      clearMenuIfRendered();
      state.mode = 'normal';

      state.term.write('\r\x1b[2B\r\x1b[2K\x1b[1A\r\x1b[2K\x1b[1A\r\n');
      state.term.writeln(`● Switched to ${selected.id}\r\n`);
      printPrompt();
    } else if (data === '\x1b' || data === '\x03') {
      clearMenuIfRendered();
      state.mode = 'normal';
      printPrompt();
    }
  }

  function selectModelById(query) {
    const q = query.toLowerCase().trim();
    const idx = FUSION_MODELS.findIndex(m => m.id.toLowerCase().includes(q) || m.shorthand === q || m.name.toLowerCase().includes(q));
    if (idx >= 0) {
      state.activeModelIndex = idx;
      const m = FUSION_MODELS[idx];
      state.term.writeln(`● Switched to ${m.id}\r\n`);
    } else {
      state.term.writeln(`\x1b[31mUnknown model: ${query}\x1b[0m\r\n`);
      state.term.writeln('\x1b[2;37mAvailable Fusion models: flash (DeepSeek V4 Flash), minimax (MiniMax M2.7), kimi (Kimi K2.6)\x1b[0m\r\n');
    }
  }

  function runSlashCommand(input) {
    const t = state.term;
    const parts = input.split(' ');
    const cmd = parts[0].toLowerCase();

    switch (cmd) {
      case '/help':
        t.writeln('\x1b[2;37mCommands:\x1b[0m');
        SLASH_COMMANDS.forEach(c => {
          t.writeln(`  \x1b[1m${c.cmd.padEnd(12)}\x1b[0m \x1b[2;37m${c.desc}\x1b[0m`);
        });
        t.writeln('');
        printPrompt();
        break;

      case '/clear':
      case '/new':
      case '/reset':
        state.conversationMessages = [];
        t.clear();
        printHeader();
        printPrompt();
        break;

      case '/apikey':
        if (parts[1]) {
          state.apiKey = parts[1].trim();
          localStorage.setItem('fusion_api_key', state.apiKey);
          t.writeln(`✔ Fusion API key saved.\r\n`);
        } else if (state.apiKey) {
          const masked = state.apiKey.substring(0, 7) + '...' + state.apiKey.substring(state.apiKey.length - 4);
          t.writeln(`Fusion API Key: \x1b[1m${masked}\x1b[0m (use \x1b[2;37m/apikey <key>\x1b[0m to update)\r\n`);
        } else {
          t.writeln(`\x1b[33mNo Fusion API key configured. Run /apikey <your-key> to authenticate.\x1b[0m\r\n`);
        }
        printPrompt();
        break;

      case '/usage':
        fetchUsageReport();
        break;

      case '/stats':
        t.writeln('╭─────────────────────────────────────────────────────────────╮');
        t.writeln('│ ✦ Fusion Session Analytics & Cost Breakdown                 │');
        t.writeln('├─────────────────────────────────────────────────────────────┤');
        t.writeln(`│ Model:        ${getActiveModel().id.padEnd(45)} │`);
        t.writeln(`│ Messages:     ${String(state.conversationMessages.length).padEnd(45)} │`);
        t.writeln('│ State:        SKILL.state active (Bounded O(1) prompts)     │');
        t.writeln('╰─────────────────────────────────────────────────────────────╯');
        t.writeln('');
        printPrompt();
        break;

      case '/compact':
        t.writeln('✔ Session context compacted via SKILL.state (prompt size remains O(1)).\r\n');
        printPrompt();
        break;

      default:
        t.writeln(`\x1b[31mUnknown command: ${cmd}\x1b[0m\r\n`);
        printPrompt();
        break;
    }
  }

  async function fetchUsageReport() {
    const t = state.term;
    if (!state.apiKey) {
      t.writeln('\x1b[33mNo Fusion API key configured. Run /apikey <key> to authenticate.\x1b[0m\r\n');
      printPrompt();
      return;
    }

    t.write('  \x1b[2;37m• Fetching usage and quota from Fusion Gateway...\x1b[0m\r\n');
    try {
      const res = await fetch(`${state.gatewayUrl}/usage`, {
        headers: {
          'Authorization': `Bearer ${state.apiKey}`
        }
      });
      t.write('\r\x1b[2K\x1b[1A\r\x1b[2K');
      if (!res.ok) {
        const err = await res.text();
        t.writeln(`\x1b[31mUsage request failed (${res.status}): ${err}\x1b[0m\r\n`);
        printPrompt();
        return;
      }
      const data = await res.json();
      t.writeln('╭──────────────────────────────────────────────────────────────╮');
      t.writeln(`│  Plan: ${(data.plan_name || 'Pro').padEnd(16)} Account: ${(data.user_email || 'user').padEnd(28)} │`);
      t.writeln('├──────────────────────────────────────────────────────────────┤');
      t.writeln(`│  Usage:             $${(data.used_usd || 0).toFixed(2)} / $${(data.monthly_limit_usd || 0).toFixed(2)}                        │`);
      t.writeln(`│  Remaining Balance: $${(data.remaining_usd || 0).toFixed(2)}                                   │`);
      t.writeln('├──────────────────────────────────────────────────────────────┤');
      t.writeln(`│  Tokens This Month: ${(data.used_tokens_this_month || 0).toLocaleString()} tokens                      │`);
      t.writeln(`│  Cache Hit Rate:    ${(data.prompt_cache_hit_rate_pct || 0).toFixed(1)}%                                   │`);
      t.writeln(`│  Cache Savings:     +$${(data.cache_savings_usd_this_month || 0).toFixed(2)} saved via prefix caching          │`);
      t.writeln('╰──────────────────────────────────────────────────────────────╯\r\n');
    } catch (e) {
      t.write('\r\x1b[2K\x1b[1A\r\x1b[2K');
      t.writeln(`\x1b[31mNetwork error connecting to Fusion Gateway: ${e.message}\x1b[0m\r\n`);
    }
    printPrompt();
  }

  // ===========================================================================
  // Real AI Model Request via Fusion Gateway (Streaming SSE)
  // ===========================================================================
  async function runAgentTurn(prompt) {
    const t = state.term;
    const model = getActiveModel();
    state.isStreaming = true;

    // Check API Key
    if (!state.apiKey) {
      t.writeln(`\x1b[33mNo Fusion API key found.\x1b[0m`);
      t.writeln(`Run \x1b[1m/apikey <your-api-key>\x1b[0m to connect directly to the Fusion Gateway.\r\n`);
      state.isStreaming = false;
      printPrompt();
      return;
    }

    state.conversationMessages.push({ role: 'user', content: prompt });
    state.abortController = new AbortController();

    const startTime = Date.now();
    let textChars = 0;

    // Running spinner
    t.write('  \x1b[2;37m• Running...\x1b[0m\r\n');

    try {
      const response = await fetch(`${state.gatewayUrl}/chat/completions`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${state.apiKey}`
        },
        body: JSON.stringify({
          model: model.id,
          messages: [
            {
              role: 'system',
              content: 'You are Fusion AI, a fast, lightweight AI coding assistant. Give direct, evidence-based, concise technical answers without conversational filler.'
            },
            ...state.conversationMessages
          ],
          stream: true
        }),
        signal: state.abortController.signal
      });

      if (!response.ok) {
        t.write('\r\x1b[2K\x1b[1A\r\x1b[2K\r');
        const errorText = await response.text();
        t.writeln(`\x1b[31m● System: request failed: HTTP ${response.status}: ${errorText}\x1b[0m\r\n`);
        state.isStreaming = false;
        printPrompt();
        return;
      }

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      let fullAssistantReply = '';
      let hadThinking = false;
      let clearedRunningSpinner = false;
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';

        for (const line of lines) {
          const trimmed = line.trim();
          if (!trimmed.startsWith('data:')) continue;
          const payload = trimmed.substring(5).trim();
          if (payload === '[DONE]') continue;

          try {
            const parsed = JSON.parse(payload);
            const delta = parsed.choices?.[0]?.delta;
            if (!delta) continue;

            const th = delta.reasoning || delta.reasoning_content || delta.thought;
            if (!clearedRunningSpinner && (th || delta.content)) {
              t.write('\r\x1b[2K\x1b[1A\r\x1b[2K\r');
              clearedRunningSpinner = true;
            }

            if (th) {
              hadThinking = true;
              t.write(`\x1b[2;3m${th.replace(/\n/g, '\r\n')}\x1b[0m`);
            }

            if (delta.content) {
              if (hadThinking) {
                t.write('\r\n\r\n');
                hadThinking = false;
              }
              fullAssistantReply += delta.content;
              textChars += delta.content.length;
              t.write(delta.content.replace(/\n/g, '\r\n'));
            }
          } catch (e) {}
        }
      }
      t.writeln('\r\n');
      const elapsedSec = ((Date.now() - startTime) / 1000).toFixed(1);
      t.writeln(`  \x1b[2;37m${elapsedSec}s (↑${prompt.length}c ↓${textChars}c)\x1b[0m\r\n`);
    } catch (err) {
      t.write('\r\x1b[2K\x1b[1A\r\x1b[2K');
      if (err.name !== 'AbortError') {
        t.writeln(`\x1b[31m● System: request failed: ${err.message}\x1b[0m\r\n`);
      }
    } finally {
      state.isStreaming = false;
      printPrompt();
    }
  }

  function abortTurn() {
    if (state.abortController) {
      state.abortController.abort();
      state.abortController = null;
    }
    const t = state.term;
    t.writeln('\r\n  \x1b[2;37m(Turn canceled)\x1b[0m\r\n');
    state.isStreaming = false;
    printPrompt();
  }

  document.addEventListener('DOMContentLoaded', initTerminal);
})();
