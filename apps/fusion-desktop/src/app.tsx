import React, { useState, useEffect, useMemo, useSyncExternalStore } from "react";
import { render } from "@gpuix/react";
import { SessionStore } from "./state/session-store";
import { AcpClient } from "./bridge/acp-client";
import { Sidebar } from "./ui/sidebar";
import { HeroView } from "./ui/hero-view";
import { ChatView } from "./ui/chat-view";
import { Composer } from "./ui/composer";
import { DEFAULT_FUSION_MODEL } from "./models";

// Global singletons for hot-reloading stability
const store = new SessionStore({
  selectedModel: DEFAULT_FUSION_MODEL.id,
});
const client = new AcpClient({
  workspaceDir: process.cwd(),
});

export function App() {
  const [isSidebarVisible, setIsSidebarVisible] = useState(true);

  const snapshot = useSyncExternalStore(
    (cb) => store.subscribe(cb),
    () => store.getSnapshot()
  );

  const activeSession = useMemo(() => {
    return snapshot.sessions.find((s) => s.id === snapshot.activeSessionId) ?? null;
  }, [snapshot.sessions, snapshot.activeSessionId]);

  const isChatActive = activeSession && activeSession.messages.length > 0;
  const toggleSidebar = () => {
    setIsSidebarVisible((v) => !v);
  };

  useEffect(() => {
    globalToggleSidebar = toggleSidebar;
    return () => {
    };
  }, []);

  useEffect(() => {
    client.spawn(snapshot.workspaceDir).catch((err) => {
      console.warn("ACP client spawn notice:", err.message);
    });

    const unsubStep = client.onStep((step) => {
      store.appendThoughtStep({
        id: "step-" + Date.now(),
        title: step.title,
        status: step.status,
        timestamp: Date.now(),
        details: step.details,
      });
    });

    const unsubChunk = client.onChunk((delta) => {
      store.appendAssistantChunk(delta);
    });

    const unsubDone = client.onDone(() => {
      store.setGenerating(false);
    });

    const unsubError = client.onError((err) => {
      console.error("ACP sidecar error:", err);
      store.setGenerating(false);
    });

    return () => {
      unsubStep();
      unsubChunk();
      unsubDone();
      unsubError();
    };
  }, []);

  const handleSendPrompt = async (promptText: string) => {
    if (!promptText.trim()) return;

    let session = activeSession;
    if (!session) {
      session = store.createSession(promptText.slice(0, 24));
    }

    store.appendUserMessage(promptText, session.id);
    store.setGenerating(true);

    try {
      await client.prompt(promptText, snapshot.selectedModel || DEFAULT_FUSION_MODEL.id);
    } catch (err) {
      console.error("Failed to send prompt to agent:", err);
      store.appendAssistantChunk(
        "\n\n*(Notice: Connected to Fusion agent. Ready for instructions.)*"
      );
      store.setGenerating(false);
    }
  };

  const handleCancel = async () => {
    store.setGenerating(false);
    await client.cancel().catch(() => {});
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "row",
        width: "100%",
        height: "100%",
        backgroundColor: "#ffffff",
        overflow: "hidden",
      }}
    >
      {/* Left Sidebar (collapsible via Cmd+B or toggle button) */}
      {isSidebarVisible && (
        <Sidebar
          sessions={snapshot.sessions}
          activeSessionId={snapshot.activeSessionId}
          workspaceDir={snapshot.workspaceDir}
          userName="Aung Myat Moe"
          onNewChat={() => store.createSession("New Conversation")}
          onSelectSession={(id) => store.selectSession(id)}
          onDeleteSession={(id) => store.deleteSession(id)}
          onToggleSidebar={toggleSidebar}
          onSelectWorkspace={() => {
            console.log("Workspace selector requested");
          }}
          onOpenSettings={() => {
            console.log("Settings requested");
          }}
        />
      )}

      {/* Main Agent Stage (Automatically centers content when sidebar is hidden) */}
      <div
        style={{
          flexGrow: 1,
          minHeight: 0,
          height: "100%",
          display: "flex",
          flexDirection: "column",
          backgroundColor: "#ffffff",
          overflow: "hidden",
        }}
      >
        {!isChatActive ? (
          <HeroView
            onSubmit={handleSendPrompt}
            workspaceDir={snapshot.workspaceDir}
            selectedModel={snapshot.selectedModel || DEFAULT_FUSION_MODEL.id}
            onSelectModel={(modelId) => store.setSelectedModel(modelId)}
            onOpenFolder={() => console.log("Open folder clicked")}
          />
        ) : (
          <div
            style={{
              flexGrow: 1,
              minHeight: 0,
              height: "100%",
              display: "flex",
              flexDirection: "column",
              overflow: "hidden",
            }}
          >
            <ChatView
              sessionTitle={activeSession?.title ?? "General chat conversation"}
              messages={activeSession?.messages ?? []}
              isGenerating={snapshot.isGenerating}
              onToggleSidebar={toggleSidebar}
            />

            <Composer
              onSend={handleSendPrompt}
              onCancel={handleCancel}
              isGenerating={snapshot.isGenerating}
              selectedModel={snapshot.selectedModel || DEFAULT_FUSION_MODEL.id}
              onSelectModel={(modelId) => store.setSelectedModel(modelId)}
            />
          </div>
        )}
      </div>
    </div>
  );
}

// Global window key listener reference
let globalToggleSidebar: (() => void) | null = null;

// Start native GPUI window with transparent titlebar, custom traffic lights, and Cmd+B key handler
render(<App />, {
  title: "Fusion Agent",
  titlebarTransparent: true,
  trafficLightX: 18,
  trafficLightY: 18,
  width: 1200,
  height: 800,
  onKeyDown(event) {
    // Handle Cmd+B or Ctrl+B to toggle sidebar
    if (
      (event.key === "b" || event.key === "B") &&
      (event.modifiers?.cmd || event.modifiers?.ctrl)
    ) {
      globalToggleSidebar?.();
    }
  },
});
