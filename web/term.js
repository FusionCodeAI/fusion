/**
 * Fusion v2 — Full-page in-browser terminal matching fx.sh/try exact parity
 */
(function () {
  'use strict';

  // Fusion Official Models
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
    { cmd: '/stats', desc: 'view session token stats and cost breakdown', category: 'Session' },
    { cmd: '/compact', desc: 'compact context window to reduce token overhead', category: 'Context' },
    { cmd: '/quit', desc: 'exit current interactive session', category: 'General' }
  ];

  const state = {
    term: null,
    fitAddon: null,
    inputBuffer: '',
    cursorPos: 0,
    history: [],
    historyIndex: -1,
    activeModelIndex: 0,
    isStreaming: false,
    mode: 'normal', // 'normal' | 'slash_menu' | 'model_menu'
    menuSelectedIndex: 0,
    renderedMenuLines: 0
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

    // Initial Screen (Exact fx parity)
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
    // Render:
    // Line 1: ┃ <input>
    // Line 2: <blank>
    // Line 3: auto · <model_name>
    t.write(`\x1b[1m┃ \x1b[0m\r\n\r\n\x1b[2;37mauto · ${model.footerName}\x1b[0m`);
    // Now move cursor back up 2 lines and to column 2 (immediately after "┃ ")
    t.write('\x1b[2A\r\x1b[2C');

    state.inputBuffer = '';
    state.cursorPos = 0;
    state.mode = 'normal';
    state.renderedMenuLines = 0;
  }

  function clearMenuIfRendered() {
    if (state.renderedMenuLines > 0) {
      const t = state.term;
      // Clear all lines drawn below the input line
      for (let i = 0; i < state.renderedMenuLines; i++) {
        t.write('\r\x1b[1B\x1b[2K');
      }
      // Move back up to input line and restore prompt + input
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
    // Clear only current line, redraw prompt and buffer
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

    // Move past the blank line and footer so output appears naturally below
    t.write('\r\x1b[2B\r\x1b[2K\x1b[1A\r\x1b[2K\x1b[1A\r\n\r\n');

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

  // ===========================================================================
  // Slash Command Interactive In-Terminal Menu (Directly under input line)
  // ===========================================================================
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

    // Header divider immediately below input line
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

    // Bottom divider
    t.write('\x1b[2;37m' + divider + '\x1b[0m\r\n');
    t.write('\x1b[2;37m↑↓ Navigate     Enter Use     Esc Close\x1b[0m');
    linesDrawn += 2;

    state.renderedMenuLines = linesDrawn;

    // Return cursor to input line immediately after input text
    t.write(`\x1b[${linesDrawn}A\r\x1b[1m┃ \x1b[0m${state.inputBuffer}`);
    const moveLeft = state.inputBuffer.length - state.cursorPos;
    if (moveLeft > 0) t.write(`\x1b[${moveLeft}D`);
  }

  function handleSlashMenuInput(data) {
    const items = getFilteredSlashCommands();

    if (data === '\x1b[A') { // Up
      state.menuSelectedIndex = (state.menuSelectedIndex - 1 + items.length) % items.length;
      renderSlashMenu();
    } else if (data === '\x1b[B') { // Down
      state.menuSelectedIndex = (state.menuSelectedIndex + 1) % items.length;
      renderSlashMenu();
    } else if (data === '\r' || data === '\t') { // Enter or Tab
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
    } else if (data === '\x1b' || data === '\x03') { // Esc or Ctrl+C
      clearMenuIfRendered();
      state.mode = 'normal';
    } else if (data === '\x7f') { // Backspace
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

  // ===========================================================================
  // Models Interactive In-Terminal Menu (Directly under input line)
  // ===========================================================================
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

    // Header divider right under input cursor
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

    // Return cursor to input line
    t.write(`\x1b[${linesDrawn}A\r\x1b[1m┃ \x1b[0m${state.inputBuffer}`);
    const moveLeft = state.inputBuffer.length - state.cursorPos;
    if (moveLeft > 0) t.write(`\x1b[${moveLeft}D`);
  }

  function handleModelMenuInput(data) {
    if (data === '\x1b[A') { // Up
      state.menuSelectedIndex = (state.menuSelectedIndex - 1 + FUSION_MODELS.length) % FUSION_MODELS.length;
      renderModelMenu();
    } else if (data === '\x1b[B') { // Down
      state.menuSelectedIndex = (state.menuSelectedIndex + 1) % FUSION_MODELS.length;
      renderModelMenu();
    } else if (data === '\r') { // Enter
      state.activeModelIndex = state.menuSelectedIndex;
      const selected = getActiveModel();
      clearMenuIfRendered();
      state.mode = 'normal';

      // Move past prompt footer to write message
      state.term.write('\r\x1b[2B\r\x1b[2K\x1b[1A\r\x1b[2K\x1b[1A\r\n');
      state.term.writeln(`● Switched to ${selected.id}\r\n`);
      printPrompt();
    } else if (data === '\x1b' || data === '\x03') { // Esc or Ctrl+C
      clearMenuIfRendered();
      state.mode = 'normal';
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
        t.clear();
        printHeader();
        printPrompt();
        break;

      case '/usage':
        t.writeln('╭──────────────────────────────────────────────────────────────╮');
        t.writeln('│  Plan: Admin [PAYG]       Account: live-browser-demo         │');
        t.writeln('├──────────────────────────────────────────────────────────────┤');
        t.writeln('│  Usage:             $0.28 / $100.00 (0.3%)                   │');
        t.writeln('│  Quota:             [░░░░░░░░░░░░░░░░░░░░░░░░░░░░]   0.3%    │');
        t.writeln('│  Remaining Balance: $99.72                                   │');
        t.writeln('├──────────────────────────────────────────────────────────────┤');
        t.writeln('│  Tokens This Month: 4.46M tokens (4,464,222)                 │');
        t.writeln('│  Cache Hit Rate:    84.2% (3.47M cached tokens, 223 hits)    │');
        t.writeln('│  Cache Savings:     +$0.62 saved via prefix caching          │');
        t.writeln('╰──────────────────────────────────────────────────────────────╯');
        t.writeln('');
        printPrompt();
        break;

      case '/stats':
        t.writeln('╭─────────────────────────────────────────────────────────────╮');
        t.writeln('│ ✦ Fusion Session Analytics & Cost Breakdown                 │');
        t.writeln('├─────────────────────────────────────────────────────────────┤');
        t.writeln(`│ Model:        ${getActiveModel().id.padEnd(45)} │`);
        t.writeln('│ Prompt:       1,820 tokens (85.2% cache hit rate)           │');
        t.writeln('│ Completion:   420 tokens                                    │');
        t.writeln('│ Total Spend:  $0.0018                                       │');
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

  function runAgentTurn(prompt) {
    const t = state.term;
    state.isStreaming = true;

    t.writeln('  \x1b[2;37m• Running (1s) (↑280 ↓12)\x1b[0m\r\n');

    setTimeout(() => {
      t.writeln('\x1b[2K\r\x1b[2;37m● 1 tool call · 1 list\x1b[0m');
      t.writeln('\x1b[2;37m└ Matched src/**/*\x1b[0m\r\n');

      setTimeout(() => {
        const model = getActiveModel();
        t.writeln(`[${model.name}] I have analyzed your request: "${prompt}". All checks passed.\r\n`);
        t.writeln('  \x1b[2;37m1.2s (↑1.2k ↓48)\x1b[0m\r\n');
        state.isStreaming = false;
        printPrompt();
      }, 600);
    }, 500);
  }

  function abortTurn() {
    const t = state.term;
    t.writeln('\r\n  \x1b[2;37m(Turn canceled)\x1b[0m\r\n');
    state.isStreaming = false;
    printPrompt();
  }

  document.addEventListener('DOMContentLoaded', initTerminal);
})();
