import type { IncomingMessage, ServerResponse } from "node:http";
import { BadRequestError, CeilingError, type ChatRequest, type ModelAdapter } from "./adapter.ts";
import type { AdapterConfig } from "./config.ts";
import type { Logger, LogEvent } from "./log.ts";

const MAX_BODY = 1_000_000;

export type ChatOptions = {
  /** Longest the client may take to send the body. */
  bodyTimeoutMs?: number;
  /** Longest the route may go without yielding a chunk. */
  idleTimeoutMs?: number;
};

class TimeoutError extends Error {
  constructor() {
    super("timed out");
    this.name = "TimeoutError";
  }
}

class ClientGoneError extends Error {
  constructor() {
    super("client gone");
    this.name = "AbortError";
  }
}

// Error names are fixed here. A route or request can set Error.name to anything, so never log it raw.
const KNOWN_ERRORS = new Set(["Error", "TypeError", "RangeError", "SyntaxError", "AbortError", "TimeoutError", "BadRequestError", "CeilingError"]);

function errClass(err: unknown): string {
  return err instanceof Error && KNOWN_ERRORS.has(err.name) ? err.name : "Error";
}

/** Resolve with `p`, or reject when `signal` aborts or `ms` passes. Always clears its timer and listener. */
function guard<T>(p: Promise<T>, signal: AbortSignal, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const onAbort = () => done(() => reject(new ClientGoneError()));
    const timer = setTimeout(() => done(() => reject(new TimeoutError())), ms);
    const done = (fn: () => void) => {
      clearTimeout(timer);
      signal.removeEventListener("abort", onAbort);
      fn();
    };
    if (signal.aborted) return onAbort();
    signal.addEventListener("abort", onAbort);
    p.then((v) => done(() => resolve(v)), (e) => done(() => reject(e)));
  });
}

async function readBody(req: IncomingMessage, signal: AbortSignal, ms: number): Promise<string> {
  const read = (async () => {
    let size = 0;
    const parts: Buffer[] = [];
    for await (const chunk of req) {
      size += (chunk as Buffer).length;
      if (size > MAX_BODY) throw new BadRequestError("body too large");
      parts.push(chunk as Buffer);
    }
    return Buffer.concat(parts).toString("utf8");
  })();
  read.catch(() => {}); // a guard that wins first must not leave this unhandled
  return guard(read, signal, ms);
}

function send(res: ServerResponse, status: number, body: unknown) {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
}

/**
 * Handler for POST /chat. Mount it behind the service's sign-in check (#29).
 * Reply is newline-delimited JSON: {"text":"..."} chunks, then {"done":true}.
 */
export function createChatHandler(config: AdapterConfig, adapter: ModelAdapter, log: Logger, options: ChatOptions = {}) {
  const bodyTimeoutMs = options.bodyTimeoutMs ?? 10_000;
  const idleTimeoutMs = options.idleTimeoutMs ?? 30_000;
  return async function handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const t0 = Date.now();
    const ev: LogEvent = { route: adapter.routeName, classification: "unknown", model: "unknown", outcome: "ok", status: 200, messages: 0, requestBytes: 0, replyBytes: 0, ms: 0 };
    const abort = new AbortController();
    // The client is gone if the socket closes before we finished the reply.
    res.on("close", () => { if (!res.writableEnded) abort.abort(); });
    let iterator: AsyncIterator<string> | undefined;
    try {
      if (req.method !== "POST") { ev.outcome = "bad_request"; ev.status = 405; return send(res, 405, { error: "method_not_allowed" }); }
      const text = await readBody(req, abort.signal, bodyTimeoutMs);
      ev.requestBytes = Buffer.byteLength(text);
      let body: Record<string, unknown>;
      try { body = JSON.parse(text) as Record<string, unknown>; } catch { throw new BadRequestError("body is not valid JSON"); }
      if (typeof body !== "object" || body === null || Array.isArray(body)) throw new BadRequestError("body must be an object");
      if (typeof body.classification === "string" && config.levels.includes(body.classification)) ev.classification = body.classification;
      if (typeof body.model === "string" && config.models.includes(body.model)) ev.model = body.model;
      else if (body.model === undefined) ev.model = config.models[0]!;
      if (Array.isArray(body.messages)) ev.messages = body.messages.length;
      // Validate (and ceiling-check) before the first byte of a 200.
      adapter.validate(body as ChatRequest);
      iterator = adapter.chat(body as ChatRequest, abort.signal)[Symbol.asyncIterator]();
      res.writeHead(200, { "content-type": "application/x-ndjson", "cache-control": "no-store" });
      try {
        for (;;) {
          const next = iterator.next();
          next.catch(() => {});
          const r = await guard(next, abort.signal, idleTimeoutMs);
          if (r.done) break;
          ev.replyBytes += Buffer.byteLength(r.value);
          res.write(JSON.stringify({ text: r.value }) + "\n");
        }
        res.end(JSON.stringify({ done: true }) + "\n");
      } catch (err) {
        if (err instanceof ClientGoneError) { ev.outcome = "aborted"; ev.status = 499; return; }
        ev.outcome = "route_error"; ev.status = 502; ev.errorClass = errClass(err);
        res.end(JSON.stringify({ error: "route_error" }) + "\n");
      } finally {
        // Let the route clean up on abort, timeout or error. Never wait on a route that ignores it.
        const it = iterator;
        iterator = undefined;
        if (it?.return) Promise.resolve(it.return()).catch(() => {});
      }
    } catch (err) {
      if (err instanceof ClientGoneError) {
        ev.outcome = "aborted"; ev.status = 499;
      } else if (err instanceof TimeoutError) {
        ev.outcome = "bad_request"; ev.status = 408; ev.errorClass = "TimeoutError";
        if (!res.headersSent) send(res, 408, { error: "request_timeout" });
      } else if (err instanceof CeilingError) {
        ev.outcome = "refused_ceiling"; ev.status = 403;
        send(res, 403, { error: "above_ceiling", reason: "This request is classified above what the model route is cleared for. It was not sent and was not changed.", ceiling: err.ceiling });
      } else if (err instanceof BadRequestError) {
        ev.outcome = "bad_request"; ev.status = err.message === "body too large" ? 413 : 400;
        send(res, ev.status, { error: "bad_request", reason: err.message });
      } else {
        ev.outcome = "route_error"; ev.status = 500; ev.errorClass = errClass(err);
        if (!res.headersSent) send(res, 500, { error: "internal" }); else res.end();
      }
    } finally {
      ev.ms = Date.now() - t0;
      try { log(ev); } catch { /* a broken log sink must not fail the request or leak its message */ }
    }
  };
}
