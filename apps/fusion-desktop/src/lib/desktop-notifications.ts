import { isTauriEnvironment, showDesktopNotification, playSystemSound } from "./fusion-ipc";

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

export interface PushToastPayload {
  id: string;
  type: DesktopNotificationEventType;
  title: string;
  body: string;
  timestamp: number;
}

type PushToastListener = (toast: PushToastPayload) => void;

const toastListeners = new Set<PushToastListener>();

export function subscribeToPushToasts(listener: PushToastListener): () => void {
  toastListeners.add(listener);
  return () => toastListeners.delete(listener);
}

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
 * Plays a clean, high-fidelity notification chime using WebAudio synthesis.
 * Guaranteed to play through Mac speakers/headphones immediately.
 */
export function playNotificationSound(): void {
  if (typeof window === "undefined") return;
  try {
    const AudioCtx =
      window.AudioContext ||
      (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    const ctx = new AudioCtx();
    if (ctx.state === "suspended") {
      ctx.resume().catch(() => {});
    }
    const now = ctx.currentTime;

    const osc1 = ctx.createOscillator();
    const osc2 = ctx.createOscillator();
    const gain = ctx.createGain();

    osc1.type = "sine";
    osc2.type = "sine";

    // Two-tone chime (F#5: 739.99 Hz -> B5: 987.77 Hz)
    osc1.frequency.setValueAtTime(739.99, now);
    osc2.frequency.setValueAtTime(987.77, now + 0.08);

    gain.gain.setValueAtTime(0.2, now);
    gain.gain.exponentialRampToValueAtTime(0.001, now + 0.4);

    osc1.connect(gain);
    osc2.connect(gain);
    gain.connect(ctx.destination);

    osc1.start(now);
    osc1.stop(now + 0.15);
    osc2.start(now + 0.08);
    osc2.stop(now + 0.4);
  } catch (err) {
    console.warn("[desktop-notifications] playNotificationSound error:", err);
  }
}

/**
 * Sends push notifications across 3 layers:
 * 1. Audio chime through speakers
 * 2. In-app floating push banner (always visible immediately)
 * 3. Native macOS / Windows Notification Center
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

  const shouldPlaySound = options.sound ?? pref.sound;
  // 1. Play real native audio sound (macOS afplay Ping.aiff + WebAudio)
  if (shouldPlaySound) {
    playSystemSound("Ping").catch(() => {});
    playNotificationSound();
  }

  // 2. Broadcast in-app floating push toast
  const toastPayload: PushToastPayload = {
    id: `notif-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
    type: eventType,
    title: options.title,
    body: options.body,
    timestamp: Date.now(),
  };

  for (const listener of toastListeners) {
    try {
      listener(toastPayload);
    } catch {}
  }

  // 3. System / OS Notification via Tauri
  if (isTauriEnvironment()) {
    await showDesktopNotification(options.title, options.body, shouldPlaySound);
    return;
  }

  // 4. Web Notification API fallback
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
