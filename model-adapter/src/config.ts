// Adapter config: the route, its classification ceiling and its models (decision 0003).
// The real route is unknown until #19; only the stub kind exists.

export const DEFAULT_LEVELS = ["official-open", "official-closed", "restricted", "confidential", "secret"] as const;

export type RouteConfig = { kind: "stub" };

export type AdapterConfig = {
  route: RouteConfig;
  /** Classification levels, lowest first. */
  levels: readonly string[];
  /** Highest level the route is cleared for. */
  ceiling: string;
  /** Models available on the route. The first is the default. */
  models: readonly string[];
};

export function loadConfig(raw: unknown): AdapterConfig {
  if (typeof raw !== "object" || raw === null) throw new Error("adapter config must be an object");
  const c = raw as Record<string, unknown>;
  const route = c.route as { kind?: unknown } | undefined;
  if (!route || route.kind !== "stub") throw new Error("adapter config: route.kind must be 'stub' until #19 names a real route");
  const levels = c.levels === undefined ? [...DEFAULT_LEVELS] : c.levels;
  if (!Array.isArray(levels) || levels.length === 0 || !levels.every((l) => typeof l === "string" && l.length > 0) || new Set(levels).size !== levels.length) {
    throw new Error("adapter config: levels must be a non-empty list of distinct strings");
  }
  if (typeof c.ceiling !== "string" || !levels.includes(c.ceiling)) throw new Error("adapter config: ceiling must be one of levels");
  const models = c.models;
  if (!Array.isArray(models) || models.length === 0 || !models.every((m) => typeof m === "string" && m.length > 0)) {
    throw new Error("adapter config: models must be a non-empty list of strings");
  }
  return { route: { kind: "stub" }, levels, ceiling: c.ceiling, models };
}

/** Read config from COMPANION_MODEL_CONFIG (JSON). No default ceiling: a missing config refuses to start. */
export function configFromEnv(env: Record<string, string | undefined>): AdapterConfig {
  const text = env.COMPANION_MODEL_CONFIG;
  if (!text) throw new Error("COMPANION_MODEL_CONFIG is not set");
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error("COMPANION_MODEL_CONFIG is not valid JSON");
  }
  return loadConfig(parsed);
}
