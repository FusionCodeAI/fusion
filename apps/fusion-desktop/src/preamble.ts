// Guarantee React Fast Refresh preamble is initialized before any component executes
declare global {
  interface Window {
    $RefreshReg$?: (type: unknown, id: string) => void;
    $RefreshSig$?: () => (type: unknown) => unknown;
    __vite_plugin_react_preamble_installed__?: boolean;
  }
}

if (typeof window !== "undefined") {
  if (!window.$RefreshReg$) {
    window.$RefreshReg$ = () => {};
  }
  if (!window.$RefreshSig$) {
    window.$RefreshSig$ = () => (type: unknown) => type;
  }
  window.__vite_plugin_react_preamble_installed__ = true;
}

export {};
