import React, { useState, useEffect } from "react";
import { X, Bell, Volume2 } from "lucide-react";

export interface NotificationModalProps {
  isOpen: boolean;
  onClose: () => void;
}

export type NotificationEventType =
  | "taskCompletion"
  | "approvalNeeded"
  | "questionAsked"
  | "sessionError";

export interface NotificationPreference {
  enabled: boolean;
  sound: boolean;
}

export type NotificationSettings = Record<NotificationEventType, NotificationPreference>;

const DEFAULT_SETTINGS: NotificationSettings = {
  taskCompletion: { enabled: true, sound: true },
  approvalNeeded: { enabled: true, sound: true },
  questionAsked: { enabled: true, sound: false },
  sessionError: { enabled: true, sound: true },
};

const EVENT_COPY: Record<NotificationEventType, { label: string; description: string }> = {
  taskCompletion: {
    label: "Task completed",
    description: "When Fusion finishes an agent run or turn.",
  },
  approvalNeeded: {
    label: "Approval needed",
    description: "When a tool or command is waiting for your approval.",
  },
  questionAsked: {
    label: "Question asked",
    description: "When Fusion needs clarification before continuing.",
  },
  sessionError: {
    label: "Session error",
    description: "When an agent turn stops because of an error.",
  },
};

const NOTIF_STORAGE_KEY = "fusion_desktop_notification_settings_v1";

export function NotificationModal({ isOpen, onClose }: NotificationModalProps) {
  const [settings, setSettings] = useState<NotificationSettings>(() => {
    if (typeof window !== "undefined" && window.localStorage) {
      try {
        const raw = window.localStorage.getItem(NOTIF_STORAGE_KEY);
        if (raw) return { ...DEFAULT_SETTINGS, ...JSON.parse(raw) };
      } catch {
        // Fallback to default
      }
    }
    return DEFAULT_SETTINGS;
  });

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && isOpen) {
        onClose();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  const updatePreference = (eventType: NotificationEventType, field: "enabled" | "sound", value: boolean) => {
    setSettings((prev) => {
      const next = {
        ...prev,
        [eventType]: {
          ...prev[eventType],
          [field]: value,
        },
      };
      if (typeof window !== "undefined" && window.localStorage) {
        window.localStorage.setItem(NOTIF_STORAGE_KEY, JSON.stringify(next));
      }
      return next;
    });
  };

  return (
    <div
      data-testid="notification-modal-overlay"
      className="fixed inset-0 z-50 bg-black/40 backdrop-blur-xs flex items-center justify-center p-4 animate-in fade-in duration-150"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        data-testid="notification-modal"
        className="w-full max-w-lg bg-white rounded-2xl border border-zinc-200 shadow-2xl overflow-hidden flex flex-col animate-in zoom-in-95 duration-150"
      >
        {/* Header */}
        <div className="px-5 py-4 border-b border-zinc-100 flex items-center justify-between select-none">
          <div className="flex items-center gap-2">
            <div className="w-7 h-7 rounded-lg bg-purple-50 text-[#5100cd] flex items-center justify-center">
              <Bell className="w-3.5 h-3.5" />
            </div>
            <div>
              <h2 className="text-base font-semibold text-zinc-900">Desktop Notifications</h2>
              <p className="text-xs text-zinc-500">
                Notify only while the Fusion window is in the background.
              </p>
            </div>
          </div>
          <button
            type="button"
            data-testid="notification-close-btn"
            onClick={onClose}
            className="p-1 rounded-lg text-zinc-400 hover:text-zinc-700 hover:bg-zinc-100 transition-colors cursor-pointer"
            aria-label="Close"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Content Table matching Cline NotificationSettings */}
        <div className="p-5 select-none">
          <div className="rounded-xl border border-zinc-200/80 bg-zinc-50/40 overflow-hidden">
            <div className="grid grid-cols-[1fr_4rem_4rem] items-center gap-2 px-4 py-2 border-b border-zinc-200/60 bg-zinc-100/60 text-[11px] font-medium uppercase tracking-wider text-zinc-500">
              <span>Event</span>
              <span className="text-center">Notify</span>
              <span className="text-center">Sound</span>
            </div>

            {(["taskCompletion", "approvalNeeded", "questionAsked", "sessionError"] as const).map((eventType) => {
              const copy = EVENT_COPY[eventType];
              const pref = settings[eventType];

              return (
                <div
                  key={eventType}
                  className="grid grid-cols-[1fr_4rem_4rem] items-center gap-2 px-4 py-3 border-b border-zinc-200/40 last:border-b-0 hover:bg-zinc-50/60 transition-colors"
                >
                  <div className="min-w-0 pr-2">
                    <p className="text-xs font-semibold text-zinc-800">{copy.label}</p>
                    <p className="text-[11px] text-zinc-500 mt-0.5">{copy.description}</p>
                  </div>

                  {/* Notify Toggle */}
                  <div className="flex justify-center">
                    <button
                      type="button"
                      data-testid={`notif-toggle-${eventType}`}
                      onClick={() => updatePreference(eventType, "enabled", !pref.enabled)}
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
                      data-testid={`sound-toggle-${eventType}`}
                      onClick={() => updatePreference(eventType, "sound", !pref.sound)}
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

        {/* Footer */}
        <div className="px-5 py-3 border-t border-zinc-100 bg-zinc-50/50 flex items-center justify-between">
          <span className="text-[11px] text-zinc-400">Settings auto-saved to preferences.</span>
          <button
            type="button"
            onClick={onClose}
            className="px-3.5 py-1.5 text-xs font-medium bg-zinc-900 hover:bg-zinc-800 text-white rounded-lg transition-colors cursor-pointer"
          >
            Done
          </button>
        </div>
      </div>
    </div>
  );
}

export default NotificationModal;
