import React, { useState, useEffect, useRef } from "react";
import { TopHeader } from "./components/TopHeader";
import { Sidebar, getWorkspaceFolderName, type SidebarSessionItem } from "./components/Sidebar";
import { ClineHeroView } from "./components/ClineHeroView";
import { ThinkingRow } from "./components/ThinkingRow";
import { UserMessage } from "./components/UserMessage";
import { ToolCallRow } from "./components/ToolCallRow";
import { DiffView } from "./components/DiffView";
import { Composer } from "./components/Composer";
import { ClineAvatar } from "./components/ClineAvatar";
import { MarkdownRenderer } from "./components/MarkdownRenderer";
import { CustomizeView } from "./components/CustomizeView";
import { SettingsView } from "./components/SettingsView";
import { SessionCommandBar } from "./components/SessionCommandBar";
import { PushToast, type PushToastMessage } from "./components/PushToast";
import { AgentBridge } from "./lib/agent-bridge";
import { notifyDesktopEvent, subscribeToPushToasts } from "./lib/desktop-notifications";
import { pickProjectFolder, checkAuthStatus, startFusionLogin } from "./lib/fusion-ipc";
import {
  loadAllSessions,
  generateSessionId,
  saveSession,
  saveAllSessions,
  deleteSessionFromStorage,
  getStoredActiveSessionId,
  setStoredActiveSessionId,
  syncNativeFusionSessions,
  loadFullNativeSessionMessages,
  type ChatSessionRecord,
} from "./state/session-storage";
import { DEFAULT_FUSION_MODEL } from "./models";
import type { ChatMessage, TurnStep } from "./types";


export interface BridgeCallbacks {
  onThought: (thought: string) => void;
  onChunk: (chunk: string) => void;
  onStep: (step: TurnStep) => void;
  onDiff: (diff: string) => void;
  onDone: () => void;
  onError: (err: Error) => void;
}

/**
 * Wires AgentBridge event subscriptions to UI update callbacks.
 * Subscribes to: thought, chunk, step, diff, done, error.
 */
export function setupBridgeListeners(bridge: AgentBridge, callbacks: BridgeCallbacks): () => void {
  const unsubThought = bridge.on("thought", callbacks.onThought);
  const unsubChunk = bridge.on("chunk", callbacks.onChunk);
  const unsubStep = bridge.on("step", callbacks.onStep);
  const unsubDiff = bridge.on("diff", callbacks.onDiff);
  const unsubDone = bridge.on("done", callbacks.onDone);
  const unsubError = bridge.on("error", callbacks.onError);

  return () => {
    unsubThought();
    unsubChunk();
    unsubStep();
    unsubDiff();
    unsubDone();
    unsubError();
  };
}

/**
 * Returns true if the keyboard event matches the Cmd+B (Mac) or Ctrl+B (Win/Linux) shortcut.
 */
