import { isTauriEnvironment, showDesktopNotification } from "./fusion-ipc";

export type DesktopNotificationEventType =
  | "taskCompletion"
  | "approvalNeeded"
  | "questionAsked"
  | "sessionError";

export interface DesktopNotificationPreference {
  enabled: boolean;
  sound: boolean;
}

export type DesktopNotificationSettings = Record<
  DesktopNotificationEventType,
  DesktopNotificationPreference
>;

export const DEFAULT_NOTIFICATION_SETTINGS: DesktopNotificationSettings = {
  taskCompletion: { enabled: true, sound: true },
  approvalNeeded: { enabled: true, sound: true },
  questionAsked: { enabled: true, sound: false },
  sessionError: { enabled: true, sound: true },
};

export const NOTIFICATION_STORAGE_KEY = "fusion_desktop_notification_settings_v1";

export function readDesktopNotificationSettings(): DesktopNotificationSettings {
  if (typeof window === "undefined" || !window.localStorage) {
    return { ...DEFAULT_NOTIFICATION_SETTINGS };
  }
  try {
    const raw = window.localStorage.getItem(NOTIFICATION_STORAGE_KEY);
    if (raw) {
      return { ...DEFAULT_NOTIFICATION_SETTINGS, ...JSON.parse(raw) };
    }
  } catch {
    // Fallback
  }
  return { ...DEFAULT_NOTIFICATION_SETTINGS };
}

export function writeDesktopNotificationSettings(
  settings: DesktopNotificationSettings
): DesktopNotificationSettings {
  if (typeof window !== "undefined" && window.localStorage) {
    window.localStorage.setItem(NOTIFICATION_STORAGE_KEY, JSON.stringify(settings));
  }
  return settings;
}

/**
 * Sends a native macOS / Windows / Linux desktop notification
 * only when the window is in the background or unfocused, matching Cline's behavior.
 */
export async function notifyDesktopEvent(
  eventType: DesktopNotificationEventType,
  options: {
    title: string;
    body: string;
    sound?: boolean;
    force?: boolean;
  }
): Promise<void> {
  const settings = readDesktopNotificationSettings();
  const pref = settings[eventType];

  if (!pref || !pref.enabled) {
    return;
  }

  // Cline rule: "Notify only while the window is in the background."
  const isBackground =
    typeof document !== "undefined" &&
    (document.hidden || !document.hasFocus());

  if (!isBackground && !options.force) {
    return;
  }

  const shouldPlaySound = options.sound ?? pref.sound;
  if (isTauriEnvironment()) {
    await showDesktopNotification(options.title, options.body, shouldPlaySound);
    return;
  }

  // Web Notification fallback
  if (typeof window !== "undefined" && "Notification" in window) {
    if (Notification.permission === "granted") {
      try {
        new Notification(options.title, {
          body: options.body,
          icon: "/favicon.ico",
        });
      } catch {}
    } else if (Notification.permission !== "denied") {
      try {
        const perm = await Notification.requestPermission();
        if (perm === "granted") {
          new Notification(options.title, {
            body: options.body,
            icon: "/favicon.ico",
          });
        }
      } catch {}
    }
  }
}
