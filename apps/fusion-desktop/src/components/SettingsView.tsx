import React, { useState, useEffect } from "react";
import {
  ArrowLeft,
  X,
  Minus,
  Plus,
  Moon,
  Sun,
  Key,
  Check,
  Eye,
  EyeOff,
  Bell,
  User,
  LogOut,
  Sparkles,
  ExternalLink,
  ShieldCheck,
  Globe,
  Sliders,
} from "lucide-react";
import { getAllAvailableModels, type FusionModel } from "../models";
import { checkAuthStatus, type AuthStatus } from "../lib/fusion-ipc";

export type SettingsTab = "general" | "api" | "notifications" | "account";

export interface SettingsViewProps {
  onClose?: () => void;
  className?: string;
  onSignOut?: () => void;
}

const ACCENT_OPTIONS = [
  { id: "violet", label: "Violet", swatch: "#5100cd" },
  { id: "graphite", label: "Graphite", swatch: "#27272a" },
  { id: "cyan", label: "Cyan", swatch: "#06b6d4" },
  { id: "pink", label: "Pink", swatch: "#ec4899" },
  { id: "espresso", label: "Espresso", swatch: "#78350f" },
  { id: "ember", label: "Ember", swatch: "#ea580c" },
];

const NOTIF_STORAGE_KEY = "fusion_desktop_notification_settings_v1";

interface NotifSettings {
  taskCompletion: { enabled: boolean; sound: boolean };
  approvalNeeded: { enabled: boolean; sound: boolean };
  questionAsked: { enabled: boolean; sound: boolean };
  sessionError: { enabled: boolean; sound: boolean };
}

const DEFAULT_NOTIF: NotifSettings = {
  taskCompletion: { enabled: true, sound: true },
  approvalNeeded: { enabled: true, sound: true },
  questionAsked: { enabled: true, sound: false },
  sessionError: { enabled: true, sound: true },
};

