/**
 * Clean monochrome SVG icons matching the reference app 1:1.
 * GPUIX renders these as sharp native vector paths tinted with style.color.
 */

export const icons = {
  sidebarToggle: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round"><rect x="2" y="2.5" width="12" height="11" rx="2"/><path d="M6 2.5v11"/></svg>`,

  arrowLeft: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m10 3.5-4.5 4.5 4.5 4.5"/></svg>`,

  arrowRight: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m6 3.5 4.5 4.5-4.5 4.5"/></svg>`,

  newChat: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M11.5 2.5a1.4 1.4 0 0 1 2 2L5 13H2.5v-2.5l8-8z"/><path d="m10.5 3.5 2 2"/></svg>`,

  search: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="7" cy="7" r="4.5"/><path d="m10.5 10.5 3.5 3.5"/></svg>`,

  automations: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3.5" width="12" height="9" rx="2"/><path d="M6 8h4M8 2v1.5M4 14.5v-2M12 14.5v-2"/></svg>`,

  customize: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 5h4M11 5h2M3 11h2M9 11h4"/><circle cx="9" cy="5" r="2"/><circle cx="7" cy="11" r="2"/></svg>`,

  plus: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round"><path d="M8 3v10M3 8h10"/></svg>`,

  circleDashed: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-dasharray="2 2"><circle cx="8" cy="8" r="5.5"/></svg>`,

  home: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="m2.5 6.5 5.5-4 5.5 4V13a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1V6.5z"/><path d="M6 14V9h4v5"/></svg>`,

  filter: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 3.5h11l-4.5 5v4l-2 1v-5l-4.5-5z"/></svg>`,

  folder: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 4a1.5 1.5 0 0 1 1.5-1.5h2.88a1.5 1.5 0 0 1 1.06.44L8.5 4H12.5A1.5 1.5 0 0 1 14 5.5v7a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 12.5V4z"/></svg>`,

  github: `<svg viewBox="0 0 16 16" fill="#000"><path fill-rule="evenodd" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8z"/></svg>`,

  settings: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="8" cy="8" r="2.2"/><path d="M13.5 8a5.5 5.5 0 0 0-.1-1l1.1-.9-1-1.7-1.4.4a5.5 5.5 0 0 0-.9-.5L11 2.9H9l-.2 1.4a5.5 5.5 0 0 0-.9.5l-1.4-.4-1 1.7 1.1.9a5.5 5.5 0 0 0 0 2l-1.1.9 1 1.7 1.4-.4c.3.2.6.4.9.5l.2 1.4h2l.2-1.4c.3-.1.6-.3.9-.5l1.4.4 1-1.7-1.1-.9c.1-.3.1-.7.1-1z"/></svg>`,

  fileDrawer: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="12" height="10" rx="2"/><path d="M2 8h12M6 8a2 2 0 0 0 4 0"/></svg>`,

  browser: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="8" cy="8" r="6"/><path d="M2 8h12M8 2a9 9 0 0 1 2.5 6 9 9 0 0 1-2.5 6 9 9 0 0 1-2.5-6 9 9 0 0 1 2.5-6z"/></svg>`,

  terminal: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="12" height="10" rx="2"/><path d="m5 6.5 2 1.5-2 1.5M9 9.5h2"/></svg>`,

  files: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 2.5h6l4 4v7a1.5 1.5 0 0 1-1.5 1.5H3A1.5 1.5 0 0 1 1.5 13.5v-9.5A1.5 1.5 0 0 1 3 2.5z"/><path d="M9 2.5V6.5h4"/></svg>`,

  externalLink: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M12 8.5v4a1.5 1.5 0 0 1-1.5 1.5h-7A1.5 1.5 0 0 1 2 12.5v-7A1.5 1.5 0 0 1 3.5 4h4M9.5 2h4.5v4.5M6.5 9.5 14 2"/></svg>`,

  dotsHorizontal: `<svg viewBox="0 0 16 16" fill="#000"><circle cx="3.5" cy="8" r="1.2"/><circle cx="8" cy="8" r="1.2"/><circle cx="12.5" cy="8" r="1.2"/></svg>`,

  thumbsUp: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5.5 6.5V13h-2A1.5 1.5 0 0 1 2 11.5v-3.5A1.5 1.5 0 0 1 3.5 6.5h2zm0 0L8.5 2c.7 0 1.5.8 1.5 1.7v1.8h3.3c.7 0 1.2.6 1.1 1.3l-.8 5.2a1.5 1.5 0 0 1-1.5 1h-6.6"/></svg>`,

  thumbsDown: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5.5 9.5V3h-2A1.5 1.5 0 0 0 2 4.5V8a1.5 1.5 0 0 0 1.5 1.5h2zm0 0L8.5 14c.7 0 1.5-.8 1.5-1.7v-1.8h3.3c.7 0 1.2-.6 1.1-1.3l-.8-5.2a1.5 1.5 0 0 0-1.5-1h-6.6"/></svg>`,

  copy: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="5.5" y="5.5" width="7.5" height="7.5" rx="1.5"/><path d="M3.5 10.5h-1A1.5 1.5 0 0 1 1 9V3.5A1.5 1.5 0 0 1 2.5 2H8a1.5 1.5 0 0 1 1.5 1.5v1"/></svg>`,

  branch: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="4.5" cy="4" r="1.5"/><circle cx="4.5" cy="12" r="1.5"/><circle cx="11.5" cy="7" r="1.5"/><path d="M4.5 5.5v5M4.5 7.5a3.5 3.5 0 0 1 3.5-3.5h2"/></svg>`,

  mic: `<svg viewBox="0 0 16 16" fill="#000"><rect x="5.5" y="2" width="5" height="7.5" rx="2.5"/><path d="M3 7.5a5 5 0 0 0 10 0M8 12.5v2M6 14.5h4" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round"/></svg>`,

  laptop: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3.5" width="10" height="7" rx="1"/><path d="M1.5 12.5h13"/></svg>`,

  refresh: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 8a5.5 5.5 0 0 1 9.4-3.9L14 6M13.5 8a5.5 5.5 0 0 1-9.4 3.9L2 10"/><path d="M14 2.5V6h-3.5M2 13.5V10h3.5"/></svg>`,

  chevronDown: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m4 6 4 4 4-4"/></svg>`,

  chevronRight: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="m6 4 4 4-4 4"/></svg>`,

  close: `<svg viewBox="0 0 16 16" fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round"><path d="m4 4 8 8M12 4 4 12"/></svg>`,
};
