import type { IncomingMessage, ServerResponse } from "node:http";
import { BadRequestError } from "./adapter.ts";
import { ClientGoneError, errClass, guard, readBody, send, TimeoutError } from "./chat.ts";
import { withinCeiling } from "./classification.ts";
import type { AdapterConfig } from "./config.ts";
import type { Logger, LogEvent } from "./log.ts";
import type { ModelRoute } from "./route.ts";

export type RecoverOptions = { bodyTimeoutMs?: number; routeTimeoutMs?: number };

/** Every field the executor's request builder may send (decision 0003). Anything else is refused. */
const REQUEST_KEYS = new Set(["kind", "classification", "app", "attempt", "max_attempts", "step", "divergence", "values", "tree", "truncated"]);

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

function checkRequest(b: Record<string, unknown>): void {
  const extra = Object.keys(b).find((k) => !REQUEST_KEYS.has(k));
  if (extra !== undefined) throw new BadRequestError("the request has a field a recovery request does not carry");
  if (b.kind !== "recover") throw new BadRequestError("kind must be 'recover'");
  if (typeof b.app !== "string" || !isObject(b.step) || !isObject(b.tree) || typeof b.divergence !== "string") {
    throw new BadRequestError("a recovery request needs app, step, divergence and tree");
  }
  if (b.values !== "masked" && b.values !== "included") throw new BadRequestError("values must be 'masked' or 'included'");
  if (!Number.isInteger(b.attempt) || !Number.isInteger(b.max_attempts)) throw new BadRequestError("attempt and max_attempts must be integers");
}

/** The three answers (TD 5.4), checked strictly. The executor checks them again against the live window. */
export function validRecoverAnswer(a: unknown): boolean {
  if (!isObject(a)) return false;
  const keys = Object.keys(a).sort().join(",");
  switch (a.answer) {
    case "retry":
      return keys === "answer" || (keys === "answer,reason" && typeof a.reason === "string");
    case "ask_officer":
      return keys === "answer,reason" && typeof a.reason === "string";
    case "act":
      if (typeof a.ref !== "string" || a.ref === "") return false;
      if (a.action === "press") return keys === "action,answer,ref";
      if (a.action === "set_value") {
        const v = a.value;
        return keys === "action,answer,ref,value" && isObject(v) && Object.keys(v).length === 1 && typeof v.input === "string";
      }
      return false;
    default:
      return false;
  }
}

class NoRecoverError extends Error {}

/**
 * Handler for POST /recover. Mount it behind the service's sign-in check (#29).
 * Ceiling first: above it, or unknown, is refused with 403 and the route is never called. Never downgraded.
 * Reply: the route's answer as JSON, only if it is plainly one of the three answers; otherwise 502.
 */
export function createRecoverHandler(config: AdapterConfig, route: ModelRoute, log: Logger, options: RecoverOptions = {}) {
  const bodyTimeoutMs = options.bodyTimeoutMs ?? 10_000;
  const routeTimeoutMs = options.routeTimeoutMs ?? 30_000;
  return async function handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const t0 = Date.now();
    const ev: LogEvent = { route: route.name, classification: "unknown", model: "unknown", outcome: "ok", status: 200, messages: 0, requestBytes: 0, replyBytes: 0, ms: 0 };
    const abort = new AbortController();
    res.on("close", () => { if (!res.writableEnded) abort.abort(); });
    try {
      if (req.method !== "POST") { ev.outcome = "bad_request"; ev.status = 405; return send(res, 405, { error: "method_not_allowed" }); }
      const text = await readBody(req, abort.signal, bodyTimeoutMs);
      ev.requestBytes = Buffer.byteLength(text);
      let body: unknown;
      try { body = JSON.parse(text); } catch { throw new BadRequestError("body is not valid JSON"); }
      const classification = isObject(body) ? body.classification : undefined;
      if (!withinCeiling(config, classification)) {
        ev.outcome = "refused_ceiling"; ev.status = 403;
        return send(res, 403, { error: "above_ceiling", reason: "This request is classified above what the model route is cleared for. It was not sent and was not changed.", ceiling: config.ceiling });
      }
      ev.classification = classification as string;
      if (!isObject(body)) throw new BadRequestError("body must be an object");
      checkRequest(body);
      if (!route.recover) throw new NoRecoverError();
      let answer: unknown;
      try {
        const p = route.recover({ request: body, signal: abort.signal });
        p.catch(() => {});
        answer = await guard(p, abort.signal, routeTimeoutMs);
      } catch (err) {
        if (err instanceof ClientGoneError) throw err;
        if (err instanceof TimeoutError) abort.abort(); // tell the route to stop
        ev.outcome = "route_error"; ev.status = 502; ev.errorClass = errClass(err);
        return send(res, 502, { error: "route_error" });
      }
      if (!validRecoverAnswer(answer)) {
        ev.outcome = "route_error"; ev.status = 502;
        return send(res, 502, { error: "route_error" });
      }
      const out = JSON.stringify(answer);
      ev.replyBytes = Buffer.byteLength(out);
      res.writeHead(200, { "content-type": "application/json", "cache-control": "no-store" });
      res.end(out);
    } catch (err) {
      if (err instanceof ClientGoneError) {
        ev.outcome = "aborted"; ev.status = 499;
      } else if (err instanceof TimeoutError) {
        ev.outcome = "bad_request"; ev.status = 408; ev.errorClass = "TimeoutError";
        if (!res.headersSent) send(res, 408, { error: "request_timeout" });
      } else if (err instanceof NoRecoverError) {
        ev.outcome = "route_error"; ev.status = 501;
        send(res, 501, { error: "not_implemented", reason: "This model route cannot answer recovery requests." });
      } else if (err instanceof BadRequestError) {
        ev.outcome = "bad_request"; ev.status = err.message === "body too large" ? 413 : 400;
        send(res, ev.status, { error: "bad_request", reason: err.message });
      } else {
        ev.outcome = "route_error"; ev.status = 500; ev.errorClass = errClass(err);
        if (!res.headersSent) send(res, 500, { error: "internal" }); else res.end();
      }
    } finally {
      ev.ms = Date.now() - t0;
      try { log(ev); } catch { /* a broken log sink must not fail the request */ }
    }
  };
}