export function SettingsView({
  onClose,
  className = "",
  onSignOut,
}: SettingsViewProps) {
  const [tab, setTab] = useState<SettingsTab>("general");

  // General settings state
  const [darkMode, setDarkMode] = useState<boolean>(() => {
    if (typeof window !== "undefined" && window.localStorage) {
      return window.localStorage.getItem("fusion_desktop_theme") === "dark";
    }
    return false;
  });

  const [fontSize, setFontSize] = useState<number>(() => {
    if (typeof window !== "undefined" && window.localStorage) {
      const stored = window.localStorage.getItem("fusion_desktop_font_size");
      if (stored) return parseInt(stored, 10) || 14;
    }
    return 14;
  });

  const [accent, setAccent] = useState<string>(() => {
    if (typeof window !== "undefined" && window.localStorage) {
      return window.localStorage.getItem("fusion_desktop_accent") || "violet";
    }
    return "violet";
  });

  const [webSearchEnabled, setWebSearchEnabled] = useState(true);
  const [autoUpdateEnabled, setAutoUpdateEnabled] = useState(true);

  // Fusion API settings state
  const [apiKey, setApiKey] = useState("");
  const [showApiKey, setShowApiKey] = useState(false);
  const [isApiKeySaved, setIsApiKeySaved] = useState(false);
  const [authStatus, setAuthStatus] = useState<AuthStatus>({ is_signed_in: true, provider: "fusion" });
  const [models] = useState<FusionModel[]>(() => getAllAvailableModels());
  const [defaultModel, setDefaultModel] = useState<string>("deepseek-v4-flash-0731");

  // Notification settings state
  const [notifSettings, setNotifSettings] = useState<NotifSettings>(() => {
    if (typeof window !== "undefined" && window.localStorage) {
      try {
        const raw = window.localStorage.getItem(NOTIF_STORAGE_KEY);
        if (raw) return { ...DEFAULT_NOTIF, ...JSON.parse(raw) };
      } catch {
        // Fallback
      }
    }
    return DEFAULT_NOTIF;
  });

  useEffect(() => {
    checkAuthStatus().then((res) => {
      setAuthStatus(res);
    });
  }, []);

  const handleUpdateFontSize = (newSize: number) => {
    const clamped = Math.max(12, Math.min(20, newSize));
    setFontSize(clamped);
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.setItem("fusion_desktop_font_size", String(clamped));
      document.documentElement.style.fontSize = `${clamped}px`;
    }
  };

  const handleUpdateAccent = (accentId: string) => {
    setAccent(accentId);
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.setItem("fusion_desktop_accent", accentId);
      document.documentElement.setAttribute("data-cline-accent", accentId);
    }
  };

  const handleToggleDarkMode = (enabled: boolean) => {
    setDarkMode(enabled);
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.setItem("fusion_desktop_theme", enabled ? "dark" : "light");
      if (enabled) {
        document.documentElement.classList.add("dark");
      } else {
        document.documentElement.classList.remove("dark");
      }
    }
  };

  const updateNotif = (event: keyof NotifSettings, key: "enabled" | "sound", val: boolean) => {
    setNotifSettings((prev) => {
      const next = {
        ...prev,
        [event]: { ...prev[event], [key]: val },
      };
      if (typeof window !== "undefined" && window.localStorage) {
        window.localStorage.setItem(NOTIF_STORAGE_KEY, JSON.stringify(next));
      }
      return next;
    });
  };

  const handleSaveApiKey = () => {
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.setItem("fusion_api_key", apiKey);
    }
    setIsApiKeySaved(true);
    setTimeout(() => setIsApiKeySaved(false), 2000);
  };

  return (
    <div
      data-testid="settings-view"
      className={`h-full w-full overflow-y-auto bg-white text-zinc-900 ${className}`}
    >
      {/* PageFrame matching Cline px-18 py-10 */}
      <div className="max-w-4xl mx-auto px-8 md:px-12 py-8">
        {/* PageHeader matching Cline */}
        <section className="mb-6 flex items-start justify-between gap-6 max-[860px]:flex-col">
          <div className="min-w-0">
            <div className="flex items-center gap-3">
              {onClose && (
                <button
                  type="button"
                  data-testid="settings-back-btn"
                  onClick={onClose}
                  className="p-1.5 rounded-lg border border-zinc-200 hover:bg-zinc-100 text-zinc-600 transition-colors cursor-pointer mr-1"
                  title="Back to conversation"
                >
                  <ArrowLeft className="size-4" />
                </button>
              )}
              <h1 className="truncate text-2xl md:text-3xl font-semibold tracking-tight text-zinc-900">
                Settings
              </h1>
            </div>
            <p className="mt-2 text-sm text-zinc-500 max-w-2xl leading-relaxed">
              Manage desktop preferences for this browser and CLI environment.
            </p>
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {onClose && (
              <button
                type="button"
                data-testid="settings-close-top-btn"
                onClick={onClose}
                className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-700 hover:bg-zinc-100 transition-colors cursor-pointer"
                title="Close"
              >
                <X className="size-4" />
              </button>
            )}
          </div>
        </section>

        {/* Sub-tabs with underline active indicator matching Cline */}
        <div className="mb-8 flex items-center gap-0 border-b border-zinc-200">
          {(
            [
              { id: "general", label: "General" },
              { id: "api", label: "Fusion API & Models" },
              { id: "notifications", label: "Notifications" },
              { id: "account", label: "Account" },
            ] as const
          ).map((tabItem) => {
            const active = tab === tabItem.id;
            return (
              <button
                key={tabItem.id}
                type="button"
                data-testid={`settings-tab-${tabItem.id}`}
                aria-current={active ? "page" : undefined}
                onClick={() => setTab(tabItem.id)}
                className={`relative px-4 py-2.5 text-xs md:text-sm font-medium transition-colors cursor-pointer flex items-center gap-1.5 ${
                  active
                    ? "text-zinc-900 font-semibold"
                    : "text-zinc-500 hover:text-zinc-900"
                }`}
              >
                <span>{tabItem.label}</span>
                {active && (
                  <span className="absolute inset-x-0 -bottom-px h-0.5 bg-[#5100cd]" />
                )}
              </button>
            );
          })}
        </div>

        {/* Tab Content */}
        {tab === "general" && (
          <div className="space-y-6 max-w-2xl">
            {/* Dark Mode */}
            <div className="flex items-center justify-between gap-5 py-4 border-b border-zinc-100">
              <div>
                <p className="text-sm font-semibold text-zinc-900">Dark mode</p>
                <p className="text-xs text-zinc-500 mt-0.5">
                  Keep the desktop interface in dark mode on this browser.
                </p>
              </div>
              <button
                type="button"
                data-testid="settings-darkmode-toggle"
                onClick={() => handleToggleDarkMode(!darkMode)}
                className={`w-9 h-5 rounded-full transition-colors relative cursor-pointer ${
                  darkMode ? "bg-[#5100cd]" : "bg-zinc-200"
                }`}
              >
                <span
                  className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                    darkMode ? "translate-x-4" : "translate-x-0"
                  }`}
                />
              </button>
            </div>

            {/* Font Size */}
            <div className="flex items-center justify-between gap-5 py-4 border-b border-zinc-100">
              <div>
                <p className="text-sm font-semibold text-zinc-900">Font size</p>
                <p className="text-xs text-zinc-500 mt-0.5">
                  Adjust the size of text and interface elements throughout the app.
                </p>
              </div>
              <div className="flex items-center gap-3 shrink-0">
                <button
                  type="button"
                  data-testid="settings-font-decrease"
                  onClick={() => handleUpdateFontSize(fontSize - 1)}
                  disabled={fontSize <= 12}
                  className="w-7 h-7 rounded-md border border-zinc-200 hover:bg-zinc-100 flex items-center justify-center text-zinc-700 disabled:opacity-40 cursor-pointer"
                >
                  <Minus className="w-3.5 h-3.5" />
                </button>
                <span
                  data-testid="settings-font-display"
                  className="w-10 text-center font-mono text-xs font-semibold text-zinc-800"
                >
                  {fontSize}px
                </span>
                <button
                  type="button"
                  data-testid="settings-font-increase"
                  onClick={() => handleUpdateFontSize(fontSize + 1)}
                  disabled={fontSize >= 20}
                  className="w-7 h-7 rounded-md border border-zinc-200 hover:bg-zinc-100 flex items-center justify-center text-zinc-700 disabled:opacity-40 cursor-pointer"
                >
                  <Plus className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>

            {/* Accent Color */}
            <div className="flex items-center justify-between gap-5 py-4 border-b border-zinc-100">
              <div>
                <p className="text-sm font-semibold text-zinc-900">Accent color</p>
                <p className="text-xs text-zinc-500 mt-0.5">
                  Tint buttons, links, and active highlights across the app.
                </p>
              </div>
              <div className="flex items-center gap-2">
                {ACCENT_OPTIONS.map((opt) => (
                  <button
                    key={opt.id}
                    type="button"
                    title={opt.label}
                    onClick={() => handleUpdateAccent(opt.id)}
                    style={{ backgroundColor: opt.swatch }}
                    className={`w-6 h-6 rounded-full border border-black/10 transition-transform hover:scale-110 cursor-pointer ${
                      accent === opt.id ? "ring-2 ring-offset-2 ring-[#5100cd]" : ""
                    }`}
                  />
                ))}
              </div>
            </div>

            {/* Web Search Toggle */}
            <div className="flex items-center justify-between gap-5 py-4 border-b border-zinc-100">
              <div>
                <p className="text-sm font-semibold text-zinc-900">Web search</p>
                <p className="text-xs text-zinc-500 mt-0.5">
                  Allow models to search the web during tasks using Fusion live web search.
                </p>
              </div>
              <button
                type="button"
                data-testid="settings-websearch-toggle"
                onClick={() => setWebSearchEnabled(!webSearchEnabled)}
                className={`w-9 h-5 rounded-full transition-colors relative cursor-pointer ${
                  webSearchEnabled ? "bg-[#5100cd]" : "bg-zinc-200"
                }`}
              >
                <span
                  className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                    webSearchEnabled ? "translate-x-4" : "translate-x-0"
                  }`}
                />
              </button>
            </div>

            {/* Keep CLI up to date */}
            <div className="flex items-center justify-between gap-5 py-4 border-b border-zinc-100">
              <div>
                <p className="text-sm font-semibold text-zinc-900">Keep CLI up to date</p>
                <p className="text-xs text-zinc-500 mt-0.5">
                  Automatically update the fusion terminal command and background engine.
                </p>
              </div>
              <button
                type="button"
                onClick={() => setAutoUpdateEnabled(!autoUpdateEnabled)}
                className={`w-9 h-5 rounded-full transition-colors relative cursor-pointer ${
                  autoUpdateEnabled ? "bg-[#5100cd]" : "bg-zinc-200"
                }`}
              >
                <span
                  className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
                    autoUpdateEnabled ? "translate-x-4" : "translate-x-0"
                  }`}
                />
              </button>
            </div>
          </div>
        )}

        {/* Fusion API & Models Tab */}
        {tab === "api" && (
          <div className="space-y-6 max-w-2xl">
            {/* Status Card */}
            <div className="p-4 rounded-xl border border-purple-200 bg-purple-50/40 flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="w-9 h-9 rounded-lg bg-[#5100cd] text-white flex items-center justify-center shrink-0">
                  <Sparkles className="w-5 h-5" />
                </div>
                <div>
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-zinc-900">Fusion API Gateway</h3>
                    <span className="text-[10px] font-medium px-2 py-0.5 rounded-full bg-emerald-100 text-emerald-700">
                      Connected & Active
                    </span>
                  </div>
                  <p className="text-xs text-zinc-500 mt-0.5">
                    High-throughput inference with 90% prefix cache discounts.
                  </p>
                </div>
              </div>
              <span className="text-xs font-mono font-medium text-[#5100cd] bg-white px-2.5 py-1 rounded-md border border-purple-200">
                https://api.fusioncode.app/v1
              </span>
            </div>

            {/* API Key configuration */}
            <div className="p-5 rounded-xl border border-zinc-200 bg-zinc-50/50 space-y-3">
              <div className="flex items-center justify-between">
                <div>
                  <label className="text-xs font-semibold text-zinc-900 flex items-center gap-1.5">
                    <Key className="w-3.5 h-3.5 text-[#5100cd]" />
                    <span>Fusion API Key</span>
                  </label>
                  <p className="text-[11px] text-zinc-500 mt-0.5">
                    Device credentials automatically synced from ~/.fusion/config.json.
                  </p>
                </div>
                <button
                  type="button"
                  data-testid="settings-save-apikey-btn"
                  onClick={handleSaveApiKey}
                  className="inline-flex items-center gap-1.5 px-3 py-1 text-xs font-medium bg-[#5100cd] hover:bg-[#4300a8] text-white rounded-lg transition-colors cursor-pointer shadow-2xs"
                >
                  {isApiKeySaved ? (
                    <>
                      <Check className="w-3.5 h-3.5 text-emerald-300" />
                      <span>Saved</span>
                    </>
                  ) : (
                    <span>Update Key</span>
                  )}
                </button>
              </div>

              <div className="relative">
                <input
                  type={showApiKey ? "text" : "password"}
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  placeholder="fc_cli_••••••••••••••••••••••••"
                  className="w-full pl-3 pr-10 py-2 text-xs font-mono rounded-lg border border-zinc-200 bg-white outline-none focus:border-[#5100cd]"
                />
                <button
                  type="button"
                  onClick={() => setShowApiKey(!showApiKey)}
                  className="absolute right-2.5 top-2 text-zinc-400 hover:text-zinc-600 cursor-pointer"
                >
                  {showApiKey ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                </button>
              </div>
            </div>

            {/* Supported Models List */}
            <div className="space-y-2">
              <h4 className="text-xs font-semibold text-zinc-900 uppercase tracking-wider text-zinc-500">
                Available Fusion Models ({models.length})
              </h4>
              <div className="rounded-xl border border-zinc-200 overflow-hidden divide-y divide-zinc-100 bg-white">
                {models.map((m) => (
                  <div key={m.id} className="p-3 flex items-center justify-between hover:bg-zinc-50/60 transition-colors">
                    <div className="min-w-0 pr-3">
                      <div className="flex items-center gap-2">
                        <span className="text-xs font-semibold text-zinc-900">{m.name}</span>
                        <span className="text-[10px] font-mono px-1.5 py-0.2 rounded bg-purple-50 text-[#5100cd]">
                          {m.badge}
                        </span>
                        {m.id === defaultModel && (
                          <span className="text-[10px] font-medium text-emerald-600 bg-emerald-50 px-2 py-0.2 rounded-full">
                            Default
                          </span>
                        )}
                      </div>
                      <p className="text-[11px] text-zinc-500 mt-0.5 truncate">{m.description}</p>
                    </div>
                    <button
                      type="button"
                      onClick={() => setDefaultModel(m.id)}
                      className="px-2.5 py-1 text-xs rounded border border-zinc-200 hover:border-zinc-300 text-zinc-700 cursor-pointer shrink-0"
                    >
                      {m.id === defaultModel ? "Active" : "Set Default"}
                    </button>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}

        {/* Notifications Tab */}
        {tab === "notifications" && (
          <div className="space-y-4 max-w-2xl">
            <div className="rounded-xl border border-zinc-200 bg-zinc-50/40 overflow-hidden">
              <div className="grid grid-cols-[1fr_4rem_4rem] items-center gap-2 px-4 py-2 border-b border-zinc-200/60 bg-zinc-100/60 text-[11px] font-medium uppercase tracking-wider text-zinc-500">
                <span>Event</span>
                <span className="text-center">Notify</span>
                <span className="text-center">Sound</span>
              </div>

              {(
                [
                  { id: "taskCompletion", label: "Task completed", desc: "When Fusion finishes an agent run or turn." },
                  { id: "approvalNeeded", label: "Approval needed", desc: "When a tool or command is waiting for your approval." },
                  { id: "questionAsked", label: "Question asked", desc: "When Fusion needs clarification before continuing." },
                  { id: "sessionError", label: "Session error", desc: "When an agent turn stops because of an error." },
                ] as const
              ).map((ev) => {
                const pref = notifSettings[ev.id];
                return (
                  <div
                    key={ev.id}
                    className="grid grid-cols-[1fr_4rem_4rem] items-center gap-2 px-4 py-3 border-b border-zinc-200/40 last:border-b-0 hover:bg-zinc-50/60 transition-colors"
                  >
                    <div className="min-w-0 pr-2">
                      <p className="text-xs font-semibold text-zinc-800">{ev.label}</p>
                      <p className="text-[11px] text-zinc-500 mt-0.5">{ev.desc}</p>
                    </div>

                    {/* Notify Toggle */}
                    <div className="flex justify-center">
                      <button
                        type="button"
                        onClick={() => updateNotif(ev.id, "enabled", !pref.enabled)}
                        className={`w-8 h-4.5 rounded-full transition-colors relative cursor-pointer ${
                          pref.enabled ? "bg-[#5100cd]" : "bg-zinc-300"
                        }`}
                      >
                        <span
                          className={`absolute top-0.5 left-0.5 w-3.5 h-3.5 rounded-full bg-white transition-transform ${
                            pref.enabled ? "translate-x-3.5" : "translate-x-0"
                          }`}
                        />
                      </button>
                    </div>

                    {/* Sound Toggle */}
                    <div className="flex justify-center">
                      <button
                        type="button"
                        disabled={!pref.enabled}
                        onClick={() => updateNotif(ev.id, "sound", !pref.sound)}
                        className={`w-8 h-4.5 rounded-full transition-colors relative ${
                          !pref.enabled
                            ? "opacity-40 cursor-not-allowed bg-zinc-200"
                            : pref.sound
                            ? "bg-[#5100cd] cursor-pointer"
                            : "bg-zinc-300 cursor-pointer"
                        }`}
                      >
                        <span
                          className={`absolute top-0.5 left-0.5 w-3.5 h-3.5 rounded-full bg-white transition-transform ${
                            pref.sound ? "translate-x-3.5" : "translate-x-0"
                          }`}
                        />
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* Account Tab */}
        {tab === "account" && (
          <div className="space-y-6 max-w-2xl">
            <div className="p-5 rounded-xl border border-zinc-200 bg-zinc-50/40 flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="w-10 h-10 rounded-full bg-purple-100 text-[#5100cd] flex items-center justify-center font-bold text-sm">
                  <User className="w-5 h-5" />
                </div>
                <div>
                  <h3 className="text-sm font-semibold text-zinc-900">
                    {authStatus.email || "Fusion Authenticated User"}
                  </h3>
                  <div className="flex items-center gap-2 mt-0.5">
                    <span className="text-[11px] font-medium px-2 py-0.5 rounded bg-purple-100 text-[#5100cd]">
                      Subscription Mode
                    </span>
                    <span className="text-xs text-zinc-500">Device linked via ~/.fusion</span>
                  </div>
                </div>
              </div>

              {onSignOut && (
                <button
                  type="button"
                  data-testid="settings-signout-btn"
                  onClick={onSignOut}
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-red-200 bg-white hover:bg-red-50 text-xs font-medium text-red-600 transition-colors cursor-pointer"
                >
                  <LogOut className="w-3.5 h-3.5" />
                  <span>Sign out</span>
                </button>
              )}
            </div>

            <div className="p-4 rounded-xl border border-zinc-200 bg-white space-y-2 text-xs text-zinc-600 leading-relaxed">
              <p className="font-semibold text-zinc-900">Wholesale API Rate Benefits:</p>
              <ul className="list-disc pl-4 space-y-1 text-zinc-500">
                <li>DeepSeek V4 Flash & GLM 5.3 Flash at $0.01 per 1,000,000 tokens.</li>
                <li>Zero 5-hour session lockouts during long multi-turn engineering tasks.</li>
                <li>Credits and local configurations never expire.</li>
              </ul>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

export default SettingsView;
