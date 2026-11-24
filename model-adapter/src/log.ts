// Metadata-only logging. The event type has no field that can hold request or reply text,
// and every string in it is either fixed by us or one of the configured values.

export type LogEvent = {
  route: string;
  /** A configured level, or "unknown". Never the caller's raw string. */
  classification: string;
  /** A configured model, or "unknown". */
  model: string;
  outcome: "ok" | "refused_ceiling" | "bad_request" | "route_error" | "aborted";
  status: number;
  messages: number;
  requestBytes: number;
  replyBytes: number;
  ms: number;
  /** Error class name only, never a message. */
  errorClass?: string;
};

export type Logger = (event: LogEvent) => void;

export const consoleLogger: Logger = (e) => console.log(JSON.stringify(e));
