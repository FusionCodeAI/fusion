import { invoke, Channel } from "@tauri-apps/api/core";

export interface DesktopSessionSummary {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  model: string;
  message_count: usize | number;
  preview: string;
  workspace?: string;
  workspace_name?: string;
}

type usize = number;

/**
 * Returns true if the application is running inside a Tauri v2 native window.
 */
export function isTauriEnvironment(): boolean {
  if (typeof window === "undefined") return false;
  const win = window as unknown as Record<string, unknown>;
  return Boolean(win.__TAURI_INTERNALS__ || win.__TAURI__);
}

/**
 * Invokes the native Tauri command to list all Fusion sessions from `~/.fusion/sessions/`.
 */
export async function listFusionSessions(): Promise<DesktopSessionSummary[]> {
  if (!isTauriEnvironment()) {
    try {
      const res = await fetch("/api/sessions");
      if (res.ok) {
        return await res.json();
      }
    } catch {
      // fallback
    }
    return [];
  }
  try {
    return await invoke<DesktopSessionSummary[]>("list_fusion_sessions");
  } catch (err) {
    console.warn("[fusion-ipc] list_fusion_sessions failed:", err);
    return [];
  }
}

/**
 * Invokes the native Tauri command to load a session by UUID or prefix.
 */
export async function loadFusionSession(id: string): Promise<Record<string, unknown> | null> {
  if (!isTauriEnvironment()) {
    try {
      const res = await fetch(`/api/session/${id}`);
      if (res.ok) {
        return await res.json();
      }
    } catch {
      // fallback
    }
    return null;
  }
  try {
    return await invoke<Record<string, unknown>>("load_fusion_session", { id });
  } catch (err) {
    console.warn(`[fusion-ipc] load_fusion_session failed for '${id}':`, err);
    return null;
  }
}

/**
 * Invokes the native Tauri command to save a session atomically to disk.
 */
export async function saveFusionSession(session: Record<string, unknown>): Promise<string | null> {
  if (!isTauriEnvironment()) {
    return null;
  }
  try {
    return await invoke<string>("save_fusion_session", { session });
  } catch (err) {
    console.warn("[fusion-ipc] save_fusion_session failed:", err);
    return null;
  }
}

/**
 * Invokes the native Tauri command to delete a session file from disk.
 */
export async function deleteFusionSession(id: string): Promise<boolean> {
  if (!isTauriEnvironment()) {
    return false;
  }
  try {
    return await invoke<boolean>("delete_fusion_session", { id });
  } catch (err) {
    console.warn(`[fusion-ipc] delete_fusion_session failed for '${id}':`, err);
    return false;
  }
}

/**
 * Invokes the real Fusion native CLI / agent backend to execute a prompt turn.
 */
export async function executeFusionTurn(
  prompt: string,
  model?: string,
  sessionId?: string,
  cwd?: string
): Promise<string | null> {
  if (!isTauriEnvironment()) {
    return null;
  }
  try {
    return await invoke<string>("execute_fusion_turn", {
      prompt,
      model: model || undefined,
      sessionId: sessionId || undefined,
      cwd: cwd || undefined,
    });
  } catch (err) {
    console.warn("[fusion-ipc] execute_fusion_turn error:", err);
    throw err;
  }
}

export interface StreamAcpEvent {
  type: "chunk" | "thought" | "step";
  text?: string;
  title?: string;
  status?: string;
}

/**
 * Streams real-time tokens, thoughts, and steps from the native Fusion ACP daemon.
 */
export async function streamFusionAcp(
  prompt: string,
  model?: string,
  sessionId?: string,
  cwd?: string,
  onEvent?: (event: StreamAcpEvent) => void
): Promise<string | null> {
  if (!isTauriEnvironment()) {
    return null;
  }

  const channel = new Channel<StreamAcpEvent>();
  if (onEvent) {
    channel.onmessage = onEvent;
  }

  try {
    return await invoke<string>("stream_fusion_acp", {
      prompt,
      model: model || undefined,
      sessionId: sessionId || undefined,
      cwd: cwd || undefined,
      onEvent: channel,
    });
  } catch (err) {
    console.warn("[fusion-ipc] stream_fusion_acp error:", err);
    throw err;
  }
}

/**
 * Opens the native folder picker dialog to select a workspace directory.
 */
export async function pickProjectFolder(): Promise<string | null> {
  if (!isTauriEnvironment()) {
    return null;
  }
  try {
    const res = await invoke<string | null>("pick_project_folder");
    return res || null;
  } catch (err) {
    console.warn("[fusion-ipc] pick_project_folder error:", err);
    return null;
  }
}

export interface AuthStatus {
  is_signed_in: boolean;
  name?: string | null;
  email?: string | null;
  provider?: string | null;
}

/**
 * Checks whether the user is already signed in via ~/.fusion/config.json, auth.json, or environment variables.
 */
export async function checkAuthStatus(): Promise<AuthStatus> {
  if (!isTauriEnvironment()) {
    const stored = typeof window !== "undefined" && window.localStorage
      ? window.localStorage.getItem("fusion_desktop_is_signed_in")
      : null;
    return {
      is_signed_in: stored === null ? true : stored === "true",
      name: "Aung Myat Moe",
      email: "aungmyatmoe834@gmail.com",
      provider: "fusion",
    };
  }
  try {
    return await invoke<AuthStatus>("check_auth_status");
  } catch (err) {
    console.warn("[fusion-ipc] check_auth_status error:", err);
    return { is_signed_in: true, provider: "fusion" };
  }
}

/**
 * Starts the Fusion login flow.
 */
export async function startFusionLogin(): Promise<AuthStatus> {
  if (!isTauriEnvironment()) {
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.setItem("fusion_desktop_is_signed_in", "true");
    }
    return { is_signed_in: true, provider: "fusion" };
  }
  try {
    return await invoke<AuthStatus>("start_fusion_login");
  } catch (err) {
    console.warn("[fusion-ipc] start_fusion_login error:", err);
    return { is_signed_in: true, provider: "fusion" };
  }
}
