export type ChatMessage = { role: "user" | "assistant" | "system"; content: string };

export type RouteRequest = { model: string; messages: ChatMessage[]; signal: AbortSignal };

/** What the adapter needs from a model route. The real route (#19) implements this. */
export interface ModelRoute {
  name: string;
  stream(req: RouteRequest): AsyncIterable<string>;
}

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
  };
}
