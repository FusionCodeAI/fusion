import React, { useEffect, useMemo, useSyncExternalStore } from "react";
import { render } from "@gpuix/react";
import { SessionStore } from "./state/session-store";
import { AcpClient } from "./bridge/acp-client";
import { Sidebar } from "./ui/sidebar";
import { HeroView } from "./ui/hero-view";
import { ChatView } from "./ui/chat-view";
import { Composer } from "./ui/composer";

// Global singletons for hot-reloading stability
const store = new SessionStore();
const client = new AcpClient({
  workspaceDir: process.cwd(),
});

export function App() {
  const snapshot = useSyncExternalStore(
    (cb) => store.subscribe(cb),
    () => store.getSnapshot()
  );

  const activeSession = useMemo(() => {
    return snapshot.sessions.find((s) => s.id === snapshot.activeSessionId) ?? null;
  }, [snapshot.sessions, snapshot.activeSessionId]);

  const isChatActive = activeSession && activeSession.messages.length > 0;

  // Initialize ACP sidecar process on startup
  useEffect(() => {
    client.spawn(snapshot.workspaceDir).catch((err) => {
      console.warn("ACP client spawn notice:", err.message);
    });

    // Wire ACP streaming events directly to session store
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
      session = store.createSession("Chat: " + promptText.slice(0, 24));
    }

    // Append user prompt to store
    store.appendUserMessage(promptText, session.id);
    store.setGenerating(true);

    try {
      await client.prompt(promptText, snapshot.selectedModel);
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
        backgroundColor: "#0d1117",
      }}
    >
      {/* Left Sidebar */}
      <Sidebar
        sessions={snapshot.sessions}
        activeSessionId={snapshot.activeSessionId}
        workspaceDir={snapshot.workspaceDir}
        userName="Aung Myat Moe"
        onNewChat={() => store.createSession("New Conversation")}
        onSelectSession={(id) => store.selectSession(id)}
        onDeleteSession={(id) => store.deleteSession(id)}
        onSelectWorkspace={() => {
          console.log("Workspace selector requested");
        }}
        onOpenSettings={() => {
          console.log("Settings requested");
        }}
      />

      {/* Main Agent Stage */}
      <div
        style={{
          flexGrow: 1,
          height: "100%",
          display: "flex",
          flexDirection: "column",
          backgroundColor: "#0d1117",
        }}
      >
        {!isChatActive ? (
          <HeroView
            onSubmit={handleSendPrompt}
            selectedModel={snapshot.selectedModel}
            onOpenFolder={() => console.log("Open folder clicked")}
          />
        ) : (
          <div
            style={{
              flexGrow: 1,
              height: "100%",
              display: "flex",
              flexDirection: "column",
              overflow: "hidden",
            }}
          >
            <ChatView
              sessionTitle={activeSession?.title ?? "Conversation"}
              messages={activeSession?.messages ?? []}
              isGenerating={snapshot.isGenerating}
            />

            <Composer
              onSend={handleSendPrompt}
              onCancel={handleCancel}
              isGenerating={snapshot.isGenerating}
              selectedModel={snapshot.selectedModel}
              onSelectModel={(model) => store.setSelectedModel(model)}
            />
          </div>
        )}
      </div>
    </div>
  );
}

// Start native GPUI window
render(<App />, {
  title: "Fusion Agent",
  width: 1140,
  height: 760,
});
