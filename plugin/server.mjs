import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ListResourcesRequestSchema,
  ListToolsRequestSchema,
  ReadResourceRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const APP_URI = "ui://sync/app.html";
const APP_VERSION = "2.1.4";

function resolveSyncExe() {
  const localAppData = process.env.LOCALAPPDATA || path.join(process.env.USERPROFILE || "", "AppData", "Local");
  return path.join(localAppData, "Programs", "AntigravitySync", "AntigravitySync.exe");
}

const HTML = `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>Antigravity Sync v${APP_VERSION}</title>
<style>
  :root {
    --bg: #0f172a;
    --card: #18233a;
    --border: #33415c;
    --text: #e2e8f0;
    --muted: #94a3b8;
    --accent: #007acc;
    --accent-hover: #1c97ea;
    --green: #34d399;
    --surface: #1e2942;
  }
  * { box-sizing: border-box; font-family: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif; }
  body { margin: 0; padding: 14px; background: var(--bg); color: var(--text); }
  .card {
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 14px;
  }
  .hdr { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
  h2 { margin: 0; font-size: 15px; font-weight: 650; }
  .ver { font-size: 11px; color: var(--muted); }
  p { margin: 0 0 12px 0; font-size: 12.5px; color: var(--muted); line-height: 1.45; }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; }
  button {
    flex: 1;
    min-width: 100px;
    height: 32px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    color: var(--text);
    background: var(--surface);
    transition: background 0.12s ease;
  }
  button:hover { background: #263552; }
  .btn-push { background: var(--accent); border-color: var(--accent); color: #fff; }
  .btn-push:hover { background: var(--accent-hover); }
  .btn-pull { border-color: var(--green); color: var(--green); }
  button:disabled { opacity: 0.6; cursor: not-allowed; }
  .status { margin-top: 10px; font-size: 12px; color: var(--muted); min-height: 16px; }
</style>
</head>
<body>
  <div class="card">
    <div class="hdr">
      <h2>Antigravity Sync</h2>
      <span class="ver">v${APP_VERSION}</span>
    </div>
    <p>Runs non-destructive smart merge via <code>AntigravitySync.exe</code> (closes Antigravity, merges chats &amp; settings, and relaunches).</p>
    <div class="actions">
      <button class="btn-push" id="pushBtn">↑ Push</button>
      <button class="btn-pull" id="pullBtn">↓ Pull</button>
      <button id="openBtn">Open App</button>
    </div>
    <div class="status" id="status">Ready.</div>
  </div>
<script>
  let nextId = 1;
  const pending = new Map();

  function rpc(method, params) {
    const id = nextId++;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      window.parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*");
    });
  }

  window.addEventListener("message", (event) => {
    const msg = event.data;
    if (!msg || typeof msg !== "object") return;
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(msg.error);
      else resolve(msg.result);
    }
  });

  rpc("ui/initialize", {
    appInfo: { name: "Antigravity Sync", version: APP_VERSION },
    appCapabilities: {},
    protocolVersion: "2026-01-26"
  }).then(() => {
    window.parent.postMessage({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} }, "*");
  }).catch(() => {});

  async function trigger(mode) {
    const status = document.getElementById("status");
    const pushBtn = document.getElementById("pushBtn");
    const pullBtn = document.getElementById("pullBtn");
    pushBtn.disabled = true;
    pullBtn.disabled = true;
    status.textContent = mode === "Open"
      ? "Opening Antigravity Sync..."
      : "Launching " + mode + " sync... Antigravity will close and relaunch.";
    try {
      await rpc("tools/call", { name: "run_sync", arguments: { mode } });
    } catch (err) {
      status.textContent = "Error: " + (err?.message || String(err));
    } finally {
      pushBtn.disabled = false;
      pullBtn.disabled = false;
    }
  }

  document.getElementById("pushBtn").addEventListener("click", () => trigger("Push"));
  document.getElementById("pullBtn").addEventListener("click", () => trigger("Pull"));
  document.getElementById("openBtn").addEventListener("click", () => trigger("Open"));
</script>
</body>
</html>`;

const server = new Server(
  { name: "antigravity-sync", version: APP_VERSION },
  { capabilities: { tools: {}, resources: {} } }
);

server.setRequestHandler(ListResourcesRequestSchema, async () => ({
  resources: [
    {
      uri: APP_URI,
      name: "Antigravity Sync UI",
      mimeType: "text/html;profile=mcp-app",
    },
  ],
}));

server.setRequestHandler(ReadResourceRequestSchema, async (req) => {
  if (req.params.uri === APP_URI) {
    return {
      contents: [
        {
          uri: APP_URI,
          mimeType: "text/html;profile=mcp-app",
          text: HTML,
        },
      ],
    };
  }
  throw new Error(`Unknown resource: ${req.params.uri}`);
});

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: "open_sync",
      description: "Open the Antigravity Google Drive Sync panel",
      inputSchema: { type: "object", properties: {} },
      _meta: {
        ui: { resourceUri: APP_URI },
        "com.google.antigravity/ui": {
          entrypoints: [
            { type: "aux_pane", title: "Sync" },
            { type: "global", title: "Sync" },
          ],
        },
      },
    },
    {
      name: "run_sync",
      description: "Run Push, Pull, or open AntigravitySync.exe",
      inputSchema: {
        type: "object",
        properties: {
          mode: { type: "string", enum: ["Push", "Pull", "Open"] },
        },
        required: ["mode"],
      },
      _meta: {
        ui: { visibility: ["app"] },
      },
    },
  ],
}));

server.setRequestHandler(CallToolRequestSchema, async (req) => {
  const { name, arguments: args } = req.params;
  if (name === "open_sync") {
    return {
      content: [{ type: "text", text: "Opened Antigravity Sync panel." }],
      structuredContent: { ready: true },
    };
  }
  if (name === "run_sync") {
    const mode = args?.mode === "Pull" ? "Pull" : args?.mode === "Open" ? "Open" : "Push";
    const exe = resolveSyncExe();
    if (!fs.existsSync(exe)) {
      throw new Error(`AntigravitySync.exe not found at ${exe}. Please install or update Antigravity Sync.`);
    }
    const cliArgs = mode === "Open" ? [] : [mode === "Pull" ? "--pull" : "--push", "--relaunch"];
    const child = spawn(exe, cliArgs, { detached: true, stdio: "ignore", windowsHide: false });
    child.unref();
    return {
      content: [{ type: "text", text: `Launched AntigravitySync.exe (${mode}).` }],
      structuredContent: { started: true, mode },
    };
  }
  throw new Error(`Unknown tool: ${name}`);
});

const transport = new StdioServerTransport();
await server.connect(transport);
