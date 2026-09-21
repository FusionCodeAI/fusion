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
                  });
                } catch {
                  // ignore unparseable
                }
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
            res.statusCode = 404;
            res.end(JSON.stringify({ error: "Session not found" }));
            return;
          }
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
