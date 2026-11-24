import type { IncomingMessage, ServerResponse } from "node:http";
import { BadRequestError, CeilingError, type ModelAdapter } from "./adapter.ts";
import type { AdapterConfig } from "./config.ts";
import type { Logger, LogEvent } from "./log.ts";

const MAX_BODY = 1_000_000;

async function readBody(req: IncomingMessage): Promise<string> {
  let size = 0;
  const parts: Buffer[] = [];
  for await (const chunk of req) {
    size += (chunk as Buffer).length;
    if (size > MAX_BODY) throw new BadRequestError("body too large");
    parts.push(chunk as Buffer);
  }
  return Buffer.concat(parts).toString("utf8");
}

function send(res: ServerResponse, status: number, body: unknown) {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
}

function errClass(err: unknown): string {
  return err instanceof Error && /^[A-Za-z0-9_]{1,40}$/.test(err.name) ? err.name : "Error";
}

/**
 * Handler for POST /chat. Mount it behind the service's sign-in check (#29).
 * Reply is newline-delimited JSON: {"text":"..."} chunks, then {"done":true}.
 */
export function createChatHandler(config: AdapterConfig, adapter: ModelAdapter, log: Logger) {
  return async function handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const t0 = Date.now();
    const ev: LogEvent = { route: adapter.routeName, classification: "unknown", model: "unknown", outcome: "ok", status: 200, messages: 0, requestBytes: 0, replyBytes: 0, ms: 0 };
    const abort = new AbortController();
    res.on("close", () => { if (!res.writableEnded) abort.abort(); });
    try {
      if (req.method !== "POST") { ev.outcome = "bad_request"; ev.status = 405; return send(res, 405, { error: "method_not_allowed" }); }
      const text = await readBody(req);
      ev.requestBytes = Buffer.byteLength(text);
      let body: Record<string, unknown>;
      try { body = JSON.parse(text) as Record<string, unknown>; } catch { throw new BadRequestError("body is not valid JSON"); }
      if (typeof body !== "object" || body === null || Array.isArray(body)) throw new BadRequestError("body must be an object");
      if (typeof body.classification === "string" && config.levels.includes(body.classification)) ev.classification = body.classification;
      if (typeof body.model === "string" && config.models.includes(body.model)) ev.model = body.model;
      else if (body.model === undefined) ev.model = config.models[0]!;
      if (Array.isArray(body.messages)) ev.messages = body.messages.length;
      // Validate (and ceiling-check) before the first byte of a 200.
      adapter.validate(body);
      const stream = adapter.chat(body, abort.signal);
      res.writeHead(200, { "content-type": "application/x-ndjson", "cache-control": "no-store" });
      try {
        for await (const piece of stream) {
          ev.replyBytes += Buffer.byteLength(piece);
          res.write(JSON.stringify({ text: piece }) + "\n");
        }
        res.end(JSON.stringify({ done: true }) + "\n");
      } catch (err) {
        if (abort.signal.aborted) { ev.outcome = "aborted"; return; }
        ev.outcome = "route_error"; ev.status = 502; ev.errorClass = errClass(err);
        res.end(JSON.stringify({ error: "route_error" }) + "\n");
      }
    } catch (err) {
      if (err instanceof CeilingError) {
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
      log(ev);
    }
  };
}