export function isSidebarToggleKeyCombo(e: { key: string; metaKey?: boolean; ctrlKey?: boolean }): boolean {
  return Boolean((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "b");
}
export interface AppProps {
  bridge?: AgentBridge;
  initialMessages?: ChatMessage[];
  initialSidebarOpen?: boolean;
  initialModel?: string;
  initialSessions?: SidebarSessionItem[];
  initialIsSignedIn?: boolean;
}

export function App({
  bridge: propBridge,
  initialMessages = [],
  initialSidebarOpen = true,
  initialModel = DEFAULT_FUSION_MODEL.id,
  initialSessions,
  initialIsSignedIn,
}: AppProps) {
  const [isSidebarOpen, setIsSidebarOpen] = useState(initialSidebarOpen);
  const [currentView, setCurrentView] = useState<"chat" | "customize" | "settings">("chat");
  const [toasts, setToasts] = useState<PushToastMessage[]>([]);

  useEffect(() => {
    return subscribeToPushToasts((newToast) => {
      setToasts((prev) => [newToast, ...prev.slice(0, 4)]);
    });
  }, []);

  const handleDismissToast = (id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  };

  const [settingsSection, setSettingsSection] = useState<"general" | "api" | "account">("general");
  const [isCommandBarOpen, setIsCommandBarOpen] = useState(false);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && (e.key.toLowerCase() === "k" || e.key.toLowerCase() === "p")) {
        e.preventDefault();
        setIsCommandBarOpen((prev) => !prev);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const [isSignedIn, setIsSignedIn] = useState<boolean>(() => {
    if (typeof initialIsSignedIn === "boolean") return initialIsSignedIn;
    if (typeof window !== "undefined" && window.localStorage) {
      const stored = window.localStorage.getItem("fusion_desktop_is_signed_in");
      if (stored !== null) return stored === "true";
    }
    return true;
  });

  useEffect(() => {
    if (typeof initialIsSignedIn === "boolean") return;
    checkAuthStatus().then((status) => {
      setIsSignedIn(status.is_signed_in);
    });
  }, [initialIsSignedIn]);

  const handleSignIn = async () => {
    const res = await startFusionLogin();
    setIsSignedIn(res.is_signed_in);
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.setItem("fusion_desktop_is_signed_in", String(res.is_signed_in));
    }
  };


  const [sidebarWidth, setSidebarWidth] = useState<number>(() => {
    if (typeof window !== "undefined" && window.localStorage) {
      const stored = window.localStorage.getItem("fusion_desktop_sidebar_width");
      if (stored) {
        const parsed = parseInt(stored, 10);
        if (!isNaN(parsed) && parsed >= 180 && parsed <= 520) return parsed;
      }
    }
    return 260;
  });

  const handleSidebarResize = (newWidth: number) => {
    setSidebarWidth(newWidth);
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.setItem("fusion_desktop_sidebar_width", String(newWidth));
    }
  };

  const handleSidebarResetWidth = () => {
    setSidebarWidth(260);
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.removeItem("fusion_desktop_sidebar_width");
    }
  };
  const [workspaceDir, setWorkspaceDir] = useState<string>(() => {
    if (typeof window !== "undefined" && window.localStorage) {
      return window.localStorage.getItem("fusion_desktop_workspace_dir") || ".";
    }
    return ".";
  });

  const workspaceName = getWorkspaceFolderName(workspaceDir);

  const handlePickProjectFolder = async () => {
    const selected = await pickProjectFolder();
    if (selected) {
      setWorkspaceDir(selected);
      if (typeof window !== "undefined" && window.localStorage) {
        window.localStorage.setItem("fusion_desktop_workspace_dir", selected);
      }
      bridge.setCwd?.(selected);
    }
  };

  const [sessionsRecord, setSessionsRecord] = useState<ChatSessionRecord[]>(() => {
    if (initialSessions && initialSessions.length > 0) {
      return initialSessions.map((s) => ({
        id: s.id,
        title: s.title,
        createdAt: s.createdAt || Date.now(),
        updatedAt: s.updatedAt || Date.now(),
        model: initialModel,
        messages: s.id === "session-default" ? initialMessages : [],
      }));
    }
    return loadAllSessions();
  });

  const [activeSessionId, setActiveSessionId] = useState<string>(() => {
    if (initialSessions && initialSessions.length > 0) return initialSessions[0].id;
    const stored = getStoredActiveSessionId();
    const found = sessionsRecord.find((s) => s.id === stored);
    return found ? found.id : (sessionsRecord[0]?.id || "session-default");
  });

  const activeSession = sessionsRecord.find((s) => s.id === activeSessionId) || sessionsRecord[0];

  const [selectedModel, setSelectedModel] = useState<string>(() => {
    if (initialModel && initialModel !== DEFAULT_FUSION_MODEL.id) return initialModel;
    return activeSession?.model || initialModel;
  });
  const [isGenerating, setIsGenerating] = useState(false);
  const [sessionTitle, setSessionTitle] = useState<string>(activeSession?.title || "General chat conversation");
  const [messages, setMessages] = useState<ChatMessage[]>(() => {
    if (initialMessages.length > 0) return initialMessages;
    return activeSession?.messages || [];
  });
  if (typeof window !== "undefined") {
    (window as any).__FUSION_SESSIONS__ = sessionsRecord;
  }


  // Persist messages and state to session storage
  useEffect(() => {
    if (!activeSessionId) return;
    setSessionsRecord((prev) => {
      const idx = prev.findIndex((s) => s.id === activeSessionId);
      if (idx < 0) return prev;
      const updated = [...prev];
      updated[idx] = {
        ...updated[idx],
        title: sessionTitle,
        model: selectedModel,
        updatedAt: Date.now(),
        messages,
      };
      saveAllSessions(updated);
      return updated;
    });
  }, [messages, sessionTitle, selectedModel, activeSessionId]);

  // On startup: synchronize session list with native ~/.fusion/sessions/
  useEffect(() => {
    syncNativeFusionSessions(sessionsRecord).then((synced) => {
      if (synced && synced.length > 0) {
        setSessionsRecord(synced);
        const currentActive = synced.find((s) => s.id === activeSessionId);
        if (!currentActive || currentActive.id === "session-default") {
          const firstReal = synced.find((s) => s.id !== "session-default") || synced[0];
          if (firstReal) {
            setActiveSessionId(firstReal.id);
            setSessionTitle(firstReal.title);
            setSelectedModel(firstReal.model);
            if (firstReal.messages && firstReal.messages.length > 0) {
              setMessages(firstReal.messages);
            } else {
              loadFullNativeSessionMessages(firstReal.id).then((loadedMsgs) => {
                if (loadedMsgs && loadedMsgs.length > 0) setMessages(loadedMsgs);
              });
            }
          }
        }
      }
    });
  }, []);

  // Lazily initialize AgentBridge if not provided
  const bridgeRef = useRef<AgentBridge | null>(null);
  if (!bridgeRef.current) {
    bridgeRef.current = propBridge ?? new AgentBridge({ defaultModel: selectedModel });
  }
  const bridge = bridgeRef.current;
  const streamRef = useRef<HTMLDivElement>(null);
  // Keep active session in storage updated and notify bridge
  useEffect(() => {
    if (!activeSessionId) return;
    bridge.setSessionId?.(activeSessionId);
    setStoredActiveSessionId(activeSessionId);
  }, [activeSessionId, bridge]);
  // Keyboard shortcut: Cmd+B (Mac) or Ctrl+B (Windows/Linux) toggles sidebar
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (isSidebarToggleKeyCombo(e)) {
        e.preventDefault();
        setIsSidebarOpen((prev) => !prev);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  // Auto-scroll stream to bottom as content streams in
  useEffect(() => {
    if (streamRef.current) {
      streamRef.current.scrollTop = streamRef.current.scrollHeight;
    }
  }, [messages, isGenerating]);

  // Subscribe to AgentBridge events: thought, chunk, step, diff, done, error
  useEffect(() => {
    return setupBridgeListeners(bridge, {
      onThought: (thought) => {
        setMessages((prev) => {
          if (prev.length === 0) return prev;
          const last = prev[prev.length - 1];
          if (last.role !== "assistant") return prev;
          return [
            ...prev.slice(0, -1),
            { ...last, thought: (last.thought || "") + thought },
          ];
        });
      },
      onChunk: (chunk) => {
        setMessages((prev) => {
          if (prev.length === 0) return prev;
          const last = prev[prev.length - 1];
          if (last.role !== "assistant") return prev;
          return [
            ...prev.slice(0, -1),
            { ...last, content: (last.content || "") + chunk },
          ];
        });
      },
      onStep: (step) => {
        setMessages((prev) => {
          if (prev.length === 0) return prev;
          const last = prev[prev.length - 1];
          if (last.role !== "assistant") return prev;
          const existing = last.steps || [];
          const idx = existing.findIndex((s) => s.id === step.id);
          let updatedSteps: TurnStep[];
          if (idx >= 0) {
            updatedSteps = [...existing];
            updatedSteps[idx] = { ...updatedSteps[idx], ...step };
          } else {
            updatedSteps = [...existing, step];
          }
          return [
            ...prev.slice(0, -1),
            { ...last, steps: updatedSteps },
          ];
        });
      },
      onDiff: (diff) => {
        setMessages((prev) => {
          if (prev.length === 0) return prev;
          const last = prev[prev.length - 1];
          if (last.role !== "assistant") return prev;
          return [
            ...prev.slice(0, -1),
            { ...last, diffPatch: (last.diffPatch || "") + diff },
          ];
        });
      },
      onDone: () => {
        setIsGenerating(false);
      },
      onError: (err) => {
        setIsGenerating(false);
        setMessages((prev) => {
          if (prev.length === 0) return prev;
          const last = prev[prev.length - 1];
          if (last.role !== "assistant") return prev;
          if (!last.content) {
            return [
              ...prev.slice(0, -1),
              { ...last, content: `Error: ${err.message}` },
            ];
          }
          return prev;
        });
      },
    });
  }, [bridge]);

  const handleSend = async (text: string) => {
    const trimmed = text.trim();
    if (!trimmed || isGenerating) return;

    const userMessage: ChatMessage = {
      id: `user-${Date.now()}`,
      role: "user",
      content: trimmed,
      timestamp: Date.now(),
    };

    // Immediate Thinking... feedback: assistant message with empty content & thought
    const assistantMessage: ChatMessage = {
      id: `assistant-${Date.now()}`,
      role: "assistant",
      content: "",
      thought: "",
      steps: [],
      timestamp: Date.now(),
    };

    setMessages((prev) => [...prev, userMessage, assistantMessage]);
    setIsGenerating(true);

    // If starting a fresh chat, update active session title
    try {
      await bridge.prompt(trimmed, selectedModel);
      notifyDesktopEvent("taskCompletion", {
        title: "Task completed",
        body: `Finished turn in "${sessionTitle}"`,
        force: true,
      });
    } catch (err) {
      console.error("[App] prompt error:", err);
      notifyDesktopEvent("sessionError", {
        title: "Session error",
        body: err instanceof Error ? err.message : String(err),
        force: true,
      });
    } finally {
      setIsGenerating(false);
    }
  };
  const handleCancel = async () => {
    if (!isGenerating) return;
    try {
      await bridge.cancel();
    } catch (err) {
      console.error("[App] cancel error:", err);
    } finally {
      setIsGenerating(false);
    }
  };

  const handleNewChat = () => {
    const newSessionId = generateSessionId();
    const newSession: ChatSessionRecord = {
      id: newSessionId,
      title: "General chat conversation",
      createdAt: Date.now(),
      updatedAt: Date.now(),
      model: selectedModel,
      messages: [],
      workspace: workspaceDir,
      workspaceName: workspaceName,
    };
    saveSession(newSession);
    setSessionsRecord((prev) => [newSession, ...prev]);
    setActiveSessionId(newSessionId);
    setStoredActiveSessionId(newSessionId);
    setSessionTitle("General chat conversation");
    setMessages([]);
    setIsGenerating(false);
    setCurrentView("chat");
  };
  const handleSelectSession = async (id: string) => {
    if (id === activeSessionId) return;
    const target = sessionsRecord.find((s) => s.id === id) || loadAllSessions().find((s) => s.id === id);
    if (target) {
      setActiveSessionId(target.id);
      setStoredActiveSessionId(target.id);
      setSessionTitle(target.title);
      setSelectedModel(target.model || DEFAULT_FUSION_MODEL.id);

      if (target.messages && target.messages.length > 0) {
        setMessages(target.messages);
      } else {
        const nativeMessages = await loadFullNativeSessionMessages(target.id);
        if (nativeMessages && nativeMessages.length > 0) {
          setMessages(nativeMessages);
          setSessionsRecord((prev) =>
            prev.map((s) => (s.id === target.id ? { ...s, messages: nativeMessages } : s))
          );
        } else {
          setMessages([]);
        }
      }
      setIsGenerating(false);
      setCurrentView("chat");
    }
  };
  const handleDeleteSession = (id: string) => {
    const updated = deleteSessionFromStorage(id);
    setSessionsRecord(updated);
    if (activeSessionId === id) {
      const nextSession = updated[0];
      if (nextSession) {
        setActiveSessionId(nextSession.id);
        setStoredActiveSessionId(nextSession.id);
        setSessionTitle(nextSession.title);
        setSelectedModel(nextSession.model || DEFAULT_FUSION_MODEL.id);
        setMessages(nextSession.messages || []);
      }
    }
  };

  const handleModelChange = (modelId: string) => {
    setSelectedModel(modelId);
  };

  return (
    <div
      data-testid="app-shell"
      className="h-screen w-screen flex flex-row overflow-hidden bg-white"
    >
      {/* Sidebar: resizable, collapsed when isSidebarOpen is false */}
      {isSidebarOpen && (
        <Sidebar
          sessions={sessionsRecord}
          activeSessionId={activeSessionId}
          onDeleteSession={handleDeleteSession}
          workspaceDir={workspaceDir}
          width={sidebarWidth}
          currentView={currentView}
          canNavigateBack={currentView !== "chat"}
          onHistoryBack={() => setCurrentView("chat")}
          onResize={handleSidebarResize}
          onNewChat={handleNewChat}
          onSelectSession={handleSelectSession}
          onOpenSearch={() => setIsCommandBarOpen(true)}
          onCustomize={() => setCurrentView("customize")}
          settingsSection={settingsSection}
          onOpenSettings={(sec) => {
            if (sec) setSettingsSection(sec);
            setCurrentView("settings");
          }}
          onToggleSidebar={() => setIsSidebarOpen(false)}
        />
      )}
      {/* Main Content Area */}
      <main className="flex-1 min-h-0 flex flex-col overflow-hidden">
        {currentView === "customize" ? (
          <CustomizeView
            onClose={() => setCurrentView("chat")}
            onOpenMarketplace={() => {}}
          />
        ) : currentView === "settings" ? (
          <SettingsView
            activeSection={settingsSection}
            onClose={() => setCurrentView("chat")}
            onSignOut={() => {
              setIsSignedIn(false);
              if (typeof window !== "undefined" && window.localStorage) {
                window.localStorage.setItem("fusion_desktop_is_signed_in", "false");
              }
              setCurrentView("chat");
            }}
          />
        ) : (
          <>
            {/* TopHeader: 40px height, aligned with macOS traffic lights and sidebar */}
            <TopHeader
              title={sessionTitle}
              onMore={() => setCurrentView("settings")}
              onToggleSidebar={() => setIsSidebarOpen((prev) => !prev)}
            />
        {/* Stream: strictly centered, vertical scroll */}
        <div
          ref={streamRef}
          data-testid="conversation-stream"
          className="flex-1 min-h-0 overflow-y-auto px-8 py-4 flex flex-col items-center"
        >
          {messages.length === 0 ? (
            <ClineHeroView
              isSignedIn={isSignedIn}
              onSignIn={handleSignIn}
              onSend={handleSend}
              selectedModel={selectedModel}
              onSelectModel={handleModelChange}
              workspaceName={workspaceName}
              onPickWorkspaceFolder={handlePickProjectFolder}
            />
          ) : (
            <div className="w-full max-w-[680px] flex flex-col gap-4">
              {messages.map((msg, index) => {
                if (msg.role === "user") {
                  return <UserMessage key={msg.id} content={msg.content} />;
                }

                const isCurrent = isGenerating && index === messages.length - 1;
                const hasThought = Boolean(msg.thought && msg.thought.length > 0);
                const showThinkingRow = hasThought || (isCurrent && !msg.content);

                return (
                  <div key={msg.id} className="w-full flex flex-col gap-2 py-1">
                    {showThinkingRow && (
                      <ThinkingRow
                        thought={msg.thought || ""}
                        isGenerating={isCurrent && !msg.content}
                      />
                    )}

                    {msg.steps && msg.steps.length > 0 && (
                      <div className="flex flex-col gap-1 w-full py-1">
                        {msg.steps.map((step) => (
                          <ToolCallRow key={step.id} step={step} />
                        ))}
                      </div>
                    )}

                    {msg.diffPatch && <DiffView patch={msg.diffPatch} />}

                    {msg.content && (
                      <div className="text-[13px] leading-relaxed text-zinc-900 font-sans select-text">
                        <MarkdownRenderer content={msg.content} />
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Composer: strictly locked layout container */}
        <footer
          className={`shrink-0 p-4 flex flex-col items-center bg-white border-t border-zinc-100 ${
            messages.length === 0 ? "hidden" : ""
          }`}
        >
          <Composer
            onSend={handleSend}
            onCancel={handleCancel}
            isGenerating={isGenerating}
            selectedModel={selectedModel}
            onSelectModel={handleModelChange}
          />
        </footer>
          </>
        )}
      </main>

      {/* Session Search Command Bar (Cmd+K) */}
      <SessionCommandBar
        open={isCommandBarOpen}
        onOpenChange={setIsCommandBarOpen}
        sessions={sessionsRecord}
        onOpenSession={(id) => {
          handleSelectSession(id);
          setCurrentView("chat");
        }}
      />

      {/* Floating Push Toast Banners */}
      <PushToast toasts={toasts} onDismiss={handleDismissToast} />
    </div>
  );
}

export default App;
