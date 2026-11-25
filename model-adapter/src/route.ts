export type ChatMessage = { role: "user" | "assistant" | "system"; content: string };

export type RouteRequest = { model: string; messages: ChatMessage[]; signal: AbortSignal };

/** What the adapter needs from a model route. The real route (#19) implements this. */
export interface ModelRoute {
  name: string;
  stream(req: RouteRequest): AsyncIterable<string>;
  /** Answer a recovery request (TD 5.4). Optional: a route without it cannot recover. */
  recover?(req: RecoverRouteRequest): Promise<unknown>;
}

/** A checked recovery request (see recover.ts) and the signal that cancels it. */
export type RecoverRouteRequest = { request: Record<string, unknown>; signal: AbortSignal };

/** Stand-in route. Sends nothing anywhere; replies with a fixed synthetic line. */
export function stubRoute(): ModelRoute {
  return {
    name: "stub",
    async *stream(req) {
      for (const word of ["Stub", "reply", "from", req.model]) {
        if (req.signal.aborted) return;
        yield word + " ";
      }
    },
    async recover() {
      return { answer: "ask_officer", reason: "No model is connected (stub route)." };
    },
  };
}
