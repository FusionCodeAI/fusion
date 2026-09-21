import React, { useState, useEffect, useRef } from "react";
import { TopHeader } from "./components/TopHeader";
import { Sidebar, type SidebarSessionItem } from "./components/Sidebar";
import { ClineHeroView } from "./components/ClineHeroView";
import { ThinkingRow } from "./components/ThinkingRow";
import { UserMessage } from "./components/UserMessage";
import { ToolCallRow } from "./components/ToolCallRow";
import { DiffView } from "./components/DiffView";
import { Composer } from "./components/Composer";
import { ClineAvatar } from "./components/ClineAvatar";
import { MarkdownRenderer } from "./components/MarkdownRenderer";
import { AgentBridge } from "./lib/agent-bridge";
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
}

export function App({
  bridge: propBridge,
  initialMessages = [],
  initialSidebarOpen = true,
  initialModel = DEFAULT_FUSION_MODEL.id,
  initialSessions,
}: AppProps) {
  const [isSidebarOpen, setIsSidebarOpen] = useState(initialSidebarOpen);
  const [selectedModel, setSelectedModel] = useState<string>(initialModel);
  const [isGenerating, setIsGenerating] = useState(false);
  const [messages, setMessages] = useState<ChatMessage[]>(initialMessages);
  const [activeSessionId, setActiveSessionId] = useState<string>("session-default");
  const [sessionTitle, setSessionTitle] = useState<string>("General chat conversation");
  const [sessions, setSessions] = useState<SidebarSessionItem[]>(() => {
    if (initialSessions && initialSessions.length > 0) {
      return initialSessions;
    }
    return [
      {
        id: "session-default",
        title: "General chat conversation",
        createdAt: Date.now() - 2 * 3600 * 1000,
        updatedAt: Date.now() - 2 * 3600 * 1000,
      },
    ];
  });

  // Lazily initialize AgentBridge if not provided
  const bridgeRef = useRef<AgentBridge | null>(null);
  if (!bridgeRef.current) {
    bridgeRef.current = propBridge ?? new AgentBridge({ defaultModel: selectedModel });
  }
  const bridge = bridgeRef.current;
  const streamRef = useRef<HTMLDivElement>(null);
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
    if (messages.length === 0) {
      const newTitle = trimmed.length > 28 ? `${trimmed.slice(0, 28)}...` : trimmed;
      setSessionTitle(newTitle);
      setSessions((prev) =>
        prev.map((s) => (s.id === activeSessionId ? { ...s, title: newTitle, updatedAt: Date.now() } : s))
      );
    }

    try {
      await bridge.prompt(trimmed, selectedModel);
    } catch (err) {
      setIsGenerating(false);
      console.error("[App] prompt error:", err);
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
    const newSessionId = `session-${Date.now()}`;
    const newSession: SidebarSessionItem = {
      id: newSessionId,
      title: "General chat conversation",
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };
    setSessions((prev) => [newSession, ...prev]);
    setActiveSessionId(newSessionId);
    setSessionTitle("General chat conversation");
    setMessages([]);
    setIsGenerating(false);
  };

  const handleSelectSession = (id: string) => {
    setActiveSessionId(id);
    const found = sessions.find((s) => s.id === id);
    if (found) {
      setSessionTitle(found.title);
    }
  };

  const handleDeleteSession = (id: string) => {
    setSessions((prev) => {
      const updated = prev.filter((s) => s.id !== id);
      if (activeSessionId === id && updated.length > 0) {
        setActiveSessionId(updated[0].id);
        setSessionTitle(updated[0].title);
      }
      return updated;
    });
  };

  const handleModelChange = (modelId: string) => {
    setSelectedModel(modelId);
  };

  return (
    <div
      data-testid="app-shell"
      className="h-screen w-screen flex flex-row overflow-hidden bg-white"
    >
      {/* Sidebar: 220px, collapsed when isSidebarOpen is false */}
      {isSidebarOpen && (
        <Sidebar
          sessions={sessions}
          activeSessionId={activeSessionId}
          onNewChat={handleNewChat}
          onSelectSession={handleSelectSession}
          onDeleteSession={handleDeleteSession}
          onToggleSidebar={() => setIsSidebarOpen(false)}
        />
      )}

      {/* Conversation Area */}
      <main className="flex-1 min-h-0 flex flex-col overflow-hidden">
        {/* TopHeader: 40px height, aligned with macOS traffic lights and sidebar */}
        <TopHeader
          title={sessionTitle}
          onToggleSidebar={() => setIsSidebarOpen((prev) => !prev)}
          isSidebarOpen={isSidebarOpen}
        />

        {/* Stream: strictly centered, vertical scroll */}
        <div
          ref={streamRef}
          data-testid="conversation-stream"
          className="flex-1 min-h-0 overflow-y-auto px-8 py-4 flex flex-col items-center"
        >
          {messages.length === 0 ? (
            <ClineHeroView
              onSend={handleSend}
              selectedModel={selectedModel}
              onSelectModel={handleModelChange}
              workspaceName="workspace"
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
                  <div key={msg.id} className="w-full flex items-start gap-3 py-1">
                    <ClineAvatar
                      isThinking={isCurrent}
                      className="w-7 h-7 shrink-0 mt-0.5"
                    />
                    <div className="flex-1 min-w-0 flex flex-col gap-2">
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
                        <div className="text-[13px] leading-relaxed text-zinc-900 font-sans">
                          <MarkdownRenderer content={msg.content} />
                        </div>
                      )}
                    </div>
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
      </main>
    </div>
  );
}

export default App;
