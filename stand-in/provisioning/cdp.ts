// Tiny headless-Chrome driver over the DevTools protocol (test helper, no dependencies).
// Uses the Chrome already on the machine (macOS runners ship it) and Node's global WebSocket.
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export function findChrome(): string | null {
  const candidates = [
    process.env.CHROME,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ];
  return candidates.find((p): p is string => !!p && existsSync(p)) ?? null;
}

export class Browser {
  private proc: ChildProcess;
  private dir: string;
  private ws!: WebSocket;
  private seq = 0;
  private pending = new Map<number, (v: CdpMessage) => void>();

  private constructor(proc: ChildProcess, dir: string) {
    this.proc = proc;
    this.dir = dir;
  }

  static async launch(chrome: string): Promise<Browser> {
    const dir = mkdtempSync(join(tmpdir(), "stand-in-chrome-"));
    const proc = spawn(
      chrome,
      ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${dir}`, "--no-first-run", "--disable-gpu", "about:blank"],
      { stdio: "ignore" },
    );
    const b = new Browser(proc, dir);
    const portFile = join(dir, "DevToolsActivePort");
    for (let i = 0; i < 100 && !existsSync(portFile); i++) await new Promise((r) => setTimeout(r, 100));
    if (!existsSync(portFile)) throw new Error("Chrome did not start");
    const port = readFileSync(portFile, "utf8").split("\n")[0];
    const targets = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()) as Target[];
    const page = targets.find((t) => t.type === "page");
    if (!page) throw new Error("no page target");
    b.ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise<void>((res, rej) => {
      b.ws.onopen = () => res();
      b.ws.onerror = () => rej(new Error("CDP connect failed"));
    });
    b.ws.onmessage = (m) => {
      const msg = JSON.parse(String(m.data)) as CdpMessage;
      const done = msg.id === undefined ? undefined : b.pending.get(msg.id);
      if (done) done(msg);
    };
    await b.send("Page.enable");
    return b;
  }

  private send(method: string, params: object = {}): Promise<EvalResult> {
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      this.pending.set(id, (msg) =>
        msg.error ? reject(new Error(msg.error.message)) : resolve((msg.result ?? {}) as EvalResult),
      );
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  /** Evaluate an expression in the page (awaits promises) and return its value. */
  async eval<T = unknown>(expression: string): Promise<T> {
    const r = await this.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.text + " " + JSON.stringify(r.exceptionDetails.exception?.description));
    return r.result?.value as T;
  }

  /** Navigate and wait until document.readyState is complete on the new URL. */
  async goto(url: string): Promise<void> {
    await this.send("Page.navigate", { url });
    await this.waitFor(`location.href === ${JSON.stringify(url)} && document.readyState === "complete"`);
  }

  async waitFor(expression: string, timeoutMs = 10_000): Promise<void> {
    const end = Date.now() + timeoutMs;
    while (Date.now() < end) {
      try {
        if (await this.eval<boolean>(expression)) return;
      } catch {
        // page mid-navigation; retry
      }
      await new Promise((r) => setTimeout(r, 50));
    }
    throw new Error(`timed out waiting for: ${expression}`);
  }

  async close(): Promise<void> {
    try {
      this.ws.close();
    } catch {}
    this.proc.kill("SIGKILL");
    await new Promise((r) => setTimeout(r, 100));
    rmSync(this.dir, { recursive: true, force: true, maxRetries: 3 });
  }
}

interface CdpMessage {
  id?: number;
  result?: unknown;
  error?: { message: string };
}

interface Target {
  type: string;
  webSocketDebuggerUrl: string;
}

interface EvalResult {
  result?: { value?: unknown };
  exceptionDetails?: { text: string; exception?: { description?: string } };
}
