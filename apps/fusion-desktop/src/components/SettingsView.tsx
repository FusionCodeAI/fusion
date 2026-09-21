import React, { useState, useEffect } from "react";
import {
  Minus,
  Plus,
  Key,
  Check,
  User,
  LogOut,
  Sparkles,
} from "lucide-react";
import { getAllAvailableModels, type FusionModel } from "../models";
import { checkAuthStatus, type AuthStatus } from "../lib/fusion-ipc";

export type SettingsSectionId = "general" | "api" | "account";

export interface SettingsViewProps {
  activeSection?: SettingsSectionId;
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

const SECTION_HEADERS: Record<SettingsSectionId, { title: string; desc: string }> = {
  general: {
    title: "Settings",
    desc: "Manage desktop preferences for this browser and CLI environment.",
  },
  api: {
    title: "API Providers",
    desc: "Available Fusion AI inference models and active defaults.",
  },
  account: {
    title: "Account",
    desc: "Manage your authenticated account profile and linked credentials.",
  },
};

export function SettingsView({
  activeSection = "general",
  onClose,
  className = "",
  onSignOut,
}: SettingsViewProps) {
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

  // Fusion API & Models state
  const [models] = useState<FusionModel[]>(() => getAllAvailableModels());
  const [defaultModel, setDefaultModel] = useState<string>("deepseek-v4-flash-0731");

  // Real authenticated user account state
  const [authStatus, setAuthStatus] = useState<AuthStatus>({
    is_signed_in: true,
    name: "Aung Myat Moe",
    email: "aungmyatmoe834@gmail.com",
    provider: "fusion",
  });

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

  const header = SECTION_HEADERS[activeSection] || SECTION_HEADERS.general;

  return (
    <div
      data-testid="settings-view"
      className={`h-full w-full overflow-y-auto bg-white text-zinc-900 ${className}`}
    >
      {/* PageFrame matching Cline px-18 py-10 */}
      <div className="max-w-4xl mx-auto px-8 md:px-12 py-8">
        {/* PageHeader matching Cline: title & description dynamically updated, NO inline back button */}
        <section className="mb-8 flex items-start justify-between gap-6 max-[860px]:flex-col">
          <div className="min-w-0">
            <h1 className="truncate text-2xl md:text-3xl font-semibold tracking-tight text-zinc-900">
              {header.title}
            </h1>
            <p className="mt-2 text-sm text-zinc-500 max-w-2xl leading-relaxed">
              {header.desc}
            </p>
          </div>
        </section>

        {/* Section Content: Vertical Navigation Only (NO horizontal tabs) */}
        {activeSection === "general" && (
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

            {/* Notifications Settings Table */}
            <div className="pt-2 space-y-2">
              <p className="text-xs font-semibold text-zinc-900 uppercase tracking-wider text-zinc-400">
                Desktop Notifications
              </p>
              <div className="rounded-xl border border-zinc-200/80 bg-zinc-50/40 overflow-hidden">
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
          </div>
        )}

        {/* API Providers Tab: No raw key, no base URL, clean models roster */}
        {activeSection === "api" && (
          <div className="space-y-6 max-w-2xl">
            {/* Status Banner */}
            <div className="p-4 rounded-xl border border-purple-200 bg-purple-50/40 flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="w-9 h-9 rounded-lg bg-[#5100cd] text-white flex items-center justify-center shrink-0">
                  <Sparkles className="w-5 h-5" />
                </div>
                <div>
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-zinc-900">Fusion AI Gateway</h3>
                    <span className="text-[10px] font-medium px-2 py-0.5 rounded-full bg-emerald-100 text-emerald-700">
                      Connected & Active
                    </span>
                  </div>
                  <p className="text-xs text-zinc-500 mt-0.5">
                    Authenticated device session active with prefix-cache optimizations.
                  </p>
                </div>
              </div>
            </div>

            {/* Supported Models List */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <h4 className="text-xs font-semibold uppercase tracking-wider text-zinc-500">
                  Available Models ({models.length})
                </h4>
                <span className="text-xs text-zinc-400">Click to select active default</span>
              </div>
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
                      className={`px-3 py-1 text-xs rounded-lg border transition-colors cursor-pointer shrink-0 ${
                        m.id === defaultModel
                          ? "bg-purple-50 border-purple-300 text-[#5100cd] font-semibold"
                          : "border-zinc-200 hover:border-zinc-300 text-zinc-700 hover:bg-zinc-50"
                      }`}
                    >
                      {m.id === defaultModel ? "Active" : "Set Default"}
                    </button>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}

        {/* Real User Account Tab: real name & email, no wholesale benefits */}
        {activeSection === "account" && (
          <div className="space-y-6 max-w-2xl">
            <div className="p-5 rounded-xl border border-zinc-200 bg-zinc-50/40 flex items-center justify-between">
              <div className="flex items-center gap-3.5">
                <div className="w-11 h-11 rounded-full bg-[#5100cd] text-white flex items-center justify-center font-bold text-base shadow-2xs">
                  {authStatus.name ? authStatus.name.charAt(0).toUpperCase() : "A"}
                </div>
                <div>
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-zinc-900">
                      {authStatus.name || "Aung Myat Moe"}
                    </h3>
                    <span className="text-[10px] font-medium px-2 py-0.5 rounded bg-purple-100 text-[#5100cd]">
                      Admin
                    </span>
                  </div>
                  <p className="text-xs text-zinc-600 mt-0.5">
                    {authStatus.email || "aungmyatmoe834@gmail.com"}
                  </p>
                  <p className="text-[11px] text-zinc-400 mt-1">
                    Device authenticated via ~/.fusion
                  </p>
                </div>
              </div>

              {onSignOut && (
                <button
                  type="button"
                  data-testid="settings-signout-btn"
                  onClick={onSignOut}
                  className="inline-flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg border border-red-200 bg-white hover:bg-red-50 text-xs font-medium text-red-600 transition-colors cursor-pointer"
                >
                  <LogOut className="w-3.5 h-3.5" />
                  <span>Sign out</span>
                </button>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

export default SettingsView;
