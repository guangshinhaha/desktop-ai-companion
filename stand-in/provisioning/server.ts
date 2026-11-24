// Stand-in account-provisioning app. Node built-ins only.
// Run: npm start (from stand-in/). PORT=3000 by default.
import { createServer, type IncomingMessage, type Server } from "node:http";
import { pathToFileURL } from "node:url";
import { createdPage, errorPage, formPage, parseSwitches, ROLES, SCHOOLS } from "./page.ts";

export interface Account {
  id: number;
  full_name: string;
  email: string;
  school: string;
  role: string;
  created_at: string;
}

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let size = 0;
    req.on("data", (c: Buffer) => {
      size += c.length;
      if (size > 64 * 1024) {
        reject(new Error("body too large"));
        req.destroy();
        return;
      }
      chunks.push(c);
    });
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

export function createApp(): Server {
  // In-memory only. The password field, when shown, is never stored or logged.
  let accounts: Account[] = [];
  let nextId = 1;

  return createServer(async (req, res) => {
    const url = new URL(req.url ?? "/", "http://localhost");
    const sw = parseSwitches(url.searchParams);
    const send = (status: number, type: string, body: string) => {
      res.writeHead(status, { "content-type": `${type}; charset=utf-8`, "cache-control": "no-store" });
      res.end(body);
    };
    const json = (status: number, v: unknown) => send(status, "application/json", JSON.stringify(v));

    try {
      if (req.method === "GET" && url.pathname === "/") return send(200, "text/html", formPage(sw));

      if (req.method === "POST" && url.pathname === "/accounts") {
        const f = new URLSearchParams(await readBody(req));
        const a = {
          full_name: (f.get("full_name") ?? "").trim(),
          email: (f.get("email") ?? "").trim(),
          school: f.get("school") ?? "",
          role: f.get("role") ?? "",
        };
        const problem =
          !a.full_name || !a.email ? "Full name and email are required."
          : !SCHOOLS.includes(a.school) ? "Unknown school."
          : !ROLES.includes(a.role) ? "Unknown role."
          : sw.password && !f.get("password") ? "Password is required."
          : "";
        if (problem) return send(400, "text/html", errorPage(problem));
        if (sw.latency) await new Promise((r) => setTimeout(r, sw.latency));
        accounts.push({ id: nextId++, ...a, created_at: new Date().toISOString() });
        return send(200, "text/html", createdPage(sw, a));
      }

      // Harness API: stable contract, documented in stand-in/README.md.
      if (req.method === "GET" && url.pathname === "/api/accounts") return json(200, { accounts });
      if (req.method === "POST" && url.pathname === "/api/reset") {
        accounts = [];
        nextId = 1;
        return json(200, { ok: true, accounts: 0 });
      }
      if (url.pathname === "/healthz") return json(200, { ok: true });

      return send(404, "text/plain", "Not found");
    } catch (e) {
      return send(500, "text/plain", e instanceof Error ? e.message : "error");
    }
  });
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  const port = Number(process.env.PORT ?? 3000);
  createApp().listen(port, "127.0.0.1", () => {
    console.log(`stand-in provisioning listening on http://127.0.0.1:${port}/`);
  });
}
