/**
 * Fusion v2 — Minimalist Web Terminal matching fx.sh
 */
(function () {
  'use strict';

  const COMMANDS = [
    { cmd: '/help', desc: 'show available slash commands' },
    { cmd: '/usage', desc: 'view cloud account quota and cache savings' },
    { cmd: '/stats', desc: 'view session token stats and cost' },
    { cmd: '/clear', desc: 'start a fresh session and clear screen' },
    { cmd: '/model', desc: 'switch model or view model catalog' },
    { cmd: '/compact', desc: 'compact context window' },
    { cmd: '/quit', desc: 'exit current turn' }
  ];

  const state = {
    term: null,
    fitAddon: null,
    inputBuffer: '',
    cursorPos: 0,
    history: [],
    historyIndex: -1,
    activeModel: 'deepseek-ai/DeepSeek-V4-Flash-0731',
    isStreaming: false,
    selectedSlashIndex: 0
  };

  function isDarkMode() {
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches;
  }

  function getTheme() {
    if (isDarkMode()) {
      return {
        background: '#000000',
        foreground: '#f5f5f5',
        cursor: '#f5f5f5',
        cursorAccent: '#000000',
        selectionBackground: 'rgba(255, 255, 255, 0.25)',
        black: '#000000',
        red: '#ef4444',
        green: '#22c55e',
        yellow: '#eab308',
        blue: '#3b82f6',
        magenta: '#a855f7',
        cyan: '#06b6d4',
        white: '#f5f5f5'
      };
    } else {
      return {
        background: '#ffffff',
        foreground: '#171717',
        cursor: '#171717',
        cursorAccent: '#ffffff',
        selectionBackground: 'rgba(0, 0, 0, 0.15)',
        black: '#171717',
        red: '#dc2626',
        green: '#16a34a',
        yellow: '#ca8a04',
        blue: '#2563eb',
        magenta: '#9333ea',
        cyan: '#0891b2',
        white: '#ffffff'
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
      lineHeight: 1.4,
      fontFamily: '"Geist Mono", "SF Mono", Menlo, Monaco, Consolas, monospace',
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
    // Exactly 1 vertical bar prompt line, followed by docked status
    t.write('\x1b[1m┃ \x1b[0m');
    state.inputBuffer = '';
    state.cursorPos = 0;
    hideSlashPopup();
  }

  function handleInput(data) {
    if (state.isStreaming) {
      if (data === '\x03') { // Ctrl+C
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
        handleTab();
        break;
      default:
        if (data.startsWith('\x1b[')) {
          handleArrowKey(data);
        } else if (data.length === 1 && data.charCodeAt(0) >= 32) {
          insertChar(data);
        } else if (data.length > 1) {
          insertText(data);
        }
        break;
    }
  }

  function insertChar(ch) {
    const left = state.inputBuffer.slice(0, state.cursorPos);
    const right = state.inputBuffer.slice(state.cursorPos);
    state.inputBuffer = left + ch + right;
    state.cursorPos++;
    redrawLine();
    checkSlashPopup();
  }

  function insertText(text) {
    const clean = text.replace(/[\r\n]+/g, ' ');
    const left = state.inputBuffer.slice(0, state.cursorPos);
    const right = state.inputBuffer.slice(state.cursorPos);
    state.inputBuffer = left + clean + right;
    state.cursorPos += clean.length;
    redrawLine();
    checkSlashPopup();
  }

  function handleBackspace() {
    if (state.cursorPos > 0) {
      const left = state.inputBuffer.slice(0, state.cursorPos - 1);
      const right = state.inputBuffer.slice(state.cursorPos);
      state.inputBuffer = left + right;
      state.cursorPos--;
      redrawLine();
      checkSlashPopup();
    }
  }

  function redrawLine() {
    state.term.write('\r\x1b[2K\x1b[1m┃ \x1b[0m' + state.inputBuffer);
    const moveLeft = state.inputBuffer.length - state.cursorPos;
    if (moveLeft > 0) {
      state.term.write(`\x1b[${moveLeft}D`);
    }
  }

  function handleArrowKey(seq) {
    const popup = document.getElementById('slash-popup');
    const isPopupVisible = popup && popup.classList.contains('visible');

    if (seq === '\x1b[A') { // Up
      if (isPopupVisible) {
        moveSlashSelection(-1);
      } else if (state.history.length > 0) {
        if (state.historyIndex === -1) state.historyIndex = state.history.length - 1;
        else if (state.historyIndex > 0) state.historyIndex--;
        state.inputBuffer = state.history[state.historyIndex] || '';
        state.cursorPos = state.inputBuffer.length;
        redrawLine();
      }
    } else if (seq === '\x1b[B') { // Down
      if (isPopupVisible) {
        moveSlashSelection(1);
      } else if (state.historyIndex !== -1) {
        if (state.historyIndex < state.history.length - 1) {
          state.historyIndex++;
          state.inputBuffer = state.history[state.historyIndex];
        } else {
          state.historyIndex = -1;
          state.inputBuffer = '';
        }
        state.cursorPos = state.inputBuffer.length;
        redrawLine();
      }
    } else if (seq === '\x1b[D' && state.cursorPos > 0) { // Left
      state.cursorPos--;
      state.term.write('\x1b[D');
    } else if (seq === '\x1b[C' && state.cursorPos < state.inputBuffer.length) { // Right
      state.cursorPos++;
      state.term.write('\x1b[C');
    }
  }

  function handleTab() {
    const popup = document.getElementById('slash-popup');
    if (popup && popup.classList.contains('visible')) {
      const items = getFilteredCommands();
      if (items[state.selectedSlashIndex]) {
        state.inputBuffer = items[state.selectedSlashIndex].cmd + ' ';
        state.cursorPos = state.inputBuffer.length;
        redrawLine();
        hideSlashPopup();
      }
    }
  }

  function handleEnter() {
    const popup = document.getElementById('slash-popup');
    if (popup && popup.classList.contains('visible')) {
      const items = getFilteredCommands();
      if (items[state.selectedSlashIndex]) {
        state.inputBuffer = items[state.selectedSlashIndex].cmd;
        state.cursorPos = state.inputBuffer.length;
        redrawLine();
        hideSlashPopup();
      }
    }

    state.term.writeln('');
    const input = state.inputBuffer.trim();

    if (!input) {
      printPrompt();
      return;
    }

    state.history.push(input);
    state.historyIndex = -1;
    hideSlashPopup();

    if (input.startsWith('/')) {
      runSlashCommand(input);
    } else {
      runAgentTurn(input);
    }
  }

  function getFilteredCommands() {
    const query = state.inputBuffer.toLowerCase().trim();
    return COMMANDS.filter(c => c.cmd.startsWith(query) || c.desc.toLowerCase().includes(query.replace('/', '')));
  }

  function checkSlashPopup() {
    const popup = document.getElementById('slash-popup');
    if (!popup) return;

    if (state.inputBuffer.startsWith('/')) {
      const items = getFilteredCommands();
      if (items.length > 0) {
        state.selectedSlashIndex = Math.min(state.selectedSlashIndex, items.length - 1);
        renderSlashItems(items);
        popup.classList.add('visible');
      } else {
        hideSlashPopup();
      }
    } else {
      hideSlashPopup();
    }
  }

  function hideSlashPopup() {
    const popup = document.getElementById('slash-popup');
    if (popup) popup.classList.remove('visible');
    state.selectedSlashIndex = 0;
  }

  function renderSlashItems(items) {
    const list = document.getElementById('slash-list');
    const count = document.getElementById('slash-count');
    if (!list) return;

    if (count) count.textContent = items.length;

    list.innerHTML = items.map((item, idx) => `
      <div class="slash-item ${idx === state.selectedSlashIndex ? 'selected' : ''}">
        <span class="slash-cmd">${item.cmd}</span>
        <span class="slash-desc">${item.desc}</span>
      </div>
    `).join('');
  }

  function moveSlashSelection(delta) {
    const items = getFilteredCommands();
    if (items.length === 0) return;
    state.selectedSlashIndex = (state.selectedSlashIndex + delta + items.length) % items.length;
    renderSlashItems(items);
  }

  function runSlashCommand(input) {
    const t = state.term;
    const parts = input.split(' ');
    const cmd = parts[0].toLowerCase();

    switch (cmd) {
      case '/help':
        t.writeln('\x1b[2;37mCommands:\x1b[0m');
        COMMANDS.forEach(c => {
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

      case '/model':
        if (parts[1]) {
          state.activeModel = parts[1];
          t.writeln(`Switched model to \x1b[1m${state.activeModel}\x1b[0m\r\n`);
        } else {
          t.writeln(`Active model: \x1b[1m${state.activeModel}\x1b[0m (use \x1b[2;37m/model <name>\x1b[0m to switch)\r\n`);
        }
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

    // Simulate clean agent execution with tool tree
    t.writeln('  \x1b[2;37m• Running (1s) (↑280 ↓12)\x1b[0m\r\n');

    setTimeout(() => {
      t.writeln('\x1b[2K\r\x1b[2;37m● 1 tool call · 1 list\x1b[0m');
      t.writeln('\x1b[2;37m└ Matched src/**/*\x1b[0m\r\n');

      setTimeout(() => {
        t.writeln(`I have analyzed your request: "${prompt}". All checks passed.\r\n`);
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
