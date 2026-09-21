import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

function fusionLocalSessionsPlugin() {
  return {
    name: "fusion-local-sessions",
    configureServer(server: any) {
      server.middlewares.use((req: any, res: any, next: any) => {
        if (req.url === "/api/sessions") {
          const sessionsDir = path.join(os.homedir(), ".fusion", "sessions");
          if (!fs.existsSync(sessionsDir)) {
            res.setHeader("Content-Type", "application/json");
            res.end(JSON.stringify([]));
            return;
          }

          try {
            const entries = fs.readdirSync(sessionsDir, { withFileTypes: true });
            const summaries: any[] = [];

            for (const entry of entries) {
              if (entry.isFile() && entry.name.endsWith(".json")) {
                try {
                  const fullPath = path.join(sessionsDir, entry.name);
                  const raw = fs.readFileSync(fullPath, "utf-8");
                  const parsed = JSON.parse(raw);
                  const ws = parsed.workspace || parsed.cwd || "";
                  const wsName = ws ? path.basename(ws) : "General Chat";
                  summaries.push({
                    id: parsed.id || entry.name.replace(".json", ""),
                    title: parsed.title || "General chat conversation",
                    created_at: parsed.created_at || "",
                    updated_at: parsed.updated_at || "",
                    model: parsed.active_model || "deepseek-v4-flash-0731",
                    message_count: Array.isArray(parsed.messages) ? parsed.messages.length : 0,
                    preview:
                      Array.isArray(parsed.messages) && parsed.messages.length > 0
                        ? (parsed.messages[parsed.messages.length - 1].content || "").slice(0, 100)
                        : "",
                    workspace: ws,
                    workspace_name: wsName,
                  });
                } catch {
                  // ignore unparseable
                }
              } else if (entry.isDirectory() && entry.name.startsWith("%2F")) {
                try {
                  const decodedWs = decodeURIComponent(entry.name);
                  const wsName = path.basename(decodedWs) || "Workspace";
                  const subDirPath = path.join(sessionsDir, entry.name);
                  const subEntries = fs.readdirSync(subDirPath, { withFileTypes: true });

                  for (const sub of subEntries) {
                    if (sub.isDirectory()) {
                      const summaryPath = path.join(subDirPath, sub.name, "summary.json");
                      if (fs.existsSync(summaryPath)) {
                        try {
                          const sum = JSON.parse(fs.readFileSync(summaryPath, "utf-8"));
                          summaries.push({
                            id: sum.info?.id || sub.name,
                            title: sum.session_summary || "Workspace session",
                            created_at: sum.created_at || "",
                            updated_at: sum.updated_at || "",
                            model: sum.current_model_id || "deepseek-v4-flash-0731",
                            message_count: sum.num_chat_messages || sum.num_messages || 0,
                            preview: `Workspace session in ${wsName}`,
                            workspace: decodedWs,
                            workspace_name: wsName,
                          });
                        } catch {}
                      }
                    }
                  }
                } catch {}
              }
            }

            // Sort by updated_at descending
            summaries.sort((a, b) => {
              const tA = a.updated_at ? new Date(a.updated_at).getTime() : 0;
              const tB = b.updated_at ? new Date(b.updated_at).getTime() : 0;
              return tB - tA;
            });

            res.setHeader("Content-Type", "application/json");
            res.end(JSON.stringify(summaries));
            return;
          } catch (err: any) {
            res.statusCode = 500;
            res.end(JSON.stringify({ error: String(err) }));
            return;
          }
        }

        if (req.url && req.url.startsWith("/api/session/")) {
          const id = req.url.replace(/^\/api\/session\//, "");
          const sessionPath = path.join(os.homedir(), ".fusion", "sessions", `${id}.json`);
          if (fs.existsSync(sessionPath)) {
            res.setHeader("Content-Type", "application/json");
            res.end(fs.readFileSync(sessionPath, "utf-8"));
            return;
          } else {
            // Check subdirectories
            const sessionsDir = path.join(os.homedir(), ".fusion", "sessions");
            const subEntries = fs.readdirSync(sessionsDir, { withFileTypes: true });
            for (const sub of subEntries) {
              if (sub.isDirectory()) {
                const subSessionPath = path.join(sessionsDir, sub.name, id, "summary.json");
                if (fs.existsSync(subSessionPath)) {
                  res.setHeader("Content-Type", "application/json");
                  res.end(fs.readFileSync(subSessionPath, "utf-8"));
                  return;
                }
              }
            }
            res.statusCode = 404;
            res.end(JSON.stringify({ error: "Session not found" }));
            return;
          }
        }
        if (req.url && req.url.startsWith("/api/sound")) {
          const soundPath = "/System/Library/Sounds/Ping.aiff";
          if (fs.existsSync(soundPath)) {
            import("node:child_process").then(({ spawn }) => {
              spawn("afplay", [soundPath]);
            });
          }
          res.setHeader("Content-Type", "application/json");
          res.end(JSON.stringify({ ok: true }));
          return;
        }

        next();
      });
    },
  };
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss(), fusionLocalSessionsPlugin()],
  server: {
    port: 5173,
  },
});
