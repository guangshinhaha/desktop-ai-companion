import { withinCeiling } from "./classification.ts";
import type { AdapterConfig } from "./config.ts";
import type { ChatMessage, ModelRoute } from "./route.ts";

export class CeilingError extends Error {
  readonly ceiling: string;
  constructor(ceiling: string) {
    super("request classification is above the route's ceiling");
    this.name = "CeilingError";
    this.ceiling = ceiling;
  }
}

export class BadRequestError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "BadRequestError";
  }
}

export type ChatRequest = { classification: unknown; model?: unknown; messages: unknown };

export type ValidChat = { model: string; messages: ChatMessage[] };

export type ModelAdapter = {
  /** Check, then stream. Throws CeilingError or BadRequestError before any route call. */
  chat(req: ChatRequest, signal: AbortSignal): AsyncIterable<string>;
  validate(req: ChatRequest): ValidChat;
  readonly routeName: string;
};

/** The one adapter. The ceiling is checked here, before the route is touched. Requests are never altered to fit. */
export function createModelAdapter(config: AdapterConfig, route: ModelRoute): ModelAdapter {
  function validate(req: ChatRequest): ValidChat {
    // Ceiling first: an above-ceiling or unclassified request is refused whatever else is wrong with it.
    if (!withinCeiling(config, req.classification)) throw new CeilingError(config.ceiling);
    const model = req.model === undefined ? config.models[0]! : req.model;
    if (typeof model !== "string" || !config.models.includes(model)) throw new BadRequestError("model is not available on this route");
    const m = req.messages;
    if (!Array.isArray(m) || m.length === 0) throw new BadRequestError("messages must be a non-empty list");
    for (const x of m) {
      const ok = typeof x === "object" && x !== null && ["user", "assistant", "system"].includes((x as ChatMessage).role) && typeof (x as ChatMessage).content === "string";
      if (!ok) throw new BadRequestError("each message needs a role and string content");
    }
    // Rebuilt, so no other field the caller sent reaches the route.
    return { model, messages: (m as ChatMessage[]).map(({ role, content }) => ({ role, content })) };
  }
  return {
    routeName: route.name,
    validate,
    chat(req, signal) {
      const v = validate(req);
      return route.stream({ ...v, signal });
    },
  };
}
